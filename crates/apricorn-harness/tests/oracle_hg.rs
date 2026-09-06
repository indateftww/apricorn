//! Oracle integration tests against the retail HeartGold (US) ROM: the
//! patched melonDS 1.1 build under `out/oracle` boots the real cart and
//! must be deterministic — the whole harness rests on that.
//!
//! These run only when both the ROM (`hg_usa.nds` at the repo root, same
//! policy as `arm_hg.rs`) and the oracle binary (built by
//! `oracle/setup.ps1` into `out/oracle/`, gitignored) are present; CI has
//! neither and skips silently. No committed artifact can make them pass
//! falsely: the oracle self-computes `rom-sha1` and `Trace::parse`
//! validates every line it emits.

use std::path::PathBuf;
use std::process::Command;

use apricorn_harness::diff;
use apricorn_harness::trace::Trace;

const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// sLCRNG_State, the game's boot LCG seed slot (`pins/arm9.tsv`): a tiny
/// 4-byte region that turns out to also be where the RTC first shows —
/// HeartGold seeds this LCG from the clock around frame 185.
const RNG_REGION: (&str, u32, u32, u32) = ("sLCRNG_State", 0x021D_15A8, 4, 1);

/// Enough frames for the game to seed its RNG from the pinned clock
/// (observed at frame ~185 in a DirectBoot retail run).
const FRAMES: u32 = 300;

fn oracle_bin() -> Option<PathBuf> {
    for name in ["apricorn-oracle.exe", "apricorn-oracle"] {
        let path = PathBuf::from(REPO).join("out/oracle").join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// The run's prerequisites, or a skip.
fn setup() -> Option<(PathBuf, PathBuf)> {
    if !PathBuf::from(ROM_PATH).is_file() {
        eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
        return None;
    }
    let Some(oracle) = oracle_bin() else {
        eprintln!("skipping: out/oracle/apricorn-oracle not built (run oracle/setup.ps1)");
        return None;
    };
    let work = std::env::temp_dir().join("apricorn-oracle-test");
    std::fs::create_dir_all(&work).unwrap();
    Some((oracle, work))
}

/// Compiles the regions blob the oracle consumes (little-endian:
/// u32 count, then addr/size/sample/name_len + name per region —
/// docs/oracle.md is the format contract).
fn write_regions_blob(path: &std::path::Path) {
    let (name, addr, size, sample) = RNG_REGION;
    let mut blob = Vec::new();
    blob.extend_from_slice(&1u32.to_le_bytes());
    blob.extend_from_slice(&addr.to_le_bytes());
    blob.extend_from_slice(&size.to_le_bytes());
    blob.extend_from_slice(&sample.to_le_bytes());
    blob.extend_from_slice(&(name.len() as u32).to_le_bytes());
    blob.extend_from_slice(name.as_bytes());
    std::fs::write(path, blob).unwrap();
}

/// One oracle run: the pinned-RTC boot with no input. Returns the trace
/// text, failing the test with the oracle's stderr if it doesn't exit 0.
fn run(oracle: &std::path::Path, work: &std::path::Path, rtc: &str, tag: &str) -> String {
    let regions = work.join(format!("{tag}.regions.bin"));
    let out = work.join(format!("{tag}.trace"));
    write_regions_blob(&regions);

    // Passthrough header values: both traces share them, and the diff
    // gate requires the pair to agree. Well-formed hex, content-free.
    let input_sha = "00".repeat(20);
    let regions_sha = "ff".repeat(20);

    let status = Command::new(oracle)
        .arg("run")
        .arg("--rom")
        .arg(ROM_PATH)
        .arg("--out")
        .arg(&out)
        .arg("--regions")
        .arg(&regions)
        .arg("--frames")
        .arg(FRAMES.to_string())
        .arg("--rtc")
        .arg(rtc)
        .arg("--input-sha1")
        .arg(&input_sha)
        .arg("--regions-sha1")
        .arg(&regions_sha)
        .output()
        .expect("failed to spawn the oracle");
    assert!(
        status.status.success(),
        "oracle exited {:?}\nstderr:\n{}",
        status.status.code(),
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(&out)
        .unwrap_or_else(|e| panic!("oracle wrote no trace at {}: {e}", out.display()))
}

#[test]
fn oracle_boot_is_deterministic() {
    let Some((oracle, work)) = setup() else {
        return;
    };

    let a = run(&oracle, &work, "2010-03-01T09:00:00", "det-a");
    let b = run(&oracle, &work, "2010-03-01T09:00:00", "det-b");

    // Two same-configuration runs must be byte-identical, not just
    // equivalent: any divergence here is the oracle, never the game.
    assert_eq!(a, b, "two identical oracle runs produced different traces");

    // And the output must be well-formed for the Rust side end to end:
    // parse both, compare, expect EQUIVALENT.
    let a = Trace::parse(&a).expect("oracle trace failed to parse");
    let b = Trace::parse(&b).expect("oracle trace failed to parse");
    let report = diff::compare(&a, &b, None).expect("diff gate refused a same-config pair");
    assert_eq!(report.verdict, apricorn_harness::Verdict::Equivalent);
}

#[test]
fn rtc_pin_changes_the_boot() {
    let Some((oracle, work)) = setup() else {
        return;
    };

    let a = run(&oracle, &work, "2010-03-01T09:00:00", "rtc-a");
    let b = run(&oracle, &work, "2011-07-04T12:34:56", "rtc-b");

    // The pinned RTC is an input like any other: a different clock must
    // reach the game (HeartGold seeds its boot LCG from it) and change
    // the trace — otherwise the oracle would be ignoring its own pin.
    let ta = Trace::parse(&a).expect("oracle trace failed to parse");
    let tb = Trace::parse(&b).expect("oracle trace failed to parse");
    assert_ne!(ta.header.rtc, tb.header.rtc, "rtc header not passthrough");
    assert_ne!(a, b, "different pinned RTCs produced identical traces");

    // The divergence must be a real state change, not just the header.
    let report = diff::compare(&ta, &tb, None).expect("diff gate refused a same-config pair");
    assert!(
        matches!(report.verdict, apricorn_harness::Verdict::Diverged { .. }),
        "different RTCs but no state divergence: {report:?}"
    );
}
