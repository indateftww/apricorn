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

pub mod cache;
pub mod formats;
pub mod nds;

/// Semantic version of the engine's state-machine / trace format.
///
/// Any change to observable game state layout bumps this so the harness can
/// reject traces produced by a different engine version.
pub const STATE_FORMAT_VERSION: u32 = 0;

/// A single stepped frame of the headless game.
///
/// Placeholder for the Phase 3 game loop. The engine advances in logical
/// NDS frames (approx. 59.8268 Hz per screen refresh; fixed-timestep).
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
        // 0 is the pre-implementation sentinel: no trace format exists yet,
        // so the harness must refuse to compare anything.
        assert_eq!(STATE_FORMAT_VERSION, 0);
    }

    #[test]
    fn frame_index_roundtrips() {
        let f = Frame { index: 59 };
        assert_eq!(f.index, 59);
    }
}
