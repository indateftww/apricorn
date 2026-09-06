//! Integration tests against a real retail HeartGold (US) dump.
//!
//! These run only when `hg_usa.nds` sits at the repo root (each developer
//! supplies their own ROM; CI has none, so the tests skip silently).
//! The expected values are ground truth established in Phase 0:
//! pret/pokeheartgold builds byte-identical retail ROMs from its manifest,
//! and the counts below were cross-checked against the ROM's own FNT/FAT.

use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");
const PRET_FILES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../refs/pokeheartgold/files"
);

/// Retail HeartGold (US) facts.
const TITLE: &str = "POKEMON HG";
const GAME_CODE: &str = "IPKE";
const DIR_COUNT: usize = 46;
const FILE_COUNT: usize = 384;
const FAT_COUNT: usize = 513;
const OVERLAY_COUNT: usize = 129;
const ROOT_DIRS: [&str; 8] = [
    "a",
    "data",
    "dwc",
    "fielddata",
    "msgdata",
    "pbr",
    "poketool",
    "tel",
];

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
fn parses_retail_header() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    assert_eq!(rom.header.title, TITLE);
    assert_eq!(rom.header.game_code_str(), GAME_CODE);
    assert_eq!(rom.header.application_end_offset, 0x078C_763C);
    assert_eq!(rom.header.rom_header_size, 0x4000);
    // ARM9 binary ends before the overlay table (with alignment padding
    // in between on retail).
    assert!(rom.header.arm9.rom_offset + rom.header.arm9.size <= rom.header.arm9_overlay.offset);
    assert!(
        rom.header_crc_ok(),
        "header CRC must verify against the retail dump"
    );
    assert!(
        rom.logo_crc_ok(),
        "logo CRC must verify against the retail dump"
    );
}

#[test]
fn nitrofs_matches_pret_manifest_scale() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let fs = rom.nitrofs();

    assert_eq!(fs.dirs().len(), DIR_COUNT);
    assert_eq!(fs.files().len(), FILE_COUNT);
    assert_eq!(fs.fat().len(), FAT_COUNT);

    let mut roots: Vec<&str> = fs
        .dirs()
        .iter()
        .filter(|d| d.parent == 0xF000)
        .map(|d| d.path.as_str())
        .collect();
    roots.sort_unstable();
    assert_eq!(roots, ROOT_DIRS);
}

#[test]
fn overlays_occupy_first_fat_ids() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    assert_eq!(rom.overlays().len(), OVERLAY_COUNT);
    for (i, overlay) in rom.overlays().iter().enumerate() {
        assert_eq!(overlay.id, i as u32, "overlays must be numbered in order");
        assert_eq!(overlay.fat_id, i as u32, "overlays occupy FAT ids 0..129");
    }
}

/// The strongest available extraction check: every NitroFS file that has a
/// same-path counterpart in pret's `files/` tree must be byte-identical to
/// it. pret rebuilds some files from converted sources (message banks as
/// JSON, NARCs unpacked), so those have no counterpart and are counted as
/// skipped instead.
#[test]
fn extracts_files_identical_to_pret_tree() {
    let Some(data) = load_rom() else { return };
    let pret_root = std::path::Path::new(PRET_FILES);
    if !pret_root.is_dir() {
        eprintln!("skipping: {PRET_FILES} not found (clone pret/pokeheartgold into refs/)");
        return;
    }
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    let mut compared = 0usize;
    let mut skipped = 0usize;
    for file in rom.nitrofs().files() {
        let reference = match std::fs::read(pret_root.join(&file.path)) {
            Ok(reference) => reference,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let extracted = rom.file(file.fat_id).expect("fat id in range");
        assert_eq!(
            extracted.len(),
            reference.len(),
            "size mismatch for {} (extracted vs pret)",
            file.path
        );
        assert!(
            extracted == reference.as_slice(),
            "byte mismatch for {} (extracted vs pret)",
            file.path
        );
        compared += 1;
    }
    assert!(compared > 0, "pret tree present but nothing to compare?");
    eprintln!(
        "compared {compared} NitroFS files against pret, {skipped} skipped (no source counterpart)"
    );
}
