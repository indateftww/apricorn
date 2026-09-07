//! The game's random number generation — the LCG family
//! (`src/math_util.c`, PLAN.md Phase 4, step 3).
//!
//! HeartGold draws its randomness from one 32-bit linear
//! congruential generator and two helpers around it:
//!
//! | pret | here | shape |
//! |---|---|---|
//! | `sLCRNG_State` + `SetLCRNGSeed`/`GetLCRNGSeed` | [`Lcrng::new`]/[`Lcrng::set_seed`]/[`Lcrng::seed`] | the main field RNG |
//! | `LCRandom` | [`Lcrng::next_u16`] | advance state, draw the top 16 bits |
//! | `LCRandRange` (static inline) | [`Lcrng::rand_range`] | draw `% maximum`, `0` for `maximum <= 1` |
//! | `PRandom` | [`prandom`] | stateless: `seed * 1812433253 + 1` |
//! | `MonEncryptionLCRNG` | a local [`Lcrng`] | the same recurrence over a caller-owned seed |
//!
//! The recurrence is the classic Pokémon LCG —
//! `state = state * 1103515245 + 24691`, draw = the top 16 bits
//! (`state >> 16`). It is full-period mod 2³² (`a ≡ 1 (mod 4)`, odd
//! increment, per Hull–Dobell), so every state is reachable from
//! every seed. The mon-encryption RNG reuses the exact recurrence
//! over a per-mon local seed, so [`Lcrng`] serves both; the C
//! originals are transcribed in `refs/pokeheartgold/src/math_util.c`.
//!
//! **Parity is differential, not asserted:** `apricorn-harness`'s
//! `tests/rng_hg.rs` calls the original pinned functions
//! (`SetLCRNGSeed`, `LCRandom`, `GetLCRNGSeed`, `PRandom`,
//! `MonEncryptionLCRNG`) out of the retail ARM9 image via
//! arm-runner and locks this implementation to their output draw by
//! draw. `LCRandRange` is a `static inline` in the C (no pinned body
//! exists); its modulo is plain arithmetic, pinned by unit tests
//! over the differential-tested draw. The Mersenne Twister
//! (`SetMTRNGSeed`/`MTRandom`) stays in the harness until an engine
//! consumer needs it, and the boot-time RTC seeding belongs to the
//! game-state machine (Phase 4, step 5) — this module is the
//! deterministic core both of those will call.

/// The LCG multiplier — pret `LCRandom`, the word `0x41C64E6D` whose
/// literal pool located the math_util pins.
pub const LCG_MULTIPLIER: u32 = 1103_515_245;

/// The LCG increment.
pub const LCG_INCREMENT: u32 = 24_691;

/// The main random number generator: pret's `sLCRNG_State` and its
/// accessors (`src/math_util.c`).
///
/// The state advances on every [`draw`](Self::next_u16); the returned
/// value is the post-advance state's top 16 bits, exactly the original
/// `LCRandom`. The same value type drives the per-mon encryption RNG —
/// instantiate it over the caller-owned seed instead of the global.
///
/// ```
/// use apricorn_core::rng::Lcrng;
///
/// let mut rng = Lcrng::new(0x1234);
/// assert_eq!(rng.next_u16(), 0x4DCB); // the committed known draw
/// assert_eq!(rng.seed(), 0x4DCB_F897); // and the state behind it
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lcrng {
    /// The 32-bit LCG state — `sLCRNG_State`.
    state: u32,
}

impl Lcrng {
    /// `SetLCRNGSeed`: a generator over `seed`.
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// `GetLCRNGSeed`: the current 32-bit state.
    #[must_use]
    pub fn seed(&self) -> u32 {
        self.state
    }

    /// `SetLCRNGSeed`: jump to `seed` (the same draw stream as a fresh
    /// [`Lcrng::new`]).
    pub fn set_seed(&mut self, seed: u32) {
        self.state = seed;
    }

    /// `LCRandom` (and `MonEncryptionLCRNG`): advance the state and
    /// return its top 16 bits — the game's fundamental draw, used for
    /// everything from encounter rolls to crits.
    pub fn next_u16(&mut self) -> u16 {
        self.state = self
            .state
            .wrapping_mul(LCG_MULTIPLIER)
            .wrapping_add(LCG_INCREMENT);
        (self.state >> 16) as u16
    }

    /// `LCRandRange` (`include/math_util.h`, static inline): a draw
    /// reduced modulo `maximum`.
    ///
    /// The original asserts `maximum != 0` (`GF_ASSERT` — a debug
    /// crash) and returns `0` for `maximum <= 1`; this port
    /// `debug_assert`s the nonzero contract and returns `0` for both
    /// cases, so release builds stay total.
    pub fn rand_range(&mut self, maximum: u16) -> u16 {
        debug_assert!(maximum != 0, "LCRandRange: maximum must be nonzero");
        if maximum <= 1 {
            return 0;
        }
        self.next_u16() % maximum
    }
}

/// `PRandom` — the stateless LCG step used to derive sub-seeds
/// (`seed * 1812433253 + 1`): save-file seeding (`src/save.c`) and egg
/// PID rerolls (`src/get_egg.c`) both run it over a caller's value.
///
/// ```
/// use apricorn_core::rng::prandom;
///
/// assert_eq!(prandom(0), 1); // the increment alone
/// ```
#[must_use]
pub fn prandom(seed: u32) -> u32 {
    seed.wrapping_mul(1_812_433_253).wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Streams computed from the C semantics (`state * 1103515245 +
    /// 24691`, draw = `state >> 16`) — the first entry of the 0x1234
    /// row is the `0x4dcb` draw documented in `docs/arm-runner.md`.
    /// The original functions confirm every value in
    /// `apricorn-harness`'s `tests/rng_hg.rs`.
    const STREAMS: [(u32, [u16; 4]); 3] = [
        (
            0x1234,
            [0x4DCB, 0xE161, 0x4340, 0xFFF1],
        ),
        (
            0,
            [0x0000, 0xE97E, 0x5271, 0x31B0],
        ),
        (
            0xFFFF_FFFF,
            [0xBE3A, 0x26DB, 0xD1F3, 0x43AA],
        ),
    ];

    #[test]
    fn draws_match_the_reference_streams() {
        for (seed, draws) in STREAMS {
            let mut rng = Lcrng::new(seed);
            for (i, &draw) in draws.iter().enumerate() {
                assert_eq!(rng.next_u16(), draw, "seed {seed:#x}, draw {i}");
            }
        }
    }

    #[test]
    fn set_seed_jumps_the_stream() {
        // A reseed mid-stream takes effect on the next draw, exactly
        // like calling SetLCRNGSeed between LCRandoms.
        let mut rng = Lcrng::new(0x1234);
        rng.next_u16();
        rng.set_seed(0);
        assert_eq!(rng.next_u16(), 0x0000);
        assert_eq!(rng.seed(), 0x6073);

        // And seeding to the current state is a no-op, draw-wise.
        let state = rng.seed();
        rng.set_seed(state);
        assert_eq!(rng.seed(), state);
    }

    #[test]
    fn rand_range_reduces_the_same_draw() {
        // Seed 0x1234's first draw is 0x4DCB = 19915: 19915 % 100 is
        // 15, % 2 is 1 — the same single underlying draw feeds both.
        let mut rng = Lcrng::new(0x1234);
        assert_eq!(rng.rand_range(100), 15);
        rng.set_seed(0x1234);
        assert_eq!(rng.rand_range(2), 1);

        // maximum == 1 draws nothing and yields 0 — the original's
        // early return. (maximum == 0 is a contract violation the
        // original GF_ASSERTs on; `debug_assert!` fires here too.)
        rng.set_seed(0x1234);
        assert_eq!(rng.rand_range(1), 0);
        assert_eq!(rng.seed(), 0x1234, "no draw happened for maximum <= 1");

        // Across a whole spread of moduli the result never leaves
        // range (and every draw stays in range by construction).
        for maximum in 2..=u16::MAX {
            let draw = rng.rand_range(maximum);
            assert!(draw < maximum);
        }
    }

    #[test]
    fn prandom_matches_the_reference() {
        // Computed from `seed * 1812433253 + 1` (mod 2^32).
        assert_eq!(prandom(0x1234), 0x7931_0285);
        assert_eq!(prandom(0), 1);
        assert_eq!(prandom(0xFFFF_FFFF), 0x93F8_769C);
    }
}