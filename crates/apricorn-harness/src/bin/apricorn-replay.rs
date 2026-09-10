//! `apricorn-replay` — run a corpus case on the oracle (or the
//! engine), diff against the committed expected trace.
//!
//! ```text
//! usage: apricorn-replay [--update] [--engine] [--rom ROM] <case-dir>
//! ```
//!
//! Loads the case (`regions.conf` + `input.apin` + optional
//! `probes.conf`), replays it on the oracle, and compares the produced
//! trace against the case's `expected.trace` with the case's own
//! buckets. Exit code 0 = EQUIVALENT, 1 = diverged, 64 = usage or gate
//! error — the same conventions as `apricorn-diff`.
//!
//! `--engine` replays the case on the real `apricorn-core` game
//! instead (`apricorn_harness::engine`) and compares *that* trace
//! against the oracle's `expected.trace` — the engine-vs-oracle
//! verdict, printed as EQUIVALENT or the first divergence's frame,
//! region, and both hashes.
//!
//! `--update` regenerates `expected.trace` from an oracle run instead
//! of comparing: a deliberate, reviewed act (the corpus's definition
//! of correct must never change implicitly) — and an oracle-only one:
//! `--update --engine` is refused, the engine never defines correct.
//! `--rom` overrides the default dump (`hg_usa.nds` at the repo
//! root); the ROM is a local, gitignored file.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apricorn_harness::diff;
use apricorn_harness::replay::Case;

const USAGE: &str = "usage: apricorn-replay [--update] [--engine] [--rom ROM] <case-dir>";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    // Flags may come before or after the case dir (the wrapper
    // scripts append theirs); exactly one positional is the case.
    let mut update = false;
    let mut engine = false;
    let mut rom: Option<PathBuf> = None;
    let mut case_dir: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--update" => update = true,
            "--engine" => engine = true,
            "--rom" => {
                let Some(path) = args.next() else {
                    eprintln!("{USAGE}\n  --rom needs a path");
                    return ExitCode::from(64);
                };
                rom = Some(PathBuf::from(path));
            }
            _ if arg.starts_with("--") => {
                eprintln!("{USAGE}\n  unknown flag {arg}");
                return ExitCode::from(64);
            }
            _ if case_dir.is_none() => case_dir = Some(arg),
            _ => {
                eprintln!("{USAGE}\n  exactly one case dir");
                return ExitCode::from(64);
            }
        }
    }
    let Some(case_dir) = case_dir else {
        eprintln!("{USAGE}");
        return ExitCode::from(64);
    };
    if update && engine {
        eprintln!(
            "{USAGE}\n  --update regenerates the oracle baseline; the engine never defines correct (drop --engine)"
        );
        return ExitCode::from(64);
    }

    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rom = rom.unwrap_or_else(|| repo.join("hg_usa.nds"));

    match run(&case_dir, &rom, update, engine) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("apricorn-replay: {message}");
            ExitCode::from(64)
        }
    }
}

fn run(case_dir: &str, rom: &Path, update: bool, engine: bool) -> Result<ExitCode, String> {
    let case = Case::load(Path::new(case_dir)).map_err(|e| e.to_string())?;

    let actual = if engine {
        case.run_engine(rom).map_err(|e| e.to_string())?
    } else {
        case.run_oracle(rom).map_err(|e| e.to_string())?
    };

    if update {
        case.write_expected(&actual).map_err(|e| e.to_string())?;
        println!(
            "updated {} ({} records)",
            case.dir.join("expected.trace").display(),
            actual.records.len()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let expected = case.expected().map_err(|e| e.to_string())?;
    let Some(expected) = expected else {
        return Err(format!(
            "case {} has no expected.trace (run with --update to create it)",
            case.dir.display()
        ));
    };

    println!(
        "comparing {} against {} ({} frames)",
        actual.header.producer, expected.header.producer, expected.header.frames
    );
    let report =
        diff::compare(&expected, &actual, Some(&case.regions)).map_err(|e| e.to_string())?;
    println!("{report}");
    Ok(match report.verdict {
        apricorn_harness::Verdict::Equivalent => ExitCode::SUCCESS,
        apricorn_harness::Verdict::Diverged { .. } => ExitCode::FAILURE,
    })
}
