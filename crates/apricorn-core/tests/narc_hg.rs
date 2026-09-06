//! Integration tests: every NARC in a retail HeartGold (US) dump.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM). The expected counts are ground truth established by
//! scanning the retail image while writing the parser — see
//! `docs/narc.md`.

use apricorn_core::formats::{Narc, is_narc};
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail HeartGold (US) NARC facts.
const NARC_COUNT: usize = 308;
const TOTAL_MEMBERS: usize = 56_689;
const ZERO_LENGTH_MEMBERS: usize = 699;
const NCGR_MEMBERS: usize = 7_937; // "RGCN"
const NCLR_MEMBERS: usize = 4_945; // "RLCN"
const NSBMD_MEMBERS: usize = 1_540; // "BMD0"

fn load_rom() -> Option<Vec<u8>> {
    match std::fs::read(ROM_PATH) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

/// Parses every NARC in the ROM and returns them with their NitroFS
/// paths. Any parse failure fails the test: all 308 retail archives
/// must parse. (The `Narc`s borrow `data`, like the `NdsRom` does.)
fn all_narcs<'a>(_data: &'a [u8], rom: &NdsRom<'a>) -> Vec<(String, Narc<'a>)> {
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
fn parses_every_retail_narc() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let narcs = all_narcs(&data, &rom);

    assert_eq!(narcs.len(), NARC_COUNT, "NARC count changed?");
    let members: usize = narcs.iter().map(|(_, n)| n.file_count()).sum();
    assert_eq!(members, TOTAL_MEMBERS, "total member count changed?");
    eprintln!("parsed {} NARCs, {TOTAL_MEMBERS} members", narcs.len());

    // A census of member formats: most members are themselves Nitro
    // files whose magic is the reversed format name. Pinning the counts
    // guards both the parser and the ROM identity.
    let mut magic_counts: std::collections::HashMap<&[u8], usize> =
        std::collections::HashMap::new();
    let mut zero_length = 0usize;
    for (_, narc) in &narcs {
        for id in 0..narc.file_count() {
            let bytes = narc.file(id).expect("member id in range");
            if bytes.is_empty() {
                zero_length += 1;
            } else if bytes.len() >= 4 {
                *magic_counts.entry(&bytes[0..4]).or_default() += 1;
            }
        }
    }
    assert_eq!(zero_length, ZERO_LENGTH_MEMBERS);
    assert_eq!(
        magic_counts.get(b"RGCN".as_slice()),
        Some(&NCGR_MEMBERS),
        "NCGR member count"
    );
    assert_eq!(
        magic_counts.get(b"RLCN".as_slice()),
        Some(&NCLR_MEMBERS),
        "NCLR member count"
    );
    assert_eq!(
        magic_counts.get(b"BMD0".as_slice()),
        Some(&NSBMD_MEMBERS),
        "NSBMD member count"
    );

    // Retail archives are index-addressed: no BTNF ever names a member.
    for (path, narc) in &narcs {
        for id in 0..narc.file_count() {
            assert!(narc.name(id).is_none(), "{path} names member {id}");
        }
    }
}

#[test]
fn spot_checks_known_archives() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let narcs = all_narcs(&data, &rom);
    let by_path = |path: &str| -> &Narc<'_> {
        narcs
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, n)| n)
            .unwrap_or_else(|| panic!("expected {path} to be a NARC in the NitroFS"))
    };

    // Growth tables: 8 binary members (level-up moves etc.).
    assert_eq!(by_path("pbr/growtbl.narc").file_count(), 8);

    // The message bank: 624 members, odd-sized and packed *without*
    // 4-byte alignment — the parser must not assume contiguity.
    let msg = by_path("pbr/msg.narc");
    assert_eq!(msg.file_count(), 624);

    // Field building models: member 0 is an NSBMD ("BMD0" little-endian).
    let bm_field = by_path("fielddata/build_model/bm_field.narc");
    assert_eq!(&bm_field.file(0).unwrap()[0..4], b"BMD0");

    // The smallest archive in the ROM: one 1-byte member holding 0x00
    // (a root-only empty BTNF, the offset-4 convention).
    let tiny = narcs
        .iter()
        .find(|(_, n)| n.file_count() == 1 && n.file(0).unwrap() == [0x00])
        .map(|(p, _)| p.clone())
        .expect("the 1-byte single-member NARC must exist");
    eprintln!("smallest NARC: {tiny}");
}
