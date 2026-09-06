//! Integration tests: `apricorn-tools convert` on a retail HeartGold
//! (US) dump.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM). Runs the real CLI binary into a fresh temp dir, then
//! checks the cache and manifest against the pinned retail census — see
//! `docs/conversion.md` for the per-kind ground truth.

use std::process::Command;

use apricorn_core::cache::{self, ChunkKind};
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail census of hg_usa.nds's convertible content.
///
/// Beyond the raw members, the counts take in the LZ77-10-compressed
/// population (4,221 decodable members ROM-wide): 1,509 NCGRs, 707 NSCRs,
/// 367 NCERs and 367 NANRs ship as images behind the 0x10 magic (no NCLR
/// is ever compressed).
///
/// The tiles figure is 24 short of the convertible total: 23 LZ77-10
/// images in `a/0/0/7` omit the trailing CPOS section their container
/// header still lists (the file's own 16 bytes missing — retail
/// inconsistency, skipped; see `docs/conversion.md`), plus
/// `data/dp_areawindow.NCGR`, a DP leftover whose container header is
/// corrupt three ways (zero BOM, file and section sizes each
/// under-declared by 8 bytes).
///
/// The text figure is 829 banks from `a/0/2/7` plus 624 more genuine MAT
/// banks living in `pbr/msg.narc` — the LZ77-10 sniff falls through to
/// the MAT sniff so banks whose message count is 0x0010 still convert.
const TILES: usize = 9_458;
const PALETTES: usize = 4_953;
const SCREENS: usize = 1_500;
const CELLS: usize = 979;
const ANIMS: usize = 963;
const TEXT: usize = 1_453;
const CHUNKS: usize = TILES + PALETTES + SCREENS + CELLS + ANIMS + TEXT;
const OVERLAYS: usize = 129;
const BLZ_DECOMPRESSED: usize = 127;

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
fn converts_verified_cache_and_manifest() {
    let Some(data) = load_rom() else { return };
    let out = std::env::temp_dir().join(format!("apricorn-convert-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let out_str = out.to_str().expect("temp path is UTF-8");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_apricorn-tools"))
            .args(["convert", ROM_PATH, out_str])
            .output()
            .expect("the apricorn-tools binary builds alongside tests")
    };

    // First run: full conversion.
    let first = run();
    assert!(first.status.success(), "convert must report success");
    assert!(
        String::from_utf8_lossy(&first.stdout).contains("19306 chunks"),
        "the summary reports the chunk total"
    );

    // Second run: same ROM SHA-1, manifest present — must skip the rebuild.
    let second = run();
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("up to date"));

    // The manifest identifies the ROM and hashes every output.
    let root = out.join("cache");
    let manifest = std::fs::read_to_string(root.join("cache.json")).expect("cache manifest");
    assert!(manifest.starts_with("{\n  \"cache_version\": 1,\n"));
    assert_eq!(
        manifest
            .matches(&format!("\"chunk_version\": {},\n", cache::VERSION))
            .count(),
        1
    );
    assert!(
        manifest.contains("\"sha1\": \"4fcded0e2713dc03929845de631d0932ea2b5a37\""),
        "the manifest carries the ROM's SHA-1"
    );
    assert!(manifest.contains("\"size\": 134217728"));

    // The census, one `"kind": "..."` per chunk.
    for (kind, expect) in [
        ("tiles", TILES),
        ("palette", PALETTES),
        ("screen", SCREENS),
        ("cells", CELLS),
        ("animation", ANIMS),
        ("text", TEXT),
    ] {
        assert_eq!(
            manifest.matches(&format!("\"kind\": \"{kind}\"")).count(),
            expect,
            "{kind} chunks"
        );
    }
    assert_eq!(
        manifest.matches("\"sha256\"").count(),
        CHUNKS + OVERLAYS,
        "one hash per chunk and overlay"
    );

    // The text banks split between the script archive and pbr/msg.narc.
    assert_eq!(
        manifest
            .lines()
            .filter(|l| l.contains("\"kind\": \"text\"") && l.contains("a/0/2/7#"))
            .count(),
        829
    );
    assert_eq!(
        manifest
            .lines()
            .filter(|l| l.contains("\"kind\": \"text\"") && l.contains("pbr/msg.narc#"))
            .count(),
        624
    );

    // The overlays: all 129 present, the 127 BLZ images decompressed to
    // their overlay-table raw sizes.
    assert_eq!(
        manifest.matches("\"compressed\": true").count(),
        BLZ_DECOMPRESSED
    );
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    for o in rom.overlays() {
        let path = root.join(format!("overlay/arm9/overlay_{:04}.bin", o.id));
        let bytes = std::fs::read(&path).unwrap_or_else(|_| panic!("overlay {} written", o.id));
        assert_eq!(
            bytes.len(),
            o.raw_size as usize,
            "overlay {} raw size",
            o.id
        );
    }

    // A PMCP-less palette chunk that the core reader must accept end to
    // end — this is the member that caught the palette count asymmetry.
    let pal_path = root.join("nitrofs/pbr/b_plist_gra.narc/23.pal");
    let pal = std::fs::read(&pal_path).expect("the b_plist_gra palette chunk exists");
    assert_eq!(cache::kind_of(&pal), Some(ChunkKind::Palette));
    let parsed = cache::Palette::parse(&pal).expect("the chunk round-trips through the reader");
    assert!(parsed.pmcp().is_empty(), "the chunk carries no PMCP table");

    // The corrupt dp_areawindow.NCGR does not become a chunk.
    assert!(!root.join("nitrofs/data/dp_areawindow.tiles").exists());

    // Nor do the 23 retail-inconsistent `a/0/0/7` NCGRs: their LZ77-10
    // images drop the trailing CPOS section the container header still
    // counts, so the strict parsers skip them (see the census constant).
    assert!(!root.join("nitrofs/a/0/0/7/9.tiles").exists());

    // The LZ77-10 members of the intro movie's gs_opening NARC
    // (`a/2/6/2`) convert: the Game Freak logo and copyright-beat
    // char/screen files — the assets Phase 3's copyright beat renders.
    for (member, kind) in [(4, "tiles"), (5, "tiles"), (12, "screen"), (14, "screen")] {
        let path = root.join(format!("nitrofs/a/2/6/2/{member}.{kind}"));
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|_| panic!("gs_opening member {member} converts to .{kind}"));
        let parsed_kind = cache::kind_of(&bytes).expect("a cache chunk");
        assert_eq!(
            parsed_kind.extension(),
            kind,
            "gs_opening member {member} is a .{kind} chunk"
        );
    }

    std::fs::remove_dir_all(&out).expect("test cleans up after itself");
}
