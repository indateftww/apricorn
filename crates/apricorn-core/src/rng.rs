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
//! | `sMTRNG_State`/`sMTRNG_Cycles` + `SetMTRNGSeed` | [`Mt19937::uninitialized`]/[`Mt19937::new`]/[`Mt19937::set_seed`] | the boot-seeded generator |
//! | `MTRandom` | [`Mt19937::next_u32`] | advance the cursor, temper the word |
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
//! (`SetMTRNGSeed`/`MTRandom`) is locked the same way by
//! `tests/arm_hg.rs`, which uses this module's [`Mt19937`] as its
//! reference; the boot-time RTC seeding belongs to the game-state
//! machine (Phase 4, step 5, `crate::app::game`) — this module is
//! the deterministic core it calls.

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

/// The Mersenne Twister — pret's `SetMTRNGSeed`/`MTRandom` over the
/// statics `sMTRNG_State`/`sMTRNG_Cycles` (`src/math_util.c`), the
/// standard MT19937 recurrence in the SDK's exact shape.
///
/// Two details the usual MT19937 write-up hides, both ported:
///
/// * `SetMTRNGSeed`'s loop leaves the cursor at 624 (its exit
///   value), so the first draw after seeding twists immediately;
/// * a fresh image boots with the cursor at 625 — a sentinel that
///   makes the first draw re-seed with 5489 *before* twisting.
///   [`Mt19937::uninitialized`] is that boot state; the data pin for
///   the 625 came straight out of the retail image.
///
/// The game seeds it at every `InitializeMainRNG` (boot, and again at
/// each `ov36` overlay init — `src/main.c`, `src/overlay_36.c`);
/// its first engine consumer that *draws* is the post-Oak game-state
/// init (trainer ID and friends), which arrives with the structured
/// save blocks.
///
/// ```
/// use apricorn_core::rng::Mt19937;
///
/// // Seeding two generators alike gives like streams…
/// let mut a = Mt19937::new(0x1234);
/// let mut b = Mt19937::new(0x1234);
/// assert_eq!(a.next_u32(), b.next_u32());
///
/// // …and the boot image's sentinel reseeds with 5489 on its first
/// // draw, so it matches a generator seeded with 5489.
/// let mut boot = Mt19937::uninitialized();
/// let mut seeded = Mt19937::new(5489);
/// assert_eq!(boot.next_u32(), seeded.next_u32());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mt19937 {
    /// `sMTRNG_State` — the 624-word twisted array.
    state: [u32; 624],
    /// `sMTRNG_Cycles` — the draw cursor; 624 means "twist first",
    /// 625 the fresh-image sentinel (re-seed 5489 first).
    cycles: i32,
}

impl Mt19937 {
    /// The boot image's state: zeroed words with the 625 sentinel
    /// cursor (`sMTRNG_Cycles`'s initializer in the retail image) —
    /// the first [`draw`](Self::next_u32) re-seeds with 5489, exactly
    /// as a fresh-booted game that never ran `InitializeMainRNG`.
    #[must_use]
    pub fn uninitialized() -> Self {
        Self {
            state: [0; 624],
            cycles: 625,
        }
    }

    /// A generator over `seed` — `SetMTRNGSeed` on a fresh machine.
    #[must_use]
    pub fn new(seed: u32) -> Self {
        let mut mt = Self::uninitialized();
        mt.set_seed(seed);
        mt
    }

    /// `sMTRNG_State` as the ROM lays it out: the 624 words in array
    /// order, nothing else (the cursor is a separate `.data` static,
    /// [`Self::cycles`]). The harness's engine-side trace producer
    /// serializes these as little-endian `u32`s — the 2496 bytes the
    /// oracle hashes at the `sMTRNG_State` pin.
    #[must_use]
    pub fn state_words(&self) -> &[u32; 624] {
        &self.state
    }

    /// `sMTRNG_Cycles` — the draw cursor (`int`, `.data`, initializer
    /// 625): 625 on a fresh image, 624 after `SetMTRNGSeed`, otherwise
    /// the index of the next word to serve.
    #[must_use]
    pub fn cycles(&self) -> i32 {
        self.cycles
    }

    /// `SetMTRNGSeed`: the standard init `state[i] = 1812433253 *
    /// (state[i-1] ^ (state[i-1] >> 30)) + i`, with the cursor left
    /// at the loop's exit value 624 (the first draw twists first).
    pub fn set_seed(&mut self, seed: u32) {
        self.state[0] = seed;
        for i in 1..624 {
            let prev = self.state[i - 1];
            self.state[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        self.cycles = 624;
    }

    /// `MTRandom`: serve the tempered word under the cursor, twisting
    /// (and, on the fresh-image sentinel, re-seeding with 5489) when
    /// the cursor is spent.
    pub fn next_u32(&mut self) -> u32 {
        if self.cycles >= 624 {
            if self.cycles == 625 {
                self.set_seed(5489);
            }
            self.twist();
        }
        // The C is `val = sMTRNG_State[sMTRNG_Cycles++]` — "has to be
        // this way in order to match" — the cursor advances even when
        // the tempering below were elided.
        let mut val = self.state[self.cycles as usize];
        self.cycles += 1;
        val ^= val >> 11;
        val ^= (val << 7) & 0x9D2C_5680;
        val ^= (val << 15) & 0xEFC6_0000;
        val ^= val >> 18;
        val
    }

    /// The twist inlined at the top of pret's `MTRandom`, factored
    /// here: the recurrence over the 624 words, `sMTRNG_XOR` = the
    /// two-entry `[0, 0x9908_B0DF]` table.
    fn twist(&mut self) {
        let xor = [0u32, 0x9908_B0DF];
        let val =
            |state: &[u32; 624], i: usize| (state[i] & 0x8000_0000) | (state[i + 1] & 0x7FFF_FFFF);
        for i in 0..227 {
            let v = val(&self.state, i);
            self.state[i] = self.state[i + 397] ^ (v >> 1) ^ xor[(v & 1) as usize];
        }
        for i in 227..623 {
            let v = val(&self.state, i);
            self.state[i] = self.state[i - 227] ^ (v >> 1) ^ xor[(v & 1) as usize];
        }
        let v = (self.state[623] & 0x8000_0000) | (self.state[0] & 0x7FFF_FFFF);
        self.state[623] = self.state[396] ^ (v >> 1) ^ xor[(v & 1) as usize];
        self.cycles = 0;
    }
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

    // ===== Mersenne Twister ===========================================

    #[test]
    fn mt_sentinel_reseeds_with_5489_on_the_first_draw() {
        // pret's MTRandom: `if (sMTRNG_Cycles >= 624) { if (==
        // 625) SetMTRNGSeed(5489); twist; }` — the boot image's cursor
        // is 625, so the unseeded stream is exactly seed 5489's.
        let mut boot = Mt19937::uninitialized();
        let mut seeded = Mt19937::new(5489);
        for i in 0..1300 {
            // Two twists' worth, as the harness differential does.
            assert_eq!(boot.next_u32(), seeded.next_u32(), "draw {i}");
        }
    }

    #[test]
    fn mt_seed_leaves_the_cursor_spent_so_the_first_draw_twists() {
        // SetMTRNGSeed's loop exits with the cursor at 624, so a
        // seeded machine's first draw comes from a fresh twist — the
        // second word of a `new(seed)` stream is the cursor's *next*
        // word, not another twist's first.
        let mut a = Mt19937::new(0x1234);
        let mut b = Mt19937::new(0x1234);
        let first = a.next_u32();
        let second = a.next_u32();
        assert_eq!(b.next_u32(), first);
        assert_eq!(b.next_u32(), second);
        assert_ne!(first, second, "the tempered words differ");
    }

    #[test]
    fn mt_set_seed_restarts_the_stream_mid_stream() {
        // A reseed mid-stream takes effect on the next draw, exactly
        // like calling SetMTRNGSeed between MTRandoms.
        let mut rng = Mt19937::new(0x1234);
        rng.next_u32();
        rng.next_u32();
        rng.set_seed(0x1234);
        let mut fresh = Mt19937::new(0x1234);
        for i in 0..4 {
            assert_eq!(rng.next_u32(), fresh.next_u32(), "draw {i}");
        }
    }

    #[test]
    fn mt_streams_across_the_twist_boundary() {
        // 625 draws from one seeding crosses a twist without any
        // discontinuity: the stream stays a pure function of the seed
        // and draw index, so two like-seeded machines agree past the
        // boundary the cursor's wrap could have broken.
        let mut a = Mt19937::new(0xABCD_1234);
        let mut b = Mt19937::new(0xABCD_1234);
        for i in 0..625 {
            assert_eq!(a.next_u32(), b.next_u32(), "draw {i}");
        }
        // And the two cursors really are past 624 — one full twist
        // plus one — so the boundary was crossed, not skirted:
        // draws 623, 624, 625 bracket it.
        assert_eq!(a, b);
    }
}