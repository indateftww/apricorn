//! # apricorn-core
//!
//! Headless HeartGold game engine and game logic.
//!
//! This crate must never contain platform code: no windowing, no audio
//! backend, no file pickers. It runs deterministically step-by-step so the
//! behavioral-equivalence harness (`apricorn-harness`) can drive it and
//! compare traces against the oracle.
//!
//! Plan reference: `PLAN.md` Phases 3-8. Substantive code starts with the
//! fixed-timestep game state loop (Phase 3) and ROM data tables (Phase 4).

#![deny(missing_docs)]

pub mod app;
pub mod assets;
pub mod cache;
pub mod formats;
pub mod frame;
pub mod input;
pub mod nds;

/// Semantic version of the engine's state-machine / trace format.
///
/// Any change to observable game state layout bumps this so the harness can
/// reject traces produced by a different engine version.
///
/// 1 is the first implemented format: the Phase 2 harness trace grammar
/// (`apricorn-harness::trace`, `docs/equivalence.md`) — the oracle, the
/// arm-runner, and the engine all emit state-identifying hashes under
/// this version, and comparators refuse cross-version pairs.
pub const STATE_FORMAT_VERSION: u32 = 1;

/// A single stepped frame of the headless game — the per-tick token
/// the [`app::App`] trait consumes.
///
/// The engine advances in logical NDS frames (approx. 59.8268 Hz per
/// screen refresh; fixed-timestep): every [`Frame`] passed to
/// `App::tick` carries the zero-based frame index since boot, and the
/// app's tick is a pure function of that index and the input, so a
/// frame range replays deterministically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Zero-based index of this frame since boot.
    pub index: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_format_version_is_stable() {
        // 1 is the Phase 2 harness trace format (see docs/equivalence.md);
        // the harness gate rejects traces whose state-format differs, so
        // this number only ever moves deliberately.
        assert_eq!(STATE_FORMAT_VERSION, 1);
    }

    #[test]
    fn frame_index_roundtrips() {
        let f = Frame { index: 59 };
        assert_eq!(f.index, 59);
    }
}
