# RNG — the LCG family (`apricorn-core::rng`)

Reference for the game's random number generation, established in
Phase 4 step 3. Source: pret `src/math_util.c` +
`include/math_util.h`, with the retail ARM9 image as the oracle:
`apricorn-harness`'s `tests/rng_hg.rs` calls the original pinned
functions through arm-runner and locks the Rust port to their output
draw by draw (Phase 2's per-function differential methodology, now
aimed at engine code for the first time).

## The recurrence

One 32-bit linear congruential generator drives everything:

```text
state = state * 1103515245 + 24691    (mod 2^32)
draw  = state >> 16                   (the top 16 bits)
```

`1103515245` is `0x41C64E6D` — the literal-pool word that located the
math_util pins (`docs/arm-runner.md`). The LCG is full-period mod 2³²
(`a ≡ 1 (mod 4)`, odd increment; Hull–Dobell), so every state is
reachable from every seed; the seed *is* the state, nothing is
premixed.

## The family

| pret (`math_util.c`) | Rust | notes |
|---|---|---|
| `sLCRNG_State` + `SetLCRNGSeed` / `GetLCRNGSeed` | `Lcrng::new` / `set_seed` / `seed` | the main field RNG |
| `u16 LCRandom(void)` | `Lcrng::next_u16` | advance, draw the top 16 bits |
| `LCRandRange(maximum)` | `Lcrng::rand_range` | `static inline` in the header; `draw % maximum`, `0` for `maximum <= 1` |
| `u32 PRandom(u32 seed)` | `rng::prandom` | stateless `seed * 1812433253 + 1` |
| `MonEncryptionLCRNG(u32 *seed)` | a local `Lcrng` | the same recurrence over a caller-owned seed |

Three quirks are pinned deliberately:

* **The draw width.** `LCRandom` returns only the top 16 bits — the
  low half of the state carries entropy forward but is never itself
  a result. Battle crit rolls, encounter checks, and shiny/PID
  generation in Gen 4 all consume 16-bit draws; getting this wrong
  shifts every downstream behavior.
* **`LCRandRange`'s early return.** `GF_ASSERT(maximum != 0)` guards a
  debug crash, then `maximum <= 1` returns `0` without drawing. The
  port `debug_assert`s the contract and stays total in release.
* **`PRandom` is not the LCG step.** It is the MT19937 *seeding*
  recurrence (`1812433253 = 0x6C078965`,
  `x_{n+1} = 1812433253·(x_n ^ (x_n >> 30)) + n`) with the XOR-shift
  dropped and the index pinned to 1, applied statelessly — the game
  uses it for save-file sub-seed derivation (`src/save.c`) and egg
  PID rerolls (`src/get_egg.c`). Phase 4's save step will lean on it.

`MonEncryptionLCRNG` is private in the C but has its own committed
pin (`MonEncryptionLCRNG`, Thumb, `0x0201FF78` — the pool scan for
the same `0x41C64E6D` constant caught the second occurrence in
`_MonEncryptSegment`). It exists because Pokémon data blocks are
XOR-masked with a stream from this recurrence seeded per-mon; the
same `Lcrng` over a local seed is the port, no separate type needed.

## Differential coverage

`crates/apricorn-harness/tests/rng_hg.rs` (ROM-gated, skips silently
without the dump — the usual `*_hg` policy):

* `lcrng_matches_the_original_stream` — four seeds (the three the
  core unit tests pin known draws for, plus `0xDEADBEEF`), 64 draws
  each: every `LCRandom` r0 and every `GetLCRNGSeed` state must match
  `Lcrng` exactly.
* `prandom_matches_the_original` — five seeds through the original
  `PRandom` vs `rng::prandom`.
* `mon_encryption_lcg_matches_the_original` — the original
  `MonEncryptionLCRNG` advancing a scratch seed slot for 64 draws vs
  a local `Lcrng`: draw *and* the in-memory state after each.

`LCRandRange` cannot be called through arm-runner (it is `static
inline` — no body in the image), so its differential is
compositional: the draw it reduces is differential-tested, and the
reduction itself (`u16 % maximum`, early return) is pinned by the
unit tests beside `Lcrng::rand_range`.

Core unit tests (`crates/apricorn-core/src/rng.rs`) carry committed
known values for ROM-less CI: the 0x1234 stream's first draw is
`0x4DCB` (the same value `docs/arm-runner.md` documents), with
seed 0 and `0xFFFFFFFF` streams alongside.

## Deferred

* **Mersenne Twister** (`SetMTRNGSeed`/`MTRandom`, `sMTRNG_State`) —
  reference-tested against the original in
  `apricorn-harness/tests/arm_hg.rs`, but the game reaches for it
  only in subsystems the engine does not have yet. Port it the day a
  consumer lands; the pins (`SetMTRNGSeed`, `MTRandom`, and the
  `sMTRNG_*` data pins) are already committed and verified.
* **Boot seeding from the RTC.** The game seeds the LCG from the
  clock around frame 185 (`docs/equivalence.md`); that is game-state
  machine behavior (Phase 4, step 5), not RNG behavior. `Lcrng` is
  the deterministic core the state machine will call with whatever
  seed the boot flow derives.