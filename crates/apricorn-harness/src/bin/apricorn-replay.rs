//! `apricorn-replay` — run a corpus case on the oracle, diff against
//! the committed expected trace.
//!
//! ```text
//! usage: apricorn-replay [--update] [--rom ROM]
//!                        [--shots FRAMES --shots-dir DIR] <case-dir>
//! ```
//!
//! Loads the case (`regions.conf` + `input.apin` + optional
//! `probes.conf`), replays it on the oracle, and compares the produced
//! trace against the case's `expected.trace` with the case's own
//! buckets. Exit code 0 = EQUIVALENT, 1 = diverged, 64 = usage or gate
//! error — the same conventions as `apricorn-diff`.
//!
//! `--update` regenerates `expected.trace` from this run instead of
//! comparing: a deliberate, reviewed act (the corpus's definition of
//! correct must never change implicitly). `--rom` overrides the
//! default dump (`hg_usa.nds` at the repo root); the ROM is a local,
//! gitignored file.
//!
//! `--shots FRAMES --shots-dir DIR` additionally writes both LCDs as
//! PNGs at the end of the listed frames (`120,300-305`: items or
//! inclusive ranges) into `DIR` — `frame_NNNNNN_top.png` /
//! `_bottom.png` — without changing the trace or the verdict. The
//! PNGs are review artifacts (ground truth for engine renders); keep
//! them under `out/`, never in the corpus.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apricorn_harness::diff;
use apricorn_harness::oracle::{ShotRequest, parse_shot_frames};
use apricorn_harness::replay::Case;

const USAGE: &str = "usage: apricorn-replay [--update] [--rom ROM] [--shots FRAMES --shots-dir DIR] <case-dir>";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();

    let mut update = false;
    let mut rom: Option<PathBuf> = None;
    let mut shot_frames: Option<String> = None;
    let mut shots_dir: Option<PathBuf> = None;
    while let Some(arg) = args.peek().cloned() {
        match arg.as_str() {
            "--update" => {
                update = true;
                args.next();
            }
            "--rom" => {
                args.next();
                let Some(path) = args.next() else {
                    eprintln!("{USAGE}\n  --rom needs a path");
                    return ExitCode::from(64);
                };
                rom = Some(PathBuf::from(path));
            }
            "--shots" => {
                args.next();
                let Some(frames) = args.next() else {
                    eprintln!("{USAGE}\n  --shots needs a frame list (120,300-305)");
                    return ExitCode::from(64);
                };
                shot_frames = Some(frames);
            }
            "--shots-dir" => {
                args.next();
                let Some(dir) = args.next() else {
                    eprintln!("{USAGE}\n  --shots-dir needs a path");
                    return ExitCode::from(64);
                };
                shots_dir = Some(PathBuf::from(dir));
            }
            _ => break,
        }
    }
    let Some(case_dir) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(64);
    };
    if args.next().is_some() {
        eprintln!("{USAGE}");
        return ExitCode::from(64);
    }
    let shots = match (shot_frames, shots_dir) {
        (None, None) => None,
        (Some(frames), Some(dir)) => match parse_shot_frames(&frames) {
            Ok(frames) => Some(ShotRequest { frames, dir }),
            Err(e) => {
                eprintln!("{USAGE}\n  {e}");
                return ExitCode::from(64);
            }
        },
        _ => {
            eprintln!("{USAGE}\n  --shots and --shots-dir go together");
            return ExitCode::from(64);
        }
    };

    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rom = rom.unwrap_or_else(|| repo.join("hg_usa.nds"));

    match run(&case_dir, &rom, update, shots.as_ref()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("apricorn-replay: {message}");
            ExitCode::from(64)
        }
    }
}

fn run(
    case_dir: &str,
    rom: &Path,
    update: bool,
    shots: Option<&ShotRequest>,
) -> Result<ExitCode, String> {
    let case = Case::load(Path::new(case_dir)).map_err(|e| e.to_string())?;

    let actual = match shots {
        Some(shots) => {
            let trace = case
                .run_oracle_with_shots(rom, shots)
                .map_err(|e| e.to_string())?;
            let written = shots
                .frames
                .iter()
                .filter(|&&f| f < case.frames())
                .count();
            println!(
                "wrote {written} frame(s) of screenshots to {}",
                shots.dir.display()
            );
            trace
        }
        None => case.run_oracle(rom).map_err(|e| e.to_string())?,
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

    let report =
        diff::compare(&expected, &actual, Some(&case.regions)).map_err(|e| e.to_string())?;
    println!("{report}");
    Ok(match report.verdict {
        apricorn_harness::Verdict::Equivalent => ExitCode::SUCCESS,
        apricorn_harness::Verdict::Diverged { .. } => ExitCode::FAILURE,
    })
}
