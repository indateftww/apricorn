//! The engine-side producer (`apricorn_harness::engine`) replayed
//! through the corpus machinery — the Phase 2 exit promise ("the real
//! apricorn-core replays through the same corpus machinery"),
//! redeemed and measured against `corpus/boot-idle`'s committed oracle
//! baseline.
//!
//! Three kinds of check:
//!
//! * **ROM-less cross-producer locks** — the engine's region
//!   serializations hash to the very digests the oracle recorded for
//!   the same values (the SHA-1s below are copied from
//!   `corpus/boot-idle/expected.trace`; they are hashes of RNG state,
//!   not ROM content). If the MT layout or the LCG byte order ever
//!   drifted, these would move first.
//! * **ROM-gated engine runs** — header gates identical to the case's,
//!   the same sampling schedule, byte-identical traces across two runs,
//!   the `frames` override.
//! * **The boot-idle verdict** — pinned exactly: the engine and the
//!   oracle diverge at frame 0 on `sLCRNG_State`, and the reason is
//!   measured, not guessed (see `docs/engine-runner.md`):
//!
//!   The oracle's frame axis starts at power-on; the retail
//!   `InitializeMainRNG` (`src/main.c:90`) lands at its VBlank 185, so
//!   its `sLCRNG_State` is zeroed bss for frames 0–184. The engine
//!   seeds at construction (`Game::new`), so its frame 0 already holds
//!   the seed. The seed *value* agrees to the bit — the oracle's hash
//!   at frames 185–242 is the engine's hash at frame 0 (`0x0309000A`,
//!   `RngSeedFromRTC()` with the vblank counter at 0) — but the
//!   oracle then soft-resets every 222 frames: its idle key mask is
//!   passed straight into melonDS's active-low `KeyInput`
//!   (`apricorn-oracle.cpp`, `SetKeyMask`), holding all twelve
//!   buttons, and L+R+START+SELECT is `NitroMain`'s reset combo. The
//!   committed baseline is that loop, never the intro. EQUIVALENT is
//!   therefore not reachable by any seeding change on the engine side;
//!   the test asserts the exact first divergence instead.
//!
//! Runs the ROM-gated parts only when `hg_usa.nds` sits at the repo
//! root (CI has no ROM and skips silently); the oracle binary is never
//! needed — the committed `expected.trace` is the oracle's side.

use std::path::Path;

use apricorn_core::rng::{Lcrng, Mt19937};
use apricorn_harness::Verdict;
use apricorn_harness::diff;
use apricorn_harness::engine::{
    self, EngineRun, FRAME_RATE, PRODUCER, lcrng_bytes, mtrng_cycles_bytes, mtrng_state_bytes,
};
use apricorn_harness::input::InputScript;
use apricorn_harness::regions::RegionSet;
use apricorn_harness::replay::Case;
use apricorn_harness::trace::{Hash, Trace, TraceRecord};

const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// The boot-idle pin: `rtc 2010-03-01T09:00:00`, and the seed
/// `RngSeedFromRTC()` derives from it with the vblank counter at 0 —
/// `10 + 3*0x100*1*0x10000 + 9*0x10000`.
const BOOT_SEED: u32 = 0x0309_000A;

// SHA-1s copied from corpus/boot-idle/expected.trace (the oracle's
// memory reads), by the frames they cover there.
/// `sLCRNG_State` frames 0–184: four zero bytes (bss before the seed).
const LC_ZERO: &str = "9069ca78e7450a285173431b3e52c5c25299e473";
/// `sLCRNG_State` frames 185–242: the boot seed, no draw.
const LC_BOOT_SEED: &str = "d1ce3a40a0b678ee610b5940e0d0610a0a845ea2";
/// `sMTRNG_State` frames 0–149: 2496 zero bytes (bss).
const MT_ZERO: &str = "6c7a78338c23aa64b5dee3c4d8f23f6fca04660b";
/// `sMTRNG_State` frames 150–209: the fresh image's first draw
/// (`sMTRNG_Cycles` 625 → `SetMTRNGSeed(5489)` → one twist).
const MT_FRESH_DRAW: &str = "2c35181dd3841939991bf8fb60a46f0791bfb011";
/// `sMTRNG_State` frames 210–269: `SetMTRNGSeed(BOOT_SEED)`.
const MT_BOOT_SEED: &str = "b29c66574f977a9b09e0038301c4b74b38c31700";
/// `sMTRNG_Cycles` values the oracle's every-frame run showed: 625 on
/// a fresh image, 2 after the two roamer draws, 624 after the seed.
const CYCLES_625: &str = "d9fdbb9a59d3583db32b8c4df493d7630def55bb";
const CYCLES_2: &str = "0aaf76f425c6e0f43a36197de768e67d9e035abb";
const CYCLES_624: &str = "b8e3a39c401882843736b6033d904e78cb5e263e";

fn hex(hash: &Hash) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_hex(bytes: &[u8]) -> String {
    hex(&Trace::hash_region(bytes))
}

fn boot_idle() -> Option<Case> {
    if !Path::new(ROM_PATH).is_file() {
        eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
        return None;
    }
    let dir = Path::new(REPO).join("corpus/boot-idle");
    Some(Case::load(&dir).unwrap_or_else(|e| panic!("boot-idle failed to load: {e}")))
}

fn committed(case: &Case) -> Trace {
    case.expected()
        .unwrap_or_else(|e| panic!("expected.trace: {e}"))
        .expect("boot-idle has a committed expected.trace")
}

/// The record's hash when it is the sample named.
fn sample_hash(trace: &Trace, frame: u32, region: &str) -> Option<String> {
    trace.records.iter().find_map(|r| match r {
        TraceRecord::Sample {
            frame: f,
            region: name,
            hash,
        } if *f == frame && name == region => Some(hex(hash)),
        _ => None,
    })
}

// ===== ROM-less: the engine's bytes hash like the oracle's memory ====

#[test]
fn lcrng_region_hashes_like_the_oracle_memory() {
    assert_eq!(hash_hex(&lcrng_bytes(&Lcrng::new(0))), LC_ZERO);
    assert_eq!(hash_hex(&lcrng_bytes(&Lcrng::new(BOOT_SEED))), LC_BOOT_SEED);
}

#[test]
fn mtrng_region_hashes_like_the_oracle_memory() {
    // The bss image: all zero words (the pins table's zero prologue
    // covers only its first 16 bytes; this is the whole region).
    let fresh = Mt19937::uninitialized();
    assert_eq!(hash_hex(&mtrng_state_bytes(&fresh)), MT_ZERO);
    assert_eq!(hash_hex(&mtrng_cycles_bytes(&fresh)), CYCLES_625);

    // The two roamer draws SaveData_New makes on a blank card, before
    // InitializeMainRNG: the 625 sentinel reseeds 5489 and twists,
    // the cursor lands on 2.
    let mut drawn = Mt19937::uninitialized();
    drawn.next_u32();
    drawn.next_u32();
    assert_eq!(hash_hex(&mtrng_state_bytes(&drawn)), MT_FRESH_DRAW);
    assert_eq!(hash_hex(&mtrng_cycles_bytes(&drawn)), CYCLES_2);

    // InitializeMainRNG's SetMTRNGSeed(seed): the cursor at 624.
    let seeded = Mt19937::new(BOOT_SEED);
    assert_eq!(hash_hex(&mtrng_state_bytes(&seeded)), MT_BOOT_SEED);
    assert_eq!(hash_hex(&mtrng_cycles_bytes(&seeded)), CYCLES_624);
}

// ===== ROM-gated: the engine run itself =============================

#[test]
fn engine_trace_header_gates_match_the_case() {
    let Some(case) = boot_idle() else { return };
    let expected = committed(&case);
    let actual = case
        .run_engine(Path::new(ROM_PATH))
        .unwrap_or_else(|e| panic!("engine run failed: {e}"));

    // The producer names the side; every gate field is the oracle's.
    assert_eq!(actual.header.producer, PRODUCER);
    assert_eq!(actual.header.rom_sha1, expected.header.rom_sha1, "rom-sha1");
    assert_eq!(
        actual.header.input_sha1, expected.header.input_sha1,
        "input-sha1"
    );
    assert_eq!(
        actual.header.regions_sha1, expected.header.regions_sha1,
        "regions-sha1"
    );
    assert_eq!(actual.header.frames, expected.header.frames);
    assert_eq!(actual.header.frame_rate, expected.header.frame_rate);
    assert_eq!(actual.header.frame_rate, FRAME_RATE);
    assert_eq!(actual.header.rtc, expected.header.rtc);

    // The sampling schedule is identical: the same (frame, region)
    // sequence, record for record — only hashes can differ.
    assert_eq!(actual.records.len(), expected.records.len());
    for (a, e) in actual.records.iter().zip(&expected.records) {
        match (a, e) {
            (
                TraceRecord::Sample {
                    frame: fa,
                    region: ra,
                    ..
                },
                TraceRecord::Sample {
                    frame: fe,
                    region: re,
                    ..
                },
            ) => assert_eq!((fa, ra), (fe, re)),
            _ => panic!("boot-idle carries frame samples only"),
        }
    }
}

#[test]
fn engine_trace_is_deterministic() {
    let Some(case) = boot_idle() else { return };
    let rom = Path::new(ROM_PATH);
    let first = case.run_engine(rom).expect("first run");
    let second = case.run_engine(rom).expect("second run");
    assert_eq!(first, second);
    // And the text form round-trips through the parser unchanged.
    let text = first.to_string();
    assert_eq!(Trace::parse(&text).expect("engine trace parses"), first);
    assert_eq!(text.lines().next(), Some("TRACE apricorn 1"));
}

#[test]
fn frames_override_runs_past_the_script_with_idle_input() {
    let Some(case) = boot_idle() else { return };
    let regions = RegionSet::parse(
        "sLCRNG_State hard 0x021D15A8 4 1\nsMTRNG_State hard 0x021D15AC 2496 30\nsMTRNG_Cycles hard 0x0210F6CC 4 7\n",
    )
    .unwrap();
    let script = InputScript::parse("rtc 2010-03-01T09:00:00\nend 10\n").unwrap();
    let output = EngineRun {
        rom: Path::new(ROM_PATH),
        regions: &regions,
        script: &script,
        save: None,
        frames: Some(31),
        producer: Some("engine-test"),
    }
    .run()
    .expect("engine run");
    assert_eq!(output.trace.header.producer, "engine-test");
    assert_eq!(output.trace.header.frames, 31);
    // 31 LCRNG samples, MT at 0 and 30, cycles at 0/7/14/21/28.
    assert_eq!(output.trace.records.len(), 31 + 2 + 5);
    assert_eq!(output.rtc, engine::parse_rtc("2010-03-01T09:00:00").unwrap());
    // The game is still in the intro after 31 ticks; the boot seed
    // is untouched (nothing in the copyright beat draws).
    assert_eq!(output.game.lcrng().seed(), BOOT_SEED);
    let _ = case;
}

#[test]
fn boot_idle_engine_vs_oracle_diverges_at_frame_0_on_the_boot_seed() {
    let Some(case) = boot_idle() else { return };
    let expected = committed(&case);

    // Guard the pin: this assertion is about the *committed* baseline
    // — the oracle's soft-reset loop (zeroed bss through frame 184,
    // the boot seed from 185). A regenerated baseline (an oracle whose
    // idle key mask no longer holds every button) must revisit the
    // verdict below rather than inherit it.
    assert_eq!(
        sample_hash(&expected, 0, "sLCRNG_State").as_deref(),
        Some(LC_ZERO),
        "the committed baseline changed: re-measure the boot-idle verdict"
    );
    assert_eq!(
        sample_hash(&expected, 185, "sLCRNG_State").as_deref(),
        Some(LC_BOOT_SEED),
        "the committed baseline changed: re-measure the boot-idle verdict"
    );
    assert_eq!(
        sample_hash(&expected, 210, "sMTRNG_State").as_deref(),
        Some(MT_BOOT_SEED)
    );

    let actual = case
        .run_engine(Path::new(ROM_PATH))
        .unwrap_or_else(|e| panic!("engine run failed: {e}"));
    let report = diff::compare(&expected, &actual, Some(&case.regions))
        .unwrap_or_else(|e| panic!("the gate refused the pair: {e}"));

    // The exact first divergence: frame 0, the LCG slot — the oracle
    // still holds zeroed bss (InitializeMainRNG is 185 VBlanks away),
    // the engine already holds the seed.
    assert_eq!(report.verdict, Verdict::Diverged { frame: 0 }, "{report}");
    let first = report.first.clone().expect("a divergence names itself");
    assert_eq!(first.what, "sLCRNG_State");
    assert!(first.detail.contains(LC_ZERO), "{}", first.detail);
    assert!(first.detail.contains(LC_BOOT_SEED), "{}", first.detail);
    assert!(report.drift.is_empty(), "both regions are hard");

    // The seed itself is the ROM's: the engine's frame-0 samples are
    // the oracle's frames 185 (LCG) and 210 (MT) — same value,
    // different frame axis.
    assert_eq!(
        sample_hash(&actual, 0, "sLCRNG_State").as_deref(),
        Some(LC_BOOT_SEED)
    );
    assert_eq!(
        sample_hash(&actual, 0, "sMTRNG_State").as_deref(),
        Some(MT_BOOT_SEED)
    );
    assert_eq!(actual.header.frames, 600);
}
