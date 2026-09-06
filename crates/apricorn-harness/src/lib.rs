//! # apricorn-harness
//!
//! Behavioral-equivalence testing harness.
//!
//! Components planned in `PLAN.md` Phase 2:
//!
//! - **arm-runner**: an ARM9 interpreter that loads original overlays and
//!   calls original functions with controlled inputs (test-only; never
//!   shipped in the game).
//! - **trace format**: periodic hashes of game-state regions dumped from both
//!   the oracle (instrumented headless melonDS) and `apricorn-core`.
//! - **input scripts**: frame-timed button/stylus scripts executable by both
//!   the oracle and the headless engine.
//! - **diff**: first-divergence comparator with hard-equality vs. may-drift
//!   state buckets.
//!
//! Nothing here is implemented yet; this crate is the landing zone.

#![deny(missing_docs)]

/// Result of comparing an oracle trace against an engine trace.
///
/// Placeholder for the Phase 2 `apricorn-diff` comparator.
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
}
