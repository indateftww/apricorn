//! Corpus regression tests: every committed case under `corpus/` must
//! still replay EQUIVALENT against its committed `expected.trace`.
//!
//! This is the Phase 2 exit criterion as a test: the oracle re-runs the
//! case and the strict comparator walks both traces. Runs only when
//! the ROM (`hg_usa.nds`) and the oracle binary are present; CI has
//! neither and skips silently.
//!
//! The committed expected traces are regenerated only by a deliberate,
//! reviewed `apricorn-replay --update` — this test never writes them.

use std::path::Path;

use apricorn_harness::Verdict;
use apricorn_harness::diff;
use apricorn_harness::replay::Case;

const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

fn setup() -> Option<()> {
    if !Path::new(ROM_PATH).is_file() {
        eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
        return None;
    }
    if apricorn_harness::oracle::find_binary().is_none() {
        eprintln!("skipping: out/oracle/apricorn-oracle not built (run oracle/setup.ps1)");
        return None;
    }
    Some(())
}

/// One case must replay EQUIVALENT against its committed expected trace.
fn replay_case(name: &str) {
    let dir = Path::new(REPO).join("corpus").join(name);
    let case = Case::load(&dir).unwrap_or_else(|e| panic!("case {name} failed to load: {e}"));
    let Some(expected) = case
        .expected()
        .unwrap_or_else(|e| panic!("case {name}: expected.trace: {e}"))
    else {
        panic!("case {name} has no committed expected.trace");
    };

    let actual = case
        .run_oracle(Path::new(ROM_PATH))
        .unwrap_or_else(|e| panic!("case {name}: oracle run failed: {e}"));

    let report = diff::compare(&expected, &actual, Some(&case.regions))
        .unwrap_or_else(|e| panic!("case {name}: diff gate refused the pair: {e}"));
    assert_eq!(
        report.verdict,
        Verdict::Equivalent,
        "case {name} diverged: {report}"
    );
}

#[test]
fn corpus_boot_idle_replays_equivalent() {
    let Some(()) = setup() else { return };
    replay_case("boot-idle");
}
