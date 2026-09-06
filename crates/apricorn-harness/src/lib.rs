//! # apricorn-harness
//!
//! Behavioral-equivalence testing harness (PLAN.md Phase 2).
//!
//! The methodology: a deterministic **oracle** (patched headless melonDS,
//! fixed RTC, scripted input) and our engine replay the same input script;
//! both periodically hash the same watched game-state regions; a
//! comparator walks the two traces in lockstep and reports the first
//! divergence. Equivalence is **frame-indexed** (VBlank count since boot),
//! never wall-clock — determinism is the whole point.
//!
//! Modules:
//!
//! - [`regions`] — the watched-region configuration (`regions.conf`):
//!   which memory ranges to hash, how often, and whether a mismatch is
//!   fatal (`hard`) or tolerated (`drift`).
//! - [`trace`] — the trace format both producers emit: a header gate
//!   (state-format version, ROM hash, canonical regions hash) plus
//!   frame-sampled region hashes and per-function call records.
//! - **input scripts** (`.apin`) — frame-timed button/stylus scripts,
//!   executable by both the oracle and the headless engine.
//! - **diff** — first-divergence comparator over two traces ([`Verdict`]).
//! - **arm-runner** — a test-only ARM9 interpreter that loads original
//!   ARM9 code and calls original functions with controlled inputs, the
//!   per-function oracle where pret has no C. Never shipped in the game.
//!
//! The trace/diff machinery is producer-agnostic so the real engine
//! (Phases 4+) plugs in without touching any format. All formats are
//! documented in `docs/equivalence.md`.

#![deny(missing_docs)]

pub mod diff;
pub mod input;
pub mod regions;
pub mod trace;

use std::fmt;

/// An error while parsing a harness format or gating a comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessError {
    /// Malformed syntax; `line` is the 1-based line in the source text.
    Syntax {
        /// 1-based line number the error was found on.
        line: usize,
        /// What is wrong.
        what: String,
    },
    /// A well-formed input rejected by a comparison gate (state-format
    /// version, ROM hash, or regions-hash mismatch).
    Gate {
        /// Which gate rejected the input and why.
        what: String,
    },
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HarnessError::Syntax { line, what } => write!(f, "line {line}: {what}"),
            HarnessError::Gate { what } => write!(f, "gate: {what}"),
        }
    }
}

impl std::error::Error for HarnessError {}

/// Result of comparing an oracle trace against an engine trace.
///
/// The Phase 2 `apricorn-diff` comparator's verdict. `Diverged` names the
/// frame of the first `hard`-bucket mismatch; a fuller report (region,
/// observed drift) travels in the comparator's `Report`, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every hard-equality state bucket matched for the whole trace.
    Equivalent,
    /// First divergence found; `frame` is where it occurred.
    Diverged {
        /// Frame index of the first mismatch.
        frame: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_verdict_has_no_frame() {
        assert_eq!(Verdict::Equivalent, Verdict::Equivalent);
        assert_ne!(Verdict::Equivalent, Verdict::Diverged { frame: 120 });
    }

    #[test]
    fn errors_display_with_location() {
        let e = HarnessError::Syntax {
            line: 7,
            what: "unknown header key".to_string(),
        };
        assert_eq!(e.to_string(), "line 7: unknown header key");
        let e = HarnessError::Gate {
            what: "state-format 2 != 1".to_string(),
        };
        assert_eq!(e.to_string(), "gate: state-format 2 != 1");
    }
}
