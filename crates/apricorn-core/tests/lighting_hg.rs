//! Integration tests: the field area-light archives against a retail
//! HeartGold (US) dump (`apricorn_core::field::lighting`).
//!
//! The five `data/*light.txt` tables parse through the loader's exact
//! two-pass walk; these tests pin their shapes (record counts, the
//! half-second thresholds' ordering and range), lock the parsed values
//! behind SHA-1 goldens over a canonical serialization, and step the
//! manager through a whole day against the direct selection. See
//! `docs/day-night.md` for the format and its evidence.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM dump).

use std::path::Path;

use apricorn_core::assets::AssetStore;
use apricorn_core::field::lighting::{
    ARCHIVE_PATHS, AreaLightArchive, AreaLightManager, HALF_SECONDS_PER_DAY, LIGHT_COUNT,
    ModelLighting, archive_for_light_type,
};
use apricorn_core::formats::Narc;
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Records per archive, in [`ARCHIVE_PATHS`] order: the three area
/// tables step through the day, the two dungeon variants are flat.
const RECORD_COUNTS: [usize; 5] = [15, 15, 15, 1, 1];

/// Each archive's first threshold (half-seconds): `area00` opens with
/// a zero-length record, the other two area tables with a short one,
/// the flat dungeon tables with the fallback-only 0.
const FIRST_UNTIL: [u32; 5] = [0, 900, 900, 0, 0];

/// SHA-1 of [`canonical`] per archive.
const GOLDENS: [&str; 5] = [
    "a405e74461fc8ad2d58af723ad2d6ac668adf848",
    "33ef70d0737ef496c01747bb6153fc65d674694b",
    "aa27e8bcad194b4205a6a5b3fb9aebd6b08ec3c2",
    "2c235b1dd32fb92b1f6c87d08f5e36470b7125c2",
    "d6d81069084c327f2408d123d3733d1373b1059c",
];

/// Opens the retail dump, or `None` to skip silently.
fn store() -> Option<AssetStore> {
    match AssetStore::open(Path::new(ROM_PATH)) {
        Ok(store) => Some(store),
        Err(apricorn_core::assets::AssetsError::Io(_)) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
        Err(err) => panic!("retail ROM must open: {err}"),
    }
}

fn archives(store: &AssetStore) -> Vec<AreaLightArchive> {
    (0..ARCHIVE_PATHS.len())
        .map(|id| AreaLightArchive::load(store, id).unwrap_or_else(|e| panic!("archive {id}: {e}")))
        .collect()
}

/// A fixed little-endian serialization of every parsed field, in
/// record order — what the goldens hash.
fn canonical(archive: &AreaLightArchive) -> Vec<u8> {
    let mut out = Vec::new();
    for t in archive.templates() {
        out.extend_from_slice(&t.until.to_le_bytes());
        for light in &t.lights {
            out.push(u8::from(light.enabled));
            out.extend_from_slice(&light.color.to_le_bytes());
            for axis in light.vector {
                out.extend_from_slice(&axis.to_le_bytes());
            }
        }
        for color in [t.diffuse, t.ambient, t.specular, t.emission] {
            out.extend_from_slice(&color.to_le_bytes());
        }
    }
    out
}

/// Whether `LIGHTING_HG_COLLECT` is set: print what the goldens pin.
fn collecting() -> bool {
    std::env::var_os("LIGHTING_HG_COLLECT").is_some()
}

#[test]
fn archives_parse_with_pinned_shapes() {
    let Some(store) = store() else { return };
    let archives = archives(&store);
    for (id, archive) in archives.iter().enumerate() {
        let path = ARCHIVE_PATHS[id];
        let t = archive.templates();
        if collecting() {
            let untils: Vec<u32> = t.iter().map(|r| r.until).collect();
            eprintln!("{path}: {} records, untils {untils:?}", t.len());
        }
        assert_eq!(t.len(), RECORD_COUNTS[id], "{path} record count");
        // Thresholds ascend strictly and fit the half-second day.
        for pair in t.windows(2) {
            assert!(pair[0].until < pair[1].until, "{path} thresholds ascend");
        }
        assert!(
            t.iter().all(|r| r.until <= HALF_SECONDS_PER_DAY),
            "{path} thresholds within a day"
        );
        assert_eq!(t[0].until, FIRST_UNTIL[id], "{path} first threshold");
        // Every record's colors are 15-bit and vectors within ±4096.
        for r in t {
            assert_eq!(r.lights.len(), LIGHT_COUNT);
            for light in &r.lights {
                assert!(light.color < 0x8000, "{path} light color");
                assert!(light.vector.iter().all(|v| v.abs() <= 4096), "{path} vector");
                if !light.enabled {
                    assert_eq!(light.color, 0);
                }
            }
            for c in [r.diffuse, r.ambient, r.specular, r.emission] {
                assert!(c < 0x8000, "{path} material color");
            }
        }
    }
    // The stepping tables' last record runs to midnight.
    for archive in &archives[..3] {
        let t = archive.templates();
        assert_eq!(t.last().map(|r| r.until), Some(HALF_SECONDS_PER_DAY));
    }
    // The flat dungeon tables are one record whose threshold is 0: no
    // half-second is ever below it, so the boot scan always falls back
    // to record 0 and the manager (one record) never steps.
    for archive in &archives[3..] {
        assert_eq!(archive.window(0), (0, 0));
        for second in [0, 4 * 3600, 43_199, 86_399] {
            assert_eq!(archive.initial_index(second), 0);
        }
    }
}

#[test]
fn parsed_values_match_the_goldens() {
    let Some(store) = store() else { return };
    let hashes: Vec<String> = archives(&store)
        .iter()
        .map(|archive| sha1::hex(&canonical(archive)))
        .collect();
    if collecting() {
        for (path, hash) in ARCHIVE_PATHS.iter().zip(&hashes) {
            eprintln!("{path}: sha1 {hash}");
        }
    }
    for (id, hash) in hashes.iter().enumerate() {
        assert_eq!(hash, GOLDENS[id], "{} golden", ARCHIVE_PATHS[id]);
    }
}

#[test]
fn morning_and_night_templates_differ() {
    let Some(store) = store() else { return };
    for (archive, path) in archives(&store)[..3].iter().zip(ARCHIVE_PATHS) {
        let nine = archive.template_at(9 * 3600);
        let twenty_one = archive.template_at(21 * 3600);
        assert_ne!(nine, twenty_one, "{path} 09:00 vs 21:00");
        // The night record is the day's last; the morning one is not.
        assert_eq!(archive.initial_index(21 * 3600), archive.templates().len() - 1, "{path}");
        assert!(archive.initial_index(9 * 3600) < archive.templates().len() - 1, "{path}");
        // Light 0 is the sun/moon key light in every stepping record.
        assert!(nine.lights[0].enabled && twenty_one.lights[0].enabled, "{path} key light");
    }
    // The flat dungeon tables show one template all day.
    for (archive, path) in archives(&store)[3..].iter().zip(&ARCHIVE_PATHS[3..]) {
        assert_eq!(archive.template_at(9 * 3600), archive.template_at(21 * 3600), "{path}");
    }
}

#[test]
fn manager_tracks_the_direct_selection_across_a_day() {
    let Some(store) = store() else { return };
    for (archive, path) in archives(&store).iter().zip(ARCHIVE_PATHS) {
        let mut attrs = ModelLighting::default();
        let mut manager = AreaLightManager::new(archive.clone(), 0, &mut attrs);
        let mut expected = ModelLighting::default();
        archive.template_at(0).apply(&mut expected);
        assert_eq!(attrs, expected, "{path} at boot");
        let mut applied = 0;
        for second in 1..86_400u32 {
            if manager.update(second, &mut attrs) {
                applied += 1;
            }
            assert_eq!(manager.active(), archive.initial_index(second), "{path} at {second}s");
        }
        // One step per record after the boot pick.
        let boot = archive.initial_index(0);
        assert_eq!(applied, archive.templates().len() - 1 - boot, "{path} steps");
        assert_eq!(manager.template(), archive.template_at(86_399), "{path} at 23:59:59");
        // Midnight wrap: a stepping table falls below its last window
        // and steps to record 0; when that record is the zero-length
        // one (area00) a second update moves on to the boot pick. The
        // flat tables never move.
        if archive.templates().len() > 1 {
            assert!(manager.update(0, &mut attrs), "{path} wrap step");
            assert_eq!(manager.active(), 0, "{path} record 0 after the wrap");
            assert_eq!(manager.update(0, &mut attrs), boot != 0, "{path} settle step");
            assert_eq!(manager.active(), boot, "{path} settled");
        } else {
            assert!(!manager.update(0, &mut attrs), "{path} flat");
        }
    }
}

#[test]
fn leftover_narc_members_are_the_same_format() {
    let Some(store) = store() else { return };
    // `data/arealight.narc` is in the cart but no code opens it (the
    // five .txt paths are the whole table). Its four members are the
    // same text format: two are byte-for-byte the shipped area01/area02
    // tables, two are earlier cuts of an area00-style table.
    let bytes = store.nitrofs_file("data/arealight.narc").expect("narc");
    let narc = Narc::parse(&bytes).expect("narc parses");
    let shipped: Vec<Vec<u8>> = archives(&store).iter().map(canonical).collect();
    let mut matches = Vec::new();
    let mut members = 0;
    while let Ok(member) = narc.file(members) {
        let archive = AreaLightArchive::parse(member)
            .unwrap_or_else(|e| panic!("arealight.narc member {members}: {e}"));
        assert!(!archive.templates().is_empty());
        let bytes = canonical(&archive);
        matches.push(shipped.iter().position(|s| *s == bytes));
        members += 1;
    }
    if collecting() {
        eprintln!("arealight.narc members match shipped archives: {matches:?}");
    }
    assert_eq!(members, 4, "arealight.narc member count");
    assert_eq!(matches, [None, Some(1), None, Some(2)], "narc members vs shipped");
}

#[test]
fn bedroom_area_data_names_its_light_archive() {
    let Some(store) = store() else { return };
    // The area-data record (`a/0/4/2`, one member per area bank; the
    // bedroom is bank 25 — `field.rs`) is the 8 bytes
    // `AreaDataManager_Alloc` reads into +0x8B0: prop archive, texture,
    // a u16, the indoor flag at byte 6 and the light type at byte 7.
    let rom_bytes = std::fs::read(ROM_PATH).expect("rom");
    let rom = NdsRom::parse(&rom_bytes).expect("rom parses");
    let narc = Narc::parse(rom.file_by_path("a/0/4/2").expect("area data")).expect("narc");
    let record = narc.file(25).expect("bedroom area data");
    assert_eq!(record.len(), 8, "area-data record size");
    if std::env::var_os("LIGHTING_HG_COLLECT").is_some() {
        eprintln!("bedroom area record: {record:?}");
    }
    let light_type = record[7];
    let id = archive_for_light_type(light_type, false);
    // Whatever the type, the archive it names loads.
    assert!(AreaLightArchive::load(&store, id).is_ok());
    assert_eq!(id, usize::from(light_type == 0), "bedroom light archive");
}

/// A test-local SHA-1 (FIPS 180-1) so the dependency-free core's
/// goldens stay hashes, not bytes.
mod sha1 {
    pub fn hex(data: &[u8]) -> String {
        let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
        let mut msg = data.to_vec();
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
        for block in msg.chunks_exact(64) {
            let mut w = [0u32; 80];
            for (i, word) in block.chunks_exact(4).enumerate() {
                w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
            }
            for i in 16..80 {
                w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
            }
            let [mut a, mut b, mut c, mut d, mut e] = h;
            for (i, &wi) in w.iter().enumerate() {
                let (f, k) = match i / 20 {
                    0 => ((b & c) | (!b & d), 0x5A82_7999),
                    1 => (b ^ c ^ d, 0x6ED9_EBA1),
                    2 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                    _ => (b ^ c ^ d, 0xCA62_C1D6),
                };
                let t = a
                    .rotate_left(5)
                    .wrapping_add(f)
                    .wrapping_add(e)
                    .wrapping_add(k)
                    .wrapping_add(wi);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = t;
            }
            for (h, v) in h.iter_mut().zip([a, b, c, d, e]) {
                *h = h.wrapping_add(v);
            }
        }
        h.iter().map(|w| format!("{w:08x}")).collect()
    }

    #[test]
    fn known_vector() {
        assert_eq!(hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }
}
