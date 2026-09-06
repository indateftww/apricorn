//! Glue to the oracle binary: compile the harness's text formats into
//! the little-endian blobs the patched melonDS consumes, locate the
//! build, shell out, and parse the trace it writes back.
//!
//! The Rust side owns every text format ([`crate::input`],
//! [`crate::regions`]); the oracle never parses text — this module is
//! the single compiler for the blob contracts documented in
//! `docs/oracle.md`:
//!
//! - regions: `u32 count`, then per region `u32 addr, size, sample,
//!   name_len` + name bytes;
//! - input: `u32 frame_count`, then per frame `u16 keymask, u8
//!   touch_down, u8 pad, u16 x, u16 y` (frames beyond the list get
//!   all-zero input — boot-idle);
//! - probes: `u32 count`, then per probe `u32 frame, entry, nargs,
//!   args[4], name_len` + name bytes.
//!
//! The trace header's `input-sha1` / `regions-sha1` are passthrough
//! hashes of the canonical text files, computed here — they gate diff
//! pairs, and only this side knows the canonical forms.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::HarnessError;
use crate::input::{FrameInput, InputScript};
use crate::regions::RegionSet;
use crate::trace::Trace;

/// One scheduled function probe (the source of a `C` record): call
/// `name` at `entry` (bit 0 = Thumb) with `args` (0..=4) at `frame`'s
/// boundary, before that frame's input and execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeSpec {
    /// Frame boundary to run the probe at.
    pub frame: u32,
    /// Entry address; bit 0 selects Thumb.
    pub entry: u32,
    /// Arguments r0..r3 (fewer than 4 leave the rest zeroed).
    pub args: Vec<u32>,
    /// The function's name — a `pins` table entry; the trace's `fn`.
    pub name: String,
}

/// Locates the built oracle under `out/oracle/` (gitignored; built by
/// `oracle/setup.ps1`). `None` when absent — callers skip silently.
#[must_use]
pub fn find_binary() -> Option<PathBuf> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["apricorn-oracle.exe", "apricorn-oracle"] {
        let path = repo.join("out/oracle").join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Compiles the regions blob. Every region in file order.
#[must_use]
pub fn regions_blob(set: &RegionSet) -> Vec<u8> {
    let regions = set.regions();
    let mut blob = Vec::new();
    push_u32(&mut blob, regions.len() as u32);
    for r in regions {
        push_u32(&mut blob, r.address);
        push_u32(&mut blob, r.size);
        push_u32(&mut blob, r.sample);
        push_u32(&mut blob, r.name.len() as u32);
        blob.extend_from_slice(r.name.as_bytes());
    }
    blob
}

/// Compiles the input blob for `frames` frames: the script's state at
/// each frame, then all-zero input for frames past its `end`.
#[must_use]
pub fn input_blob(script: &InputScript, frames: u32) -> Vec<u8> {
    let mut blob = Vec::new();
    push_u32(&mut blob, frames);
    for frame in 0..frames {
        let FrameInput { mask, touch } = if frame < script.end {
            script.at(frame)
        } else {
            FrameInput::IDLE
        };
        blob.extend_from_slice(&mask.to_le_bytes());
        let (down, x, y) = match touch {
            Some((x, y)) => (1u8, x, y),
            None => (0u8, 0, 0),
        };
        blob.push(down);
        blob.push(0); // padding — part of the format contract
        blob.extend_from_slice(&x.to_le_bytes());
        blob.extend_from_slice(&y.to_le_bytes());
    }
    blob
}

/// Compiles the probes blob. `args` pads to four; more than four is a
/// programming error caught here rather than on the C++ side.
///
/// # Errors
/// Returns a [`HarnessError::Gate`] naming the probe if it carries
/// more than 4 arguments.
pub fn probes_blob(probes: &[ProbeSpec]) -> Result<Vec<u8>, HarnessError> {
    let mut blob = Vec::new();
    push_u32(&mut blob, probes.len() as u32);
    for p in probes {
        if p.args.len() > 4 {
            return Err(HarnessError::Gate {
                what: format!("probe {}: more than 4 args", p.name),
            });
        }
        push_u32(&mut blob, p.frame);
        push_u32(&mut blob, p.entry);
        push_u32(&mut blob, p.args.len() as u32);
        for a in 0..4 {
            push_u32(&mut blob, p.args.get(a).copied().unwrap_or(0));
        }
        push_u32(&mut blob, p.name.len() as u32);
        blob.extend_from_slice(p.name.as_bytes());
    }
    Ok(blob)
}

/// One oracle invocation.
#[derive(Debug, Clone)]
pub struct OracleRun<'a> {
    /// The ROM dump to boot.
    pub rom: &'a Path,
    /// The watched regions (hash source + blob + passthrough hash).
    pub regions: &'a RegionSet,
    /// The input script; `None` boots with no input at all.
    pub input: Option<&'a InputScript>,
    /// The probes to run, in order.
    pub probes: &'a [ProbeSpec],
    /// How many frames to run.
    pub frames: u32,
    /// The pinned RTC (`YYYY-MM-DDTHH:MM:SS`); `None` uses the oracle's
    /// documented default.
    pub rtc: Option<&'a str>,
    /// The `producer` name for the trace header; `None` uses the
    /// oracle's own default.
    pub producer: Option<&'a str>,
}

/// Distinct scratch filenames per process, and per concurrent run.
static RUN_COUNTER: AtomicU32 = AtomicU32::new(0);

impl OracleRun<'_> {
    /// The `input-sha1` passthrough this run will pin: the script's
    /// canonical-text hash, or the hash of the canonical empty script
    /// when there is no input.
    #[must_use]
    pub fn input_sha1_hex(&self) -> String {
        match self.input {
            Some(script) => script.sha1_hex(),
            None => InputScript {
                rtc: None,
                end: 0,
                events: Vec::new(),
            }
            .sha1_hex(),
        }
    }

    /// The `regions-sha1` passthrough: the canonical `regions.conf`
    /// hash of this run's region set.
    #[must_use]
    pub fn regions_sha1_hex(&self) -> String {
        self.regions.sha1_hex()
    }

    /// Runs the oracle and parses the trace it writes.
    ///
    /// Scratch blobs and the raw trace go to the system temp dir and
    /// are removed afterwards. `--frames 0` is valid and produces a
    /// probes-only trace (no `F` records).
    ///
    /// # Errors
    /// Returns a [`HarnessError::Gate`] when the oracle is missing,
    /// exits nonzero (its stderr is included), or writes a malformed
    /// trace ([`HarnessError::Syntax`]).
    pub fn run(&self) -> Result<Trace, HarnessError> {
        let Some(binary) = find_binary() else {
            return Err(HarnessError::Gate {
                what: "oracle binary not built (run oracle/setup.ps1)".to_string(),
            });
        };
        let id = RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("apricorn-oracle-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| HarnessError::Gate {
            what: format!("cannot create {}: {e}", dir.display()),
        })?;
        // Best-effort cleanup on every path.
        let cleanup = |dir: &Path| {
            let _ = std::fs::remove_dir_all(dir);
        };

        let regions_path = dir.join("regions.bin");
        let out_path = dir.join("out.trace");
        if let Err(e) = std::fs::write(&regions_path, regions_blob(self.regions)) {
            cleanup(&dir);
            return Err(HarnessError::Gate {
                what: format!("cannot write {}: {e}", regions_path.display()),
            });
        }

        let input_path = dir.join("input.bin");
        let has_input = self.input.is_some();
        if has_input
            && let Err(e) = std::fs::write(
                &input_path,
                input_blob(self.input.expect("checked above"), self.frames),
            )
        {
            cleanup(&dir);
            return Err(HarnessError::Gate {
                what: format!("cannot write {}: {e}", input_path.display()),
            });
        }

        let probes_path = dir.join("probes.bin");
        let has_probes = !self.probes.is_empty();
        let probes = match probes_blob(self.probes) {
            Ok(p) => p,
            Err(e) => {
                cleanup(&dir);
                return Err(e);
            }
        };
        if has_probes && let Err(e) = std::fs::write(&probes_path, probes) {
            cleanup(&dir);
            return Err(HarnessError::Gate {
                what: format!("cannot write {}: {e}", probes_path.display()),
            });
        }

        let mut cmd = Command::new(&binary);
        cmd.arg("run")
            .arg("--rom")
            .arg(self.rom)
            .arg("--out")
            .arg(&out_path)
            .arg("--regions")
            .arg(&regions_path)
            .arg("--frames")
            .arg(self.frames.to_string())
            .arg("--input-sha1")
            .arg(self.input_sha1_hex())
            .arg("--regions-sha1")
            .arg(self.regions_sha1_hex());
        if has_input {
            cmd.arg("--input").arg(&input_path);
        }
        if has_probes {
            cmd.arg("--probes").arg(&probes_path);
        }
        if let Some(rtc) = self.rtc {
            cmd.arg("--rtc").arg(rtc);
        }
        if let Some(producer) = self.producer {
            cmd.arg("--producer").arg(producer);
        }

        let status = cmd.output();
        let output = match status {
            Ok(o) => o,
            Err(e) => {
                cleanup(&dir);
                return Err(HarnessError::Gate {
                    what: format!("cannot run {}: {e}", binary.display()),
                });
            }
        };
        if !output.status.success() {
            cleanup(&dir);
            return Err(HarnessError::Gate {
                what: format!(
                    "oracle exited {:?}: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            });
        }

        let text = std::fs::read_to_string(&out_path).map_err(|e| {
            cleanup(&dir);
            HarnessError::Gate {
                what: format!("oracle wrote no trace at {}: {e}", out_path.display()),
            }
        });
        cleanup(&dir);
        let text = text?;
        Trace::parse(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region_set() -> RegionSet {
        RegionSet::parse("# name bucket address size sample\nrng hard 0x021D15A8 4 1\n").unwrap()
    }

    fn script() -> InputScript {
        InputScript::parse("# apricorn input v1\nend 4\n1 down A\n2 up A\n").unwrap()
    }

    #[test]
    fn regions_blob_matches_the_format_contract() {
        let blob = regions_blob(&region_set());
        // count, addr, size, sample, name_len, name
        let expected: Vec<u8> = [1u32, 0x021D_15A8, 4, 1, 3]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .chain(b"rng".iter().copied())
            .collect();
        assert_eq!(blob, expected);
    }

    #[test]
    fn input_blob_covers_the_script_then_idle() {
        let script = script();
        // 3 frames: down-A at 1, up-A at 2, frame 3 past `end` = idle.
        let blob = input_blob(&script, 3);
        assert_eq!(&blob[..4], &3u32.to_le_bytes());
        // frame 0: idle. frame 1: A (bit 0 of melonDS's keymask layout).
        assert_eq!(&blob[4..12], &[0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&blob[12..20], &[1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&blob[20..28], &[0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn probes_blob_pads_args_to_four() {
        let blob = probes_blob(&[ProbeSpec {
            frame: 5,
            entry: 0x0201_FD39, // | Thumb bit
            args: vec![0x1234],
            name: "SetLCRNGSeed".to_string(),
        }])
        .unwrap();
        let expected: Vec<u8> = [1u32, 5, 0x0201_FD39, 1, 0x1234, 0, 0, 0, 12]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .chain(b"SetLCRNGSeed".iter().copied())
            .collect();
        assert_eq!(blob, expected);
    }

    #[test]
    fn probes_blob_rejects_five_args() {
        let err = probes_blob(&[ProbeSpec {
            frame: 0,
            entry: 0,
            args: vec![1, 2, 3, 4, 5],
            name: "greedy".to_string(),
        }]);
        assert!(matches!(err, Err(HarnessError::Gate { .. })));
    }

    #[test]
    fn empty_input_sha_is_the_empty_script_sha() {
        let run = OracleRun {
            rom: Path::new("rom"),
            regions: &region_set(),
            input: None,
            probes: &[],
            frames: 0,
            rtc: None,
            producer: None,
        };
        assert_eq!(
            run.input_sha1_hex(),
            InputScript {
                rtc: None,
                end: 0,
                events: Vec::new()
            }
            .sha1_hex()
        );
    }
}
