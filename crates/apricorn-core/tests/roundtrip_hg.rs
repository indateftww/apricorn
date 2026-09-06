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
use apricorn_core::nds::{NdsRom, lz10};

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
/// The same census for the LZ77-10-compressed population: members that
/// decode to an image of one of the five formats. No NCLR ever ships
/// compressed.
const LZ_NCGR_FILES: usize = 1_509;
const LZ_NCLR_FILES: usize = 0;
const LZ_NSCR_FILES: usize = 707;
const LZ_NCER_FILES: usize = 367;
const LZ_NANR_FILES: usize = 367;
/// LZ77-10 images whose magic sniffs but whose parse fails — retail
/// inconsistency the game tolerates (NNS loaders never validate the
/// container), skipped and pinned here: the 23 `a/0/0/7` NCGRs whose
/// image drops the trailing CPOS section the container header still
/// lists. Raw files whose magic sniffs must parse (or dp_areawindow
/// would have shipped); only decoded images get this tolerance.
const LZ_RETAIL_SKIPPED: usize = 23;
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

/// Round-trip counters, one field per pinned population.
#[derive(Default)]
struct Census {
    ncgr: usize,
    nclr: usize,
    nscr: usize,
    ncer: usize,
    nanr: usize,
    btx: usize,
    narc: usize,
    lz_ncgr: usize,
    lz_nclr: usize,
    lz_nscr: usize,
    lz_ncer: usize,
    lz_nanr: usize,
    lz_skipped: usize,
}

#[test]
fn round_trips_every_file_byte_exact() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    let mut census = Census::default();

    // One pass over every file the parse could reach: loose NitroFS files
    // plus every NARC member (matching the format-specific tests' walk).
    for file in rom.nitrofs().files() {
        let bytes = rom.file(file.fat_id).expect("fat id in range");
        let path = &file.path;

        check_formats(bytes, path, &mut census);

        if is_narc(bytes) {
            let parsed =
                Narc::parse(bytes).unwrap_or_else(|e| panic!("NARC {path} failed to parse: {e}"));
            assert_eq!(parsed.to_bytes(), bytes, "{path}: NARC round-trip");
            census.narc += 1;
            for id in 0..parsed.file_count() {
                let member = parsed.file(id).expect("member id in range");
                check_formats(member, path, &mut census);
            }
        }
    }

    assert_eq!(census.ncgr, NCGR_FILES, "NCGR round-trips");

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
    assert_eq!(census.nclr, NCLR_FILES, "NCLR round-trips");
    assert_eq!(census.nscr, NSCR_FILES, "NSCR round-trips");
    assert_eq!(census.ncer, NCER_FILES, "NCER round-trips");
    assert_eq!(census.nanr, NANR_FILES, "NANR round-trips");
    assert_eq!(census.btx, BTX_FILES, "BTX round-trips");
    assert_eq!(census.narc, NARC_FILES, "NARC round-trips");

    // The LZ77-10 images behind the 0x10 magic.
    assert_eq!(census.lz_ncgr, LZ_NCGR_FILES, "LZ NCGR round-trips");
    assert_eq!(census.lz_nclr, LZ_NCLR_FILES, "no NCLR ships compressed");
    assert_eq!(census.lz_nscr, LZ_NSCR_FILES, "LZ NSCR round-trips");
    assert_eq!(census.lz_ncer, LZ_NCER_FILES, "LZ NCER round-trips");
    assert_eq!(census.lz_nanr, LZ_NANR_FILES, "LZ NANR round-trips");
    assert_eq!(
        census.lz_skipped, LZ_RETAIL_SKIPPED,
        "retail-inconsistent LZ images skipped"
    );

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

    let Census {
        ncgr,
        nclr,
        nscr,
        ncer,
        nanr,
        btx,
        narc,
        lz_ncgr,
        lz_nscr,
        lz_ncer,
        lz_nanr,
        lz_skipped,
        ..
    } = census;
    eprintln!(
        "round-tripped {ncgr} NCGR + {nclr} NCLR + {nscr} NSCR + {ncer} NCER + \
         {nanr} NANR + {btx} BTX + {narc} NARC + {mat} MAT byte-exact, plus LZ \
         images {lz_ncgr} NCGR + {lz_nscr} NSCR + {lz_ncer} NCER + {lz_nanr} NANR \
         ({lz_skipped} retail-inconsistent, skipped)"
    );
}

/// The per-format sniff/parse/re-serialize/compare, shared by loose files
/// and NARC members.
#[allow(clippy::too_many_lines)]
fn check_formats(bytes: &[u8], path: &str, census: &mut Census) {
    if is_ncgr(bytes) {
        let parsed = Ncgr::parse(bytes).unwrap_or_else(|e| panic!("{path} NCGR: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NCGR round-trip");
        census.ncgr += 1;
    } else if is_nclr(bytes) {
        let parsed = Nclr::parse(bytes).unwrap_or_else(|e| panic!("{path} NCLR: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NCLR round-trip");
        census.nclr += 1;
    } else if is_nscr(bytes) {
        let parsed = Nscr::parse(bytes).unwrap_or_else(|e| panic!("{path} NSCR: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NSCR round-trip");
        census.nscr += 1;
    } else if is_ncer(bytes) {
        let parsed = Ncer::parse(bytes).unwrap_or_else(|e| panic!("{path} NCER: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NCER round-trip");
        census.ncer += 1;
    } else if is_nanr(bytes) {
        let parsed = Nanr::parse(bytes).unwrap_or_else(|e| panic!("{path} NANR: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: NANR round-trip");
        census.nanr += 1;
    } else if is_btx(bytes) {
        let parsed = Btx::parse(bytes).unwrap_or_else(|e| panic!("{path} BTX: {e}"));
        assert_eq!(parsed.to_bytes(), bytes, "{path}: BTX round-trip");
        census.btx += 1;
    } else if lz10::is_lz10(bytes) {
        // A 0x10 first byte is also the start of many other binaries
        // (and of a MAT bank whose message count is 0x0010 — the MAT
        // census above covers those); only a decodable stream leads
        // anywhere, and one nesting level is all Nitro uses.
        if let Ok(image) = lz10::decompress(bytes) {
            check_image(&image, path, census);
        }
    }
}

/// The same sniff ladder for an LZ77-10-decoded image, one nesting level
/// down. A sniff hit that fails its parse is the known retail
/// inconsistency the census pins — the game's loaders never validate
/// the container — so it counts as a skip instead of panicking; a raw
/// file whose magic sniffs must parse (check_formats above panics).
#[allow(clippy::too_many_lines)]
fn check_image(image: &[u8], path: &str, census: &mut Census) {
    // A sniff hit whose parse fails is the retail inconsistency the
    // census pins (see LZ_RETAIL_SKIPPED): the game tolerates a container
    // whose image omits bytes its header counts, so the strict parser
    // skips the file rather than failing.
    if is_ncgr(image) {
        match Ncgr::parse(image) {
            Ok(p) => {
                assert_eq!(p.to_bytes(), image, "{path}: LZ NCGR round-trip");
                census.lz_ncgr += 1;
            }
            Err(_) => census.lz_skipped += 1,
        }
    } else if is_nclr(image) {
        match Nclr::parse(image) {
            Ok(p) => {
                assert_eq!(p.to_bytes(), image, "{path}: LZ NCLR round-trip");
                census.lz_nclr += 1;
            }
            Err(_) => census.lz_skipped += 1,
        }
    } else if is_nscr(image) {
        match Nscr::parse(image) {
            Ok(p) => {
                assert_eq!(p.to_bytes(), image, "{path}: LZ NSCR round-trip");
                census.lz_nscr += 1;
            }
            Err(_) => census.lz_skipped += 1,
        }
    } else if is_ncer(image) {
        match Ncer::parse(image) {
            Ok(p) => {
                assert_eq!(p.to_bytes(), image, "{path}: LZ NCER round-trip");
                census.lz_ncer += 1;
            }
            Err(_) => census.lz_skipped += 1,
        }
    } else if is_nanr(image) {
        match Nanr::parse(image) {
            Ok(p) => {
                assert_eq!(p.to_bytes(), image, "{path}: LZ NANR round-trip");
                census.lz_nanr += 1;
            }
            Err(_) => census.lz_skipped += 1,
        }
    }
    // Anything else is some other binary's compressed image; not our
    // census.
}
