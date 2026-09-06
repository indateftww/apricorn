//! `apricorn-diff` — compare two traces, print the verdict.
//!
//! ```text
//! usage: apricorn-diff [--regions REGIONS_CONF] <expected.trace> <actual.trace>
//! ```
//!
//! Compares the traces in lockstep and prints the report: drift
//! mismatches first (non-fatal), then the first divergence and verdict.
//! Exit code 0 = EQUIVALENT, 1 = diverged, 64 = usage or gate error —
//! the same conventions as `apricorn-tools`.
//!
//! `--regions` supplies the hard/drift buckets; without it every
//! mismatch is hard. With it, a region the trace references but the
//! config does not know is a gate error.

use std::process::ExitCode;

use apricorn_harness::diff::compare;
use apricorn_harness::regions::RegionSet;
use apricorn_harness::trace::Trace;

const USAGE: &str = "usage: apricorn-diff [--regions REGIONS_CONF] <expected.trace> <actual.trace>";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();

    // Optional --regions, then exactly two trace paths.
    let mut regions_path = None;
    while args.peek().is_some_and(|a| a == "--regions") {
        args.next();
        let Some(path) = args.next() else {
            eprintln!("{USAGE}\n  --regions needs a path");
            return ExitCode::from(64);
        };
        regions_path = Some(path);
    }
    let paths: Vec<String> = args.collect();
    if paths.len() != 2 {
        eprintln!("{USAGE}");
        return ExitCode::from(64);
    }

    let read = |path: &str| std::fs::read_to_string(path).map_err(|e| e.to_string());
    let run = || -> Result<ExitCode, String> {
        let expected_text = read(&paths[0])?;
        let actual_text = read(&paths[1])?;
        let expected = Trace::parse(&expected_text).map_err(|e| e.to_string())?;
        let actual = Trace::parse(&actual_text).map_err(|e| e.to_string())?;
        let regions = match &regions_path {
            Some(path) => Some(RegionSet::parse(&read(path)?).map_err(|e| e.to_string())?),
            None => None,
        };
        let report = compare(&expected, &actual, regions.as_ref()).map_err(|e| e.to_string())?;
        println!("{report}");
        Ok(match report.verdict {
            apricorn_harness::Verdict::Equivalent => ExitCode::SUCCESS,
            apricorn_harness::Verdict::Diverged { .. } => ExitCode::FAILURE,
        })
    };
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("apricorn-diff: {message}");
            ExitCode::from(64)
        }
    }
}
