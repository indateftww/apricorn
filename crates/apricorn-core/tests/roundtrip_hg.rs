//! Integration tests: re-serialize every supported format in a retail
//! HeartGold (US) dump and byte-compare against the original bytes.
//!
//! Each parser's `to_bytes()` rebuilds the file from the parsed fields
//! (re-deriving everything the writer laid out, re-applying the MAT
//! encryption, and copying the few retained writer artifacts verbatim),
//! so a byte-identical result means the parser understood every byte it
//! read. A parser bug — a field read from the wrong offset, a dropped
//! section, a mis-decoded entry — shows up here as a mismatch even when
//! the parse itself succeeded.
//!
//! SDAT is the one format excluded from byte round-trip: the parsed
//! struct does not retain the writer's SYMB string-pool packing, INFO
//! record interleaving, or block padding (and STRMPLAYER records are
//! not exposed at all), so those bytes cannot be reproduced without
//! re-running the SDK's writer choices. Phase 7 revisits SDAT; see
//! `docs/roundtrip.md` for the full rationale.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM dump). The expected counts are ground truth established
//! by scanning the retail image while writing the parsers — see
//! `docs/conversion.md` (the same census, from the conversion step).

use apricorn_core::formats::btx::is_btx;
use apricorn_core::formats::nanr::is_nanr;
use apricorn_core::formats::ncer::is_ncer;
use apricorn_core::formats::ncgr::is_ncgr;
use apricorn_core::formats::nclr::is_nclr;
use apricorn_core::formats::nscr::is_nscr;
use apricorn_core::formats::{Btx, MsgBank, Nanr, Narc, Ncer, Ncgr, Nclr, Nscr, is_narc};
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail HeartGold (US) round-trip census (see `docs/conversion.md`).
const NCGR_FILES: usize = 7_949; // sniffed-and-parsed; the corrupt DP
// leftover below has a zero BOM, so the magic sniff never reaches it
const NCLR_FILES: usize = 4_953;
const NSCR_FILES: usize = 793;
const NCER_FILES: usize = 612;
const NANR_FILES: usize = 596;
const BTX_FILES: usize = 1_157;
const NARC_FILES: usize = 308;
/// MAT text banks: 829 in the script archive `a/0/2/7`, 624 more in
/// `pbr/msg.narc`.
const MAT_SCRIPT: usize = 829;
const MAT_PBR: usize = 624;
/// The one RGCN-magic file in the image with a corrupt container header
/// (a DP leftover): a zero byte-order mark (so the magic sniff, which
/// checks the BOM too, never reaches it) and file/section sizes each
/// under-declared by 8 bytes. The strict parsers reject it and the
/// round trip skips it; see `docs/conversion.md`.
const CORRUPT_NCGR: &str = "data/dp_areawindow.NCGR";

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
fn round_trips_every_file_byte_exact() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    let mut ncgr = 0usize;
    let mut nclr = 0usize;
    let mut nscr = 0usize;
    let mut ncer = 0usize;
    let mut nanr = 0usize;
    let mut btx = 0usize;
    let mut narc = 0usize;

    // One pass over every file the parse could reach: loose NitroFS files
    // plus every NARC member (matching the format-specific tests' walk).
    for file in rom.nitrofs().files() {
        let bytes = rom.file(file.fat_id).expect("fat id in range");
        let path = &file.path;

        check_formats(
            bytes, path, &mut ncgr, &mut nclr, &mut nscr, &mut ncer, &mut nanr, &mut btx,
        );

        if is_narc(bytes) {
            let parsed =
                Narc::parse(bytes).unwrap_or_else(|e| panic!("NARC {path} failed to parse: {e}"));
            assert_eq!(parsed.to_bytes(), bytes, "{path}: NARC round-trip");
            narc += 1;
            for id in 0..parsed.file_count() {
                let member = parsed.file(id).expect("member id in range");
                check_formats(
                    member, path, &mut ncgr, &mut nclr, &mut nscr, &mut ncer, &mut nanr, &mut btx,
                );
            }
        }
    }

    assert_eq!(ncgr, NCGR_FILES, "NCGR round-trips");

    // The known corrupt leftover: RGCN magic, but a zero BOM (invisible
    // to the sniff) and a container header the strict parser rejects.
    let corrupt = rom
        .file_by_path(CORRUPT_NCGR)
        .expect("corrupt leftover present");
    assert_eq!(&corrupt[0..4], b"RGCN", "the leftover does carry the magic");
    assert!(!is_ncgr(corrupt), "its zero BOM hides it from the sniff");
    assert!(
        Ncgr::parse(corrupt).is_err(),
        "the strict parser rejects it"
    );
    assert_eq!(nclr, NCLR_FILES, "NCLR round-trips");
    assert_eq!(nscr, NSCR_FILES, "NSCR round-trips");
    assert_eq!(ncer, NCER_FILES, "NCER round-trips");
    assert_eq!(nanr, NANR_FILES, "NANR round-trips");
    assert_eq!(btx, BTX_FILES, "BTX round-trips");
    assert_eq!(narc, NARC_FILES, "NARC round-trips");

    // The MAT banks have no magic to sniff — they are the members of two
    // known archives. Re-encryption must reproduce the ciphertext.
    let mut mat = 0usize;
    for (archive, expected) in [("a/0/2/7", MAT_SCRIPT), ("pbr/msg.narc", MAT_PBR)] {
        let parsed = Narc::parse(rom.file_by_path(archive).expect("archive is in the FAT"))
            .unwrap_or_else(|e| panic!("{archive} must be a NARC: {e}"));
        assert_eq!(parsed.file_count(), expected, "{archive} member count");
        for id in 0..parsed.file_count() {
            let member = parsed.file(id).expect("member id in range");
            let bank = MsgBank::parse(member)
                .unwrap_or_else(|e| panic!("{archive} member {id} failed to parse: {e}"));
            assert_eq!(
                bank.to_bytes(),
                member,
                "{archive} member {id}: MAT round-trip"
            );
            mat += 1;
        }
    }
    assert_eq!(mat, MAT_SCRIPT + MAT_PBR, "MAT round-trips");

    eprintln!(
        "round-tripped {ncgr} NCGR + {nclr} NCLR + {nscr} NSCR + {ncer} NCER + \
         {nanr} NANR + {btx} BTX + {narc} NARC + {mat} MAT byte-exact"
    );
}

/// The per-format sniff/parse/re-serialize/compare, shared by loose files
/// and NARC members.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn check_formats(
    bytes: &[u8],
    path: &str,
    ncgr: &mut usize,
    nclr: &mut usize,
    nscr: &mut usize,
    ncer: &mut usize,
    nanr: &mut usize,
    btx: &mut usize,
) {
    if is_ncgr(bytes) {
        let parsed = Ncgr::parse(bytes).unwrap_or_else(|e| panic!("{path} NCGR: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NCGR round-trip");
        *ncgr += 1;
    } else if is_nclr(bytes) {
        let parsed = Nclr::parse(bytes).unwrap_or_else(|e| panic!("{path} NCLR: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NCLR round-trip");
        *nclr += 1;
    } else if is_nscr(bytes) {
        let parsed = Nscr::parse(bytes).unwrap_or_else(|e| panic!("{path} NSCR: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NSCR round-trip");
        *nscr += 1;
    } else if is_ncer(bytes) {
        let parsed = Ncer::parse(bytes).unwrap_or_else(|e| panic!("{path} NCER: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NCER round-trip");
        *ncer += 1;
    } else if is_nanr(bytes) {
        let parsed = Nanr::parse(bytes).unwrap_or_else(|e| panic!("{path} NANR: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NANR round-trip");
        *nanr += 1;
    } else if is_btx(bytes) {
        let parsed = Btx::parse(bytes).unwrap_or_else(|e| panic!("{path} BTX: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: BTX round-trip");
        *btx += 1;
    }
}
