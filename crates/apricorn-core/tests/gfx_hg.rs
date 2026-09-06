//! Integration tests: every BG graphics member in a retail HeartGold (US)
//! dump.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM). The expected counts are ground truth established by
//! scanning the retail image while writing the parsers — see
//! `docs/nitro-gfx.md`.

use std::collections::HashMap;

use apricorn_core::formats::{
    CharMapping, Narc, Ncgr, Nclr, Nscr, PixelFmt, is_narc, is_ncgr, is_nclr, is_nscr,
};
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail HeartGold (US) graphics census.
const NCGR_MEMBERS: usize = 7_937;
const NCGR_DIMS_VALID: usize = 4_928; // H/W declared (not 0xFFFF)
const NCGR_DIMS_NONE: usize = 3_009; // linear OBJ data
const NCGR_WITH_CPOS: usize = 860;
const NCGR_4BPP: usize = 7_888;
const NCGR_8BPP: usize = 49;
const NCGR_MAP_2D: usize = 4_928;
const NCGR_MAP_1D_32K: usize = 2_539;
const NCGR_MAP_1D_64K: usize = 197;
const NCGR_MAP_1D_128K: usize = 239;
const NCGR_MAP_1D_256K: usize = 34;

const NCLR_MEMBERS: usize = 4_945;
const NCLR_FMT_16C: usize = 2_448; // fmt == 3
const NCLR_FMT_256C: usize = 345; // fmt == 4
const NCLR_FMT_FLAGGED: usize = 2_152; // fmt == 0x000A0004 (see Nclr::fmt_raw)
const NCLR_EXTENDED: usize = 115;
const NCLR_WITH_PMCP: usize = 748;
const NCLR_COMPRESSED: usize = 193; // logical_size > stored bytes

const NSCR_MEMBERS: usize = 791;
const NSCR_COLOR_16: usize = 759; // colorMode == 0
const NSCR_COLOR_256: usize = 15; // colorMode == 1
const NSCR_COLOR_OTHER: usize = 17; // colorMode == 2 (unpinned)
const NSCR_FORMAT_TEXT: usize = 772; // screenFormat == 0
const NSCR_FORMAT_OTHER: usize = 19; // screenFormat 1/2 (rotation variants)

fn load_rom() -> Option<Vec<u8>> {
    match std::fs::read(ROM_PATH) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

/// Parses every NARC in the ROM. Any parse failure fails the test: all
/// 308 retail archives must parse. (The `Narc`s borrow `data`, like the
/// `NdsRom` does.)
fn all_narcs<'a>(rom: &NdsRom<'a>) -> Vec<(String, Narc<'a>)> {
    let mut narcs = Vec::new();
    for file in rom.nitrofs().files() {
        let bytes = rom.file(file.fat_id).expect("fat id in range");
        if is_narc(bytes) {
            let narc = Narc::parse(bytes)
                .unwrap_or_else(|e| panic!("NARC {} failed to parse: {e}", file.path));
            narcs.push((file.path.clone(), narc));
        }
    }
    narcs
}

#[test]
fn parses_every_bg_member_and_matches_census() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let narcs = all_narcs(&rom);

    let mut ncgr = 0usize;
    let mut ncgr_dims_valid = 0usize;
    let mut ncgr_dims_none = 0usize;
    let mut ncgr_cpos = 0usize;
    let mut ncgr_bpp_4 = 0usize;
    let mut ncgr_bpp_8 = 0usize;
    let mut mapping: HashMap<CharMapping, usize> = HashMap::new();
    let mut char_fmt: HashMap<u32, usize> = HashMap::new();

    let mut nclr = 0usize;
    let mut nclr_extended = 0usize;
    let mut nclr_pmcp = 0usize;
    let mut nclr_compressed = 0usize;
    let mut nclr_fmt: HashMap<u32, usize> = HashMap::new();

    let mut nscr = 0usize;
    let mut nscr_color = [0usize; 3];
    let mut nscr_format = [0usize; 3];

    for (path, narc) in &narcs {
        for id in 0..narc.file_count() {
            let bytes = narc.file(id).expect("member id in range");
            let where_ = |what: &str| format!("{path} member {id}: {what}");
            if is_ncgr(bytes) {
                let file = Ncgr::parse(bytes)
                    .unwrap_or_else(|e| panic!("{}: {e}", where_("NCGR must parse")));
                ncgr += 1;
                match (file.height(), file.width()) {
                    (Some(h), Some(w)) => {
                        ncgr_dims_valid += 1;
                        // A sheet that declares a grid holds exactly
                        // H×W tiles.
                        assert_eq!(
                            file.tile_data().len(),
                            usize::from(h) * usize::from(w) * file.pixel_fmt().tile_size(),
                            "{}",
                            where_("tile data size must equal H×W×tile")
                        );
                    }
                    (None, None) => ncgr_dims_none += 1,
                    _ => panic!("{}", where_("H and W must agree (both set or both 0xFFFF)")),
                }
                if let Some((cw, ch)) = file.cpos() {
                    ncgr_cpos += 1;
                    // CPOS always restates the CHAR grid dims (and only
                    // grid files carry it).
                    assert_eq!(
                        (cw, ch),
                        (
                            file.width().expect("CPOS files have a grid"),
                            file.height().expect("CPOS files have a grid")
                        ),
                        "{}",
                        where_("CPOS size must mirror the CHAR dims")
                    );
                }
                match file.bpp() {
                    4 => ncgr_bpp_4 += 1,
                    8 => ncgr_bpp_8 += 1,
                    _ => unreachable!("bpp is 4 or 8"),
                }
                *mapping.entry(file.mapping()).or_default() += 1;
                *char_fmt.entry(file.character_fmt()).or_default() += 1;
            } else if is_nclr(bytes) {
                let file = Nclr::parse(bytes)
                    .unwrap_or_else(|e| panic!("{}: {e}", where_("NCLR must parse")));
                nclr += 1;
                *nclr_fmt.entry(file.fmt_raw()).or_default() += 1;
                if file.is_extended() {
                    nclr_extended += 1;
                }
                if file.pmcp().is_some() {
                    nclr_pmcp += 1;
                }
                if file.is_compressed() {
                    nclr_compressed += 1;
                }
            } else if is_nscr(bytes) {
                let file = Nscr::parse(bytes)
                    .unwrap_or_else(|e| panic!("{}: {e}", where_("NSCR must parse")));
                nscr += 1;
                let color = usize::from(file.color_mode());
                assert!(color < 3, "{}", where_("unexpected colorMode"));
                nscr_color[color] += 1;
                let format = usize::from(file.screen_format());
                assert!(format < 3, "{}", where_("unexpected screenFormat"));
                nscr_format[format] += 1;
            }
        }
    }

    assert_eq!(ncgr, NCGR_MEMBERS, "NCGR member count");
    assert_eq!(ncgr_dims_valid, NCGR_DIMS_VALID);
    assert_eq!(ncgr_dims_none, NCGR_DIMS_NONE);
    assert_eq!(ncgr_cpos, NCGR_WITH_CPOS);
    assert_eq!(ncgr_bpp_4, NCGR_4BPP);
    assert_eq!(ncgr_bpp_8, NCGR_8BPP);
    assert_eq!(mapping.get(&CharMapping::TwoD), Some(&NCGR_MAP_2D));
    assert_eq!(mapping.get(&CharMapping::OneD32K), Some(&NCGR_MAP_1D_32K));
    assert_eq!(mapping.get(&CharMapping::OneD64K), Some(&NCGR_MAP_1D_64K));
    assert_eq!(mapping.get(&CharMapping::OneD128K), Some(&NCGR_MAP_1D_128K));
    assert_eq!(mapping.get(&CharMapping::OneD256K), Some(&NCGR_MAP_1D_256K));

    assert_eq!(nclr, NCLR_MEMBERS, "NCLR member count");
    assert_eq!(nclr_fmt.get(&3), Some(&NCLR_FMT_16C));
    assert_eq!(nclr_fmt.get(&4), Some(&NCLR_FMT_256C));
    assert_eq!(nclr_fmt.get(&0x000A_0004), Some(&NCLR_FMT_FLAGGED));
    assert_eq!(nclr_extended, NCLR_EXTENDED);
    assert_eq!(nclr_pmcp, NCLR_WITH_PMCP);
    assert_eq!(nclr_compressed, NCLR_COMPRESSED);

    assert_eq!(nscr, NSCR_MEMBERS, "NSCR member count");
    assert_eq!(nscr_color[0], NSCR_COLOR_16);
    assert_eq!(nscr_color[1], NSCR_COLOR_256);
    assert_eq!(nscr_color[2], NSCR_COLOR_OTHER);
    assert_eq!(nscr_format[0], NSCR_FORMAT_TEXT);
    assert_eq!(nscr_format[1] + nscr_format[2], NSCR_FORMAT_OTHER);

    eprintln!("parsed {ncgr} NCGRs, {nclr} NCLRs, {nscr} NSCRs");
}

#[test]
fn spot_checks_known_graphics() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let narcs = all_narcs(&rom);
    let by_path = |path: &str| -> &Narc<'_> {
        narcs
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, n)| n)
            .unwrap_or_else(|| panic!("expected {path} to be a NARC in the NitroFS"))
    };

    // The Johto follower-sprite archive: member 0 is the NCGR sheet for
    // the first follower, member 4 its NCLR.
    let followers = by_path("a/0/0/4");
    let ncgr = Ncgr::parse(followers.file(0).expect("member 0")).expect("follower NCGR");
    assert_eq!((ncgr.height(), ncgr.width()), (Some(10), Some(20)));
    assert_eq!(ncgr.pixel_fmt(), PixelFmt::Pltt16);
    assert_eq!(ncgr.tile_data().len(), 10 * 20 * 32);

    let nclr = Nclr::parse(followers.file(4).expect("member 4")).expect("follower NCLR");
    assert_eq!(nclr.fmt_raw(), 0x000A_0004);
    assert_eq!(nclr.logical_size(), 32);
    assert_eq!(nclr.color_count(), 16);

    // A real follower palette: all 16 colors of pokegra member 10 are
    // set (BGR555; 0x7FFF is white).
    let pokegra = by_path("pbr/pokegra.narc");
    let nclr = Nclr::parse(pokegra.file(10).expect("member 10")).expect("pokegra NCLR");
    assert_eq!(nclr.fmt_raw(), 0x000A_0004);
    let colors = nclr.palette_data();
    assert_eq!(colors.len(), 32);
    assert!(colors.chunks_exact(2).all(|c| c != [0, 0]));

    // A full-screen text map exists: 256×192 px = 32×24 tiles, u16
    // entries = 1536 bytes.
    let fullscreen = narcs
        .iter()
        .filter_map(|(_, narc)| {
            (0..narc.file_count()).find_map(|id| {
                let bytes = narc.file(id).ok()?;
                if !is_nscr(bytes) {
                    return None;
                }
                let nscr = Nscr::parse(bytes).ok()?;
                (nscr.width() == 256 && nscr.height() == 192 && nscr.entries().len() == 1_536)
                    .then_some(nscr)
            })
        })
        .count();
    assert!(fullscreen > 0, "a 256×192 text screen must exist");
}
