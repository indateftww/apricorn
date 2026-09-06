//! Integration tests: every sprite cell bank (NCER) and animation bank
//! (NANR) in a retail HeartGold (US) dump.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM). The expected counts are ground truth established by
//! scanning the retail image while writing the parsers — see
//! `docs/nitro-sprite.md`.

use std::collections::HashMap;

use apricorn_core::formats::{
    AnimElement, AnimResult, CellMapping, Nanr, Ncer, PlayMode, is_nanr, is_narc, is_ncer,
};
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail HeartGold (US) cell-bank census.
const NCER_MEMBERS: usize = 608;
const NCER_EXTENDED: usize = 473; // extended records (bounding boxes)
const NCER_VRAM: usize = 157; // VRAM transfer blocks
const NCER_UCAT: usize = 146; // UCAT user-attribute blocks
const NCER_OAM_TOTAL: usize = 11_703; // OAM entries across every bank
const NCER_MAP_1D_32K: usize = 284;
const NCER_MAP_1D_64K: usize = 198;
const NCER_MAP_1D_128K: usize = 92;
const NCER_MAP_1D_256K: usize = 34;

/// Retail HeartGold (US) animation-bank census.
const NANR_MEMBERS: usize = 591;
const NANR_SEQUENCES: usize = 2_500;
const NANR_FRAMES: usize = 6_171;
const NANR_ELEM_CELL: usize = 2_246;
const NANR_ELEM_SRT: usize = 73;
const NANR_ELEM_TRANSLATE: usize = 181;
const NANR_PLAY_FORWARD: usize = 1_048;
const NANR_PLAY_FORWARD_LOOP: usize = 1_452;
const NANR_UAAT: usize = 146;

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
/// 308 retail archives must parse.
fn all_narcs<'a>(rom: &NdsRom<'a>) -> Vec<(String, apricorn_core::formats::Narc<'a>)> {
    let mut narcs = Vec::new();
    for file in rom.nitrofs().files() {
        let bytes = rom.file(file.fat_id).expect("fat id in range");
        if is_narc(bytes) {
            let narc = apricorn_core::formats::Narc::parse(bytes)
                .unwrap_or_else(|e| panic!("NARC {} failed to parse: {e}", file.path));
            narcs.push((file.path.clone(), narc));
        }
    }
    narcs
}

#[test]
fn parses_every_sprite_member_and_matches_census() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let narcs = all_narcs(&rom);

    let mut ncer = 0usize;
    let mut ncer_extended = 0usize;
    let mut ncer_vram = 0usize;
    let mut ncer_ucat = 0usize;
    let mut ncer_oam = 0usize;
    let mut mapping: HashMap<CellMapping, usize> = HashMap::new();

    let mut nanr = 0usize;
    let mut nanr_sequences = 0usize;
    let mut nanr_frames = 0usize;
    let mut nanr_uaat = 0usize;
    let mut element: HashMap<AnimElement, usize> = HashMap::new();
    let mut play: HashMap<PlayMode, usize> = HashMap::new();

    for (path, narc) in &narcs {
        for id in 0..narc.file_count() {
            let bytes = narc.file(id).expect("member id in range");
            let where_ = |what: &str| format!("{path} member {id}: {what}");
            if is_ncer(bytes) {
                let file = Ncer::parse(bytes)
                    .unwrap_or_else(|e| panic!("{}: {e}", where_("NCER must parse")));
                ncer += 1;
                if file.is_extended() {
                    ncer_extended += 1;
                }
                if file.vram_transfer().is_some() {
                    ncer_vram += 1;
                }
                if file.ucat().is_some() {
                    ncer_ucat += 1;
                }
                ncer_oam += file.cells().iter().map(|c| c.oam_count).sum::<usize>();
                *mapping.entry(file.mapping()).or_default() += 1;
            } else if is_nanr(bytes) {
                let file = Nanr::parse(bytes)
                    .unwrap_or_else(|e| panic!("{}: {e}", where_("NANR must parse")));
                nanr += 1;
                nanr_sequences += file.sequence_count();
                nanr_frames += file.total_frames();
                if file.uaat().is_some() {
                    nanr_uaat += 1;
                }
                for seq in file.sequences() {
                    *element.entry(seq.element()).or_default() += 1;
                    *play.entry(seq.play_mode()).or_default() += 1;
                    // Every frame's result decodes from the shared pool.
                    for i in 0..seq.frame_count() {
                        assert!(
                            seq.result(i).is_some(),
                            "{}",
                            where_("frame result must decode")
                        );
                    }
                }
            }
        }
    }

    assert_eq!(ncer, NCER_MEMBERS, "NCER member count");
    assert_eq!(ncer_extended, NCER_EXTENDED);
    assert_eq!(ncer_vram, NCER_VRAM);
    assert_eq!(ncer_ucat, NCER_UCAT);
    assert_eq!(ncer_oam, NCER_OAM_TOTAL);
    assert_eq!(mapping.get(&CellMapping::OneD32K), Some(&NCER_MAP_1D_32K));
    assert_eq!(mapping.get(&CellMapping::OneD64K), Some(&NCER_MAP_1D_64K));
    assert_eq!(mapping.get(&CellMapping::OneD128K), Some(&NCER_MAP_1D_128K));
    assert_eq!(mapping.get(&CellMapping::OneD256K), Some(&NCER_MAP_1D_256K));
    // No retail bank declares 2D mapping (raw value 4): the enum's TwoD
    // variant exists but HeartGold never emits it.
    assert_eq!(mapping.get(&CellMapping::TwoD), None);

    assert_eq!(nanr, NANR_MEMBERS, "NANR member count");
    assert_eq!(nanr_sequences, NANR_SEQUENCES);
    assert_eq!(nanr_frames, NANR_FRAMES);
    assert_eq!(nanr_uaat, NANR_UAAT);
    assert_eq!(element.get(&AnimElement::Cell), Some(&NANR_ELEM_CELL));
    assert_eq!(element.get(&AnimElement::Srt), Some(&NANR_ELEM_SRT));
    assert_eq!(
        element.get(&AnimElement::Translate),
        Some(&NANR_ELEM_TRANSLATE)
    );
    assert_eq!(play.get(&PlayMode::Forward), Some(&NANR_PLAY_FORWARD));
    assert_eq!(
        play.get(&PlayMode::ForwardLoop),
        Some(&NANR_PLAY_FORWARD_LOOP)
    );

    eprintln!(
        "parsed {ncer} NCERs ({ncer_oam} OAM), {nanr} NANRs ({nanr_sequences} sequences, {nanr_frames} frames)"
    );
}

#[test]
fn spot_checks_known_sprite_banks() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let narcs = all_narcs(&rom);
    let by_path = |path: &str| -> &apricorn_core::formats::Narc<'_> {
        narcs
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, n)| n)
            .unwrap_or_else(|| panic!("expected {path} to be a NARC in the NitroFS"))
    };

    // The SDK's default animation, shipped loose in the NitroFS: one
    // sequence, one frame, showing cell 0 for 4 ticks.
    let fat_id = rom
        .nitrofs()
        .fat_id_by_path("data/clact_default.NANR")
        .expect("data/clact_default.NANR exists");
    let nanr = Nanr::parse(rom.file(fat_id).expect("fat id in range"))
        .expect("clact_default.NANR must parse");
    let seq = nanr.sequence(0).unwrap();
    assert_eq!(seq.label(), "CellAnime0");
    assert_eq!(seq.element(), AnimElement::Cell);
    assert_eq!(seq.play_mode(), PlayMode::ForwardLoop);
    assert_eq!(seq.frames()[0].delay, 4);
    assert_eq!(seq.result(0), Some(AnimResult::Cell { cell: 0 }));

    // A cell bank with every optional block: one extended cell of six
    // OAM entries, a VRAM transfer block, and a UCAT block.
    let ncer = Ncer::parse(by_path("a/0/5/8").file(2).expect("member 2"))
        .expect("a/0/5/8 member 2 must be a NCER");
    let cell = ncer.cell(0).unwrap();
    assert_eq!(ncer.cell_count(), 1);
    assert!(ncer.is_extended());
    assert_eq!(cell.oam_count, 6);
    assert_eq!(cell.radius(), 0xE);
    assert!(cell.has_bounding_rect());
    assert_eq!(
        cell.bounding_box(),
        Some(apricorn_core::formats::BoundingBox {
            max_x: 39,
            max_y: 39,
            min_x: -40,
            min_y: -40
        })
    );
    assert_eq!(cell.oam_attr(0), Some((216, 49_624, 0)));
    let vram = ncer.vram_transfer().expect("VRAM transfer block");
    assert_eq!(vram.sz_byte_max, 3_200);
    assert_eq!(ncer.ucat().expect("UCAT block").attr(0), Some(0));

    // The follow-walk scene banks: 224 cells named by region (johto*,
    // kanto*), 16 labels — label count independent of cell count.
    let ncer = Ncer::parse(by_path("a/0/4/9").file(58).expect("member 58"))
        .expect("a/0/4/9 member 58 must be a NCER");
    assert_eq!(ncer.cell_count(), 224);
    assert_eq!(ncer.labels().len(), 16);
    assert_eq!(
        &ncer.labels()[..8],
        &[
            "johto0", "johto1", "johto2", "johto3", "johto4", "johto5", "johto6", "johto7"
        ]
    );

    // An SRT sequence from the bag-graphics bank: frame 0 shows cell 1,
    // unrotated at fx32 scale 1.0 (rotation is the raw rotZ counter).
    let nanr = Nanr::parse(by_path("pbr/bag_gra.narc").file(0).expect("member 0"))
        .expect("pbr/bag_gra.narc member 0 must be a NANR");
    assert_eq!(nanr.sequence_count(), 16);
    let seq = nanr.sequence(8).unwrap();
    assert_eq!(seq.element(), AnimElement::Srt);
    assert_eq!(seq.frames()[0].delay, 2);
    assert_eq!(
        seq.result(0),
        Some(AnimResult::Srt {
            cell: 1,
            rotation: 64_080,
            scale_x: 4_096,
            scale_y: 4_096,
            x: 0,
            y: 0
        })
    );

    // A translate-only sequence from the Poké Ball icon bank.
    let nanr = Nanr::parse(by_path("pbr/poke_icon.narc").file(3).expect("member 3"))
        .expect("pbr/poke_icon.narc member 3 must be a NANR");
    let seq = nanr.sequence(5).unwrap();
    assert_eq!(seq.element(), AnimElement::Translate);
    assert_eq!(seq.frame_count(), 3);
    assert_eq!(seq.frames()[0].delay, 32);
    assert_eq!(
        seq.result(0),
        Some(AnimResult::Translate {
            cell: 0,
            x: 0,
            y: 0
        })
    );

    // An animation bank with a UAAT block: both attributes are zero.
    let nanr = Nanr::parse(by_path("a/0/5/8").file(3).expect("member 3"))
        .expect("a/0/5/8 member 3 must be a NANR");
    let uaat = nanr.uaat().expect("UAAT block");
    assert_eq!(uaat.seq_attr(0), Some(0));
    assert_eq!(nanr.frame_attr(0, 0), Some(0));
}
