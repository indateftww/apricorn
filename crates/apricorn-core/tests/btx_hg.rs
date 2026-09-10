//! Integration tests: every texture archive (BTX) in a retail HeartGold
//! (US) dump.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM). The expected counts are ground truth established by
//! scanning the retail image while writing the parser — see
//! `docs/nitro-btx.md`.

use std::collections::HashMap;

use apricorn_core::formats::{Btx, BtxPalette, BtxTexture, Narc, TexFmt, is_btx, is_narc};
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail HeartGold (US) texture-archive census.
const BTX_FILES: usize = 1_157; // all NSBTX files
const BTX_LOOSE: usize = 11; // NitroFS `data/*.nsbtx`
const BTX_TEX_ENTRIES: usize = 14_735;
const BTX_PLTT_ENTRIES: usize = 7_729;
const TEX_PLTT16: usize = 12_794;
const TEX_PLTT4: usize = 1_617;
const TEX_A5I3: usize = 169;
const TEX_A3I5: usize = 145;
const TEX_PLTT256: usize = 10;
const TEX_COLOR0_TRANSPARENT: usize = 10_411;
const BTX_USES_PLTT4: usize = 216; // files whose palettes are 4-color sets
const PLTT_WORD1_ONE: usize = 1_612; // palette entries with word1 == 1
/// Correct 2-bpp PLTT4 decoding leaves no truncated retail textures.
const BTX_OVERRUN_FILES: usize = 0;
const BTX_OVERRUN_ENTRIES: usize = 0;

fn load_rom() -> Option<Vec<u8>> {
    match std::fs::read(ROM_PATH) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

#[test]
fn parses_every_btx_and_matches_census() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    let mut files = 0usize;
    let mut loose = 0usize;
    let mut tex_entries = 0usize;
    let mut pltt_entries = 0usize;
    let mut fmts: HashMap<TexFmt, usize> = HashMap::new();
    let mut color0_transparent = 0usize;
    let mut uses_pltt4 = 0usize;
    let mut word1_one = 0usize;
    let mut overrun_files = 0usize;
    let mut overrun_entries = 0usize;

    let mut check = |path: &str, btx: &Btx<'_>| {
        files += 1;
        if btx.uses_pltt4() {
            uses_pltt4 += 1;
        }
        let mut file_overruns = 0usize;
        let mut names = std::collections::HashSet::new();
        for tex in btx.textures() {
            tex_entries += 1;
            *fmts.entry(tex.fmt()).or_default() += 1;
            if tex.color0_transparent() {
                color0_transparent += 1;
            }
            if tex.declared_size() > tex.data().len() {
                file_overruns += 1;
            }
            // Data slices stay inside the texture-data area, at or after
            // the texture's own offset.
            assert!(
                tex.data().len() <= btx.texture_data().len() - tex.offset(),
                "{path}: texture {} escapes the texture data",
                tex.name()
            );
            names.insert(tex.name());
        }
        if file_overruns > 0 {
            overrun_files += 1;
            overrun_entries += file_overruns;
        }
        assert_eq!(
            names.len(),
            btx.texture_count(),
            "{path}: retail texture names are unique per file"
        );
        let mut names = std::collections::HashSet::new();
        for pltt in btx.palettes() {
            pltt_entries += 1;
            if pltt.word1() == 1 {
                word1_one += 1;
            }
            assert!(
                pltt.data().len() <= btx.palette_data().len() - pltt.offset(),
                "{path}: palette {} escapes the palette data",
                pltt.name()
            );
            names.insert(pltt.name());
        }
        assert_eq!(
            names.len(),
            btx.palette_count(),
            "{path}: retail palette names are unique per file"
        );
    };

    for file in rom.nitrofs().files() {
        let bytes = rom.file(file.fat_id).expect("fat id in range");
        let path = &file.path;
        if is_btx(bytes) {
            loose += 1;
            let btx = Btx::parse(bytes).unwrap_or_else(|e| panic!("{path} failed to parse: {e}"));
            check(path, &btx);
        } else if is_narc(bytes) {
            let narc =
                Narc::parse(bytes).unwrap_or_else(|e| panic!("NARC {path} failed to parse: {e}"));
            for id in 0..narc.file_count() {
                let member = narc.file(id).expect("member id in range");
                if is_btx(member) {
                    let btx = Btx::parse(member)
                        .unwrap_or_else(|e| panic!("{path} member {id} failed to parse: {e}"));
                    check(path, &btx);
                }
            }
        }
    }

    assert_eq!(files, BTX_FILES, "BTX file count");
    assert_eq!(loose, BTX_LOOSE, "loose NSBTX count");
    assert_eq!(files - loose, BTX_FILES - BTX_LOOSE, "NARC member count");
    assert_eq!(tex_entries, BTX_TEX_ENTRIES, "texture entries");
    assert_eq!(pltt_entries, BTX_PLTT_ENTRIES, "palette entries");
    assert_eq!(fmts.get(&TexFmt::Pltt16), Some(&TEX_PLTT16));
    assert_eq!(fmts.get(&TexFmt::Pltt4), Some(&TEX_PLTT4));
    assert_eq!(fmts.get(&TexFmt::A5i3), Some(&TEX_A5I3));
    assert_eq!(fmts.get(&TexFmt::A3i5), Some(&TEX_A3I5));
    assert_eq!(fmts.get(&TexFmt::Pltt256), Some(&TEX_PLTT256));
    assert_eq!(color0_transparent, TEX_COLOR0_TRANSPARENT);
    assert_eq!(uses_pltt4, BTX_USES_PLTT4);
    assert_eq!(word1_one, PLTT_WORD1_ONE);
    assert_eq!(overrun_files, BTX_OVERRUN_FILES);
    assert_eq!(overrun_entries, BTX_OVERRUN_ENTRIES);

    eprintln!(
        "parsed {files} BTX files ({loose} loose): {tex_entries} textures, {pltt_entries} palettes"
    );
}

/// The NARC at `path`, for member spot checks.
fn narc<'a>(rom: &NdsRom<'a>, path: &str) -> Narc<'a> {
    let fat_id = rom
        .nitrofs()
        .fat_id_by_path(path)
        .unwrap_or_else(|| panic!("expected {path} in the NitroFS"));
    let bytes = rom.file(fat_id).expect("fat id in range");
    Narc::parse(bytes).unwrap_or_else(|e| panic!("expected {path} to be a NARC: {e}"))
}

/// The loose BTX at `path`.
fn loose<'a>(rom: &NdsRom<'a>, path: &str) -> Btx<'a> {
    let fat_id = rom
        .nitrofs()
        .fat_id_by_path(path)
        .unwrap_or_else(|| panic!("expected {path} in the NitroFS"));
    Btx::parse(rom.file(fat_id).expect("fat id in range"))
        .unwrap_or_else(|e| panic!("{path} must parse: {e}"))
}

#[test]
fn spot_checks_known_archives() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    // The largest field-texture bank: 67 textures, 66 palettes, 4-color
    // palettes; cavekn opens the file at offset 0.
    let btx = Btx::parse(narc(&rom, "a/0/4/4").file(2).expect("member 2"))
        .expect("a/0/4/4 member 2 must be a BTX");
    assert_eq!(btx.texture_count(), 67);
    assert_eq!(btx.palette_count(), 66);
    assert!(btx.uses_pltt4());
    assert_eq!(btx.texture_data().len(), 32_000);
    assert_eq!(btx.palette_data().len(), 1_520);
    let cavekn = btx.texture_by_name("cavekn").expect("texture cavekn");
    assert_eq!((cavekn.width(), cavekn.height()), (32, 32));
    assert_eq!(cavekn.fmt(), TexFmt::Pltt16);
    assert_eq!(cavekn.offset(), 0);
    assert!(cavekn.color0_transparent());
    // Textures legitimately share offsets: road05 reuses road01's data.
    assert_eq!(
        btx.texture_by_name("road05").map(BtxTexture::offset),
        btx.texture_by_name("road01").map(BtxTexture::offset)
    );

    // The biggest bank in the ROM: ends with a 256x256 PLTT256 sky
    // gradient whose span ends the file exactly.
    let btx = Btx::parse(narc(&rom, "a/0/7/0").file(26).expect("member 26"))
        .expect("a/0/7/0 member 26 must be a BTX");
    assert_eq!(btx.texture_count(), 11);
    assert_eq!(btx.texture_data().len(), 70_656);
    let sky = btx.texture_by_name("kk_sky_grad_").expect("kk_sky_grad_");
    assert_eq!((sky.width(), sky.height()), (256, 256));
    assert_eq!(sky.fmt(), TexFmt::Pltt256);
    assert_eq!(sky.offset(), 0x1400);
    assert_eq!(sky.offset() + sky.data().len(), 70_656);

    // The four-color shadow is 2 bpp: its complete span is 64 bytes.
    let btx = Btx::parse(narc(&rom, "a/0/7/0").file(30).expect("member 30"))
        .expect("a/0/7/0 member 30 must be a BTX");
    let kage = btx.texture_by_name("h_kage").expect("h_kage");
    assert_eq!(kage.fmt(), TexFmt::Pltt4);
    assert_eq!(kage.declared_size(), 64);
    assert_eq!(kage.data().len(), 64);

    // A bank whose palette names fill all 16 bytes of their dictionary
    // slots with no NUL terminator.
    let btx = Btx::parse(narc(&rom, "a/0/4/4").file(27).expect("member 27"))
        .expect("a/0/4/4 member 27 must be a BTX");
    assert_eq!(btx.texture_count(), 99);
    assert_eq!(btx.palette_count(), 98);
    assert_eq!(
        btx.palettes().get(87).map(BtxPalette::name),
        Some("fh01_19tuta02_pl")
    );

    // A loose file: the sea-dungeon animation sheet, eight 16x16 tiles
    // with one shared 16-byte palette — no PLTT4 flag.
    let btx = loose(&rom, "data/dun_sea.nsbtx");
    assert_eq!(btx.texture_count(), 8);
    assert_eq!(btx.palette_count(), 1);
    assert!(!btx.uses_pltt4());
    assert_eq!(btx.texture_data().len(), 1_024);
    assert_eq!(btx.palette_data().len(), 16);
    assert_eq!(btx.textures()[7].name(), "dun_sea.8");

    // A loose PLTT4 file: four 16x16 tiles sharing offset 0 for every
    // palette, word1 == 1 on all four.
    let btx = loose(&rom, "data/t3_fl_r.nsbtx");
    assert!(btx.uses_pltt4());
    assert_eq!(btx.texture_count(), 4);
    for pltt in btx.palettes() {
        assert_eq!(pltt.offset(), 0);
        assert_eq!(pltt.word1(), 1);
    }
}
