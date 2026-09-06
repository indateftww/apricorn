//! Integration tests: `apricorn-tools extract` on a retail HeartGold
//! (US) dump.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM). Runs the real CLI binary, then checks the tree and
//! manifest against the pinned retail census: 46 NitroFS directories,
//! 384 files, 129 overlays, 3 binaries — see `docs/extraction.md`.

use std::process::Command;

use apricorn_core::nds::NdsRom;
use sha2::{Digest as _, Sha256};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail census of hg_usa.nds's extractable content.
const DIRS: usize = 46;
const FILES: usize = 384;
const OVERLAYS: usize = 129;
const BINARIES: usize = 3; // header.bin, arm9.bin, arm7.bin

fn load_rom() -> Option<Vec<u8>> {
    match std::fs::read(ROM_PATH) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

/// One entry per hashed item: 3 binaries + 384 files + 129 overlays.
fn sha256_entries(manifest: &str) -> usize {
    manifest.matches("\"sha256\"").count()
}

#[test]
fn extracts_verified_tree_and_manifest() {
    let Some(data) = load_rom() else { return };
    let out = std::env::temp_dir().join(format!("apricorn-extract-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);

    let status = Command::new(env!("CARGO_BIN_EXE_apricorn-tools"))
        .args([
            "extract",
            ROM_PATH,
            out.to_str().expect("temp path is UTF-8"),
        ])
        .status()
        .expect("the apricorn-tools binary builds alongside tests");
    assert!(status.success(), "extract must report success");

    // The manifest identifies the ROM and hashes every item it wrote.
    let manifest = std::fs::read_to_string(out.join("manifest.json")).expect("manifest exists");
    assert!(manifest.starts_with("{\n  \"manifest_version\": 1,\n"));
    assert!(manifest.contains("\"game_code\": \"IPKE\""));
    assert!(manifest.contains("\"size\": 134217728"));
    assert_eq!(
        sha256_entries(&manifest),
        BINARIES + FILES + OVERLAYS,
        "one hash per binary, NitroFS file, and overlay"
    );
    assert_eq!(manifest.matches("\"fat_id\"").count(), FILES + OVERLAYS);
    // 127 of the 129 overlays are BLZ-compressed (backwards LZ77; the
    // compstatic flag, bit 24 of the compressed-size word); only 35 and
    // 124 are stored plain.
    assert_eq!(manifest.matches("\"compressed\": true").count(), 127);
    // The directories array carries all 46 paths — the root's empty
    // string included, one per line.
    let dirs_section = manifest
        .split("  \"directories\": [\n")
        .nth(1)
        .and_then(|rest| rest.split("\n  ],").next())
        .expect("manifest has a directories array");
    assert_eq!(dirs_section.lines().count(), DIRS);

    // The binaries are straight slices of the image.
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let header = std::fs::read(out.join("header.bin")).expect("header.bin extracted");
    assert_eq!(header.len(), 0x4000);
    assert_eq!(header, data[0..0x4000]);
    let arm9 = std::fs::read(out.join("arm9.bin")).expect("arm9.bin extracted");
    assert_eq!(arm9.len(), rom.header.arm9.size as usize);
    assert_eq!(
        arm9,
        data[rom.header.arm9.rom_offset as usize
            ..rom.header.arm9.rom_offset as usize + rom.header.arm9.size as usize]
    );

    // The overlays land under overlay/arm9/, one file each, byte-exact.
    let overlay_count = std::fs::read_dir(out.join("overlay/arm9"))
        .expect("overlay directory exists")
        .count();
    assert_eq!(overlay_count, OVERLAYS);
    let overlay0 =
        std::fs::read(out.join("overlay/arm9/overlay_0000.bin")).expect("overlay 0 exists");
    assert_eq!(overlay0, rom.file(0).expect("overlay 0 is FAT id 0"));

    // A NitroFS spot check: the main SDAT, byte-exact and hashed in the
    // manifest under its original path.
    let sdat_path = out.join("nitrofs/data/sound/gs_sound_data.sdat");
    let sdat = std::fs::read(&sdat_path).expect("the SDAT extracted");
    assert_eq!(sdat.len(), 8_660_096);
    assert_eq!(
        sdat,
        rom.file_by_path("data/sound/gs_sound_data.sdat").unwrap()
    );
    let hash: String = Sha256::digest(&sdat)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(
        manifest.contains(&hash),
        "the manifest hashes the SDAT as extracted"
    );
    assert!(manifest.contains("\"fat_id\": 479, \"offset\":"));

    // The NitroFS tree carries the retail census of files.
    let nitrofs_files = std::fs::read_dir(out.join("nitrofs"))
        .expect("nitrofs root extracted")
        .count();
    assert_eq!(nitrofs_files, 8, "the eight NitroFS root entries");

    std::fs::remove_dir_all(&out).expect("test cleans up after itself");
}
