# Behavioral equivalence — the harness methodology

Reference for `apricorn-harness` (PLAN.md Phase 2). This document defines
the methodology and the wire formats; the oracle and arm-runner each
have their own notes: `docs/oracle.md`, `docs/arm-runner.md`.

## The method

A deterministic **oracle** (patched headless melonDS, pinned RTC,
scripted input) and our engine replay the **same input script** on the
**same ROM**. Both periodically SHA-1 the same watched game-state
regions. A comparator walks the two traces in lockstep and reports the
first divergence: frame, region, and both hashes. Parity is
*demonstrated*, never asserted.

Three invariants make the comparison meaningful:

* **Frame-indexed equivalence.** A "frame" is a VBlank count since boot
  (the NDS refreshes at 59.8268 Hz — a documentation figure only;
  equivalence is never judged on wall-clock time, which is why the
  oracle pins the RTC rather than racing a real clock).
* **Hashes, not content.** Traces carry SHA-1 digests plus small
  register metadata, never ROM content or raw state — so they can be
  committed as regression baselines without shipping game data. (Raw
  oracle dumps stay gitignored regardless: `*.trace`,
  `*.state-dump`.)
* **Gates before comparison.** Both traces must agree on the engine
  state-format version (`apricorn_core::STATE_FORMAT_VERSION`), the ROM
  SHA-1, and the canonical regions SHA-1. A stale trace refuses to
  compare rather than producing a bogus divergence list.

## Producers

Three producers emit traces; the engine joined in Phase 4 without any
format change:

| Producer | What it replays |
|---|---|
| **Oracle** (patched melonDS, `docs/oracle.md`) | The full retail ROM: boot, title, menus, gameplay — the behavioral source of truth. |
| **arm-runner** (`docs/arm-runner.md`) | Original ARM9 functions called with controlled inputs in a test-only interpreter — the per-function oracle where pret has no C. |
| **Engine** (`apricorn-core` via `apricorn_harness::engine`, `docs/engine-runner.md`) | The real game under the same script and regions; regions resolve by pin *name* to engine state serialized as the ROM lays it out (`producer engine-apricorn-core`; `apricorn-run`, `apricorn-replay --engine`). |

Phase 2's demonstrated equivalence, before the engine existed, was:
(a) oracle replay traces (determinism baseline), and (b)
**oracle-vs-arm-runner differential probes** — the same function run
on both sides with the same seeded memory, hashes compared. That
validated our interpreter against the emulator before any game logic
existed. The engine-vs-oracle verdict on `corpus/boot-idle` is
measured in `tests/engine_hg.rs` and explained in
`docs/engine-runner.md` (a first divergence at frame 0, pinned
exactly; the oracle's committed baseline turned out to be a
soft-reset loop caused by its key-mask polarity, not an idle title).

Phase 2 closed with both demonstrated live:
`tests/equivalence_hg.rs` runs `SetLCRNGSeed(0x1234)` then `LCRandom`
at frame 20's boundary on the whole emulated console and on the bare
interpreter — registers and region hashes identical through the strict
comparator; and `corpus/boot-idle` replays EQUIVALENT through
`scripts/replay.ps1` (a different pinned RTC diverges at frame 185,
`sLCRNG_State`, with both hashes printed).

## The formats

All formats are line-oriented text: the C++ oracle emits them with
`printf`, humans can read them, and a divergence can be pinned to an
exact line.

### `regions.conf` — what to watch

One region per line, whitespace-separated:

```text
# name         bucket  address     size   sample  note
rng            hard    0x02001234  4      1       sLCRNG_State (arm9 bss)
anim-counter   drift   0x020089ab  4      30      OAM/anim scratch
```

* `bucket` — `hard` (mismatch = divergence) or `drift` (mismatch
  recorded, non-fatal; for state that may legitimately run out of step,
  e.g. animation counters).
* `sample` — hash every N frames (1 = every frame).

Addresses are **pinned data**, not code: they live in
`apricorn-harness`'s pins table with a per-pin prologue hash, verified
against the loaded image on every run. pret/pokeheartgold ships no
symbol files, so pins are discovered by scanning the ARM9 binary for
known constants (the LCG multiplier `0x41C64E6D`, the MT19937
`0x9908B0DF`, the CRC-CCITT polynomial `0x1021`, …) and confirmed
interactively once.

**Canonical form** (the bytes `regions-sha1` hashes): each region as
`name bucket 0x%08X size sample`, newline-terminated, in file order —
notes and comments excluded, so purely editorial edits never
invalidate stored traces.

### `input.apin` — what to replay

```text
rtc 2010-03-01T09:00:00
end 600
120 down A
150 up A
300 touch 128 96
310 lift
```

Actions are frame-timed; `at(frame)` yields the button mask and stylus
state for that frame — a single state machine both the oracle and the
headless engine evaluate, so neither side can interpret input
differently.

### Trace — what they report

Header (gate fields) + two record kinds:

```text
TRACE apricorn 1
producer oracle-melonds-1.1
state-format 1
rom-sha1 <sha1 of the ROM>
input-sha1 <sha1 of input.apin>
regions-sha1 <sha1 of canonical regions.conf>
frames 600
frame-rate 59.8268
rtc 2010-03-01T09:00:00
F 120 rng <sha1 of the region's bytes at frame 120>
C 0 LCRandom r0=0x00001234 r1=0x00000000 r2=0x00000000 r3=0x00000000 state=<sha1 of the region set>
```

* The first line's `1` is the **trace text format** version (bump when
  the grammar changes).
* `state-format` must equal `apricorn_core::STATE_FORMAT_VERSION`
  (currently 1) — the engine observable-state version. `Trace::parse`
  refuses mismatches; the comparator refuses pairs that differ in
  `state-format`, `rom-sha1`, or `regions-sha1`.
* `F` records hash one watched region at one frame.
* `C` records are per-function probes: the arguments (r0–r3) and the
  post-call hash over the region set.

## Comparison and the corpus

`apricorn-diff <expected> <actual>` walks both traces in lockstep: a
`hard` mismatch (or a record present on only one side) is a divergence
at that frame; `drift` mismatches are collected as warnings. Exit 0 =
EQUIVALENT, 1 = diverged, 64 = usage/gate error.

A **corpus case** is a committed directory:

```text
corpus/boot-idle/
  input.apin       # the pinned RTC, the length in frames, the events
  regions.conf     # what to watch, how often, which bucket
  probes.conf      # optional: fnprobe cases only
  expected.trace   # the committed baseline — hashes only
```

`probes.conf` (when present) schedules function probes, one per line:
`frame fn [args…]`, where `fn` is a `pins/arm9.tsv` code pin (its
table entry supplies the address and the Thumb bit) and args are hex
(`0x…`) or decimal, at most four. Frames must not decrease — probes run
in file order at their frame's boundary. The first Phase 2 corpus
case, `boot-idle`, is a pure replay (no probes): 600 frames, no input,
the pinned-clock boot.

`apricorn-replay [--update] [--engine] [--rom ROM] <case-dir>` runs a
case on the oracle and diffs the fresh trace against the case's
committed `expected.trace` (with the case's own buckets). Exit codes
match `apricorn-diff`. `--engine` runs the case on the engine instead
and diffs *that* trace against the oracle baseline — the
engine-vs-oracle verdict. `--update` regenerates the baseline instead
of comparing — a deliberate, reviewed, oracle-only act (`--update
--engine` is refused); baselines never change implicitly.
`scripts/replay.ps1` / `scripts/replay.sh` wrap the whole flow (build
the bin, run it; `-Engine` / `--engine` pass through) and refuse to
guess when the ROM or the oracle binary is missing.

Cases skip silently when the ROM (or oracle binary)
is absent, so ROM-less CI stays green — the same convention as
apricorn-core's `*_hg` tests. The corpus grows with the phases:
boot-idle (Phase 2), title screen (Phase 3), new-game flow (Phase 4),
map walks (Phase 5), battles (Phase 6), long-play scripts (Phase 8).