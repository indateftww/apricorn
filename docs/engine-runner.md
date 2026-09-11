# The engine runner — `apricorn-core` as a trace producer

Reference for `apricorn_harness::engine`, the `apricorn-run` binary,
and `apricorn-replay --engine` (Phase 4). This is the Phase 2 exit
promise redeemed: the real engine replays a corpus case through the
same script, the same regions, and the same comparator as the oracle,
and the verdict is measured, not asserted. `docs/equivalence.md`
defines the formats; `docs/oracle.md` the other producer.

## Usage

```text
apricorn-run --rom <rom.nds> --input <script.apin>
             [--save <card.sav>] [--regions <regions.conf>]
             [--trace <out.trace>] [--png <frame,frame,...>]
             [--out <dir>] [--frames <N>]
```

* Boots `Game::new` on the ROM with the script's `rtc` (or the
  oracle's default `2010-03-01T09:00:00` when the script pins none)
  and the card backup from `--save` (a blank card without it), then
  ticks frames `0..end` with the script's input — `--frames` overrides
  the length; frames past the script's `end` get idle input, exactly
  as the oracle's input blob does.
* `--png` writes the listed frames as `frame_%06d_top.png` /
  `frame_%06d_bottom.png` (the frame's display select honored, the
  `apricorn-gfx-dump` idiom) into `--out`, else `$APRICORN_RENDER_OUT`,
  else `out/apricorn-run` — all gitignored — and prints each screen's
  SHA-1 over the raw RGBA, the golden tests' digests.
* `--trace` writes the engine trace over `--regions` (no regions: a
  header-only trace). `apricorn-diff --regions regions.conf
  oracle.trace engine.trace` compares it against an oracle trace of
  the same case.
* The end state is printed: frames run, `GameState`, the LCG state,
  the MT cursor, and the player's identity once Oak has confirmed one.
* Exit codes: 0 ok, 1 runtime error, 64 usage — the harness's
  conventions.

The corpus form:

```text
apricorn-replay --engine corpus/boot-idle        # engine trace vs the oracle baseline
scripts/replay.ps1 corpus/boot-idle -Engine      # (or scripts/replay.sh … --engine)
```

prints `EQUIVALENT` or the first divergence (frame, region, both
hashes) and exits 0/1/64 like the oracle mode. `--update` stays an
oracle-only act: `--update --engine` is refused — the engine never
defines correct.

## Input: `.apin` → `apricorn_core::input::Input`

One state machine (`InputScript::at(frame)`) feeds both producers.
The engine mapping is bit-for-bit — both tables use the hardware
`REG_KEYXY` layout with **bit set = held** (the register is active-low;
each producer inverts at its own edge):

| `.apin` button | `key::*` bit | value |
|---|---|---|
| `A` | `key::A` | `1<<0` |
| `B` | `key::B` | `1<<1` |
| `SELECT` | `key::SELECT` | `1<<2` |
| `START` | `key::START` | `1<<3` |
| `RIGHT` | `key::RIGHT` | `1<<4` |
| `LEFT` | `key::LEFT` | `1<<5` |
| `UP` | `key::UP` | `1<<6` |
| `DOWN` | `key::DOWN` | `1<<7` |
| `R` | `key::R` | `1<<8` |
| `L` | `key::L` | `1<<9` |
| `X` | `key::X` | `1<<10` |
| `Y` | `key::Y` | `1<<11` |

`<frame> touch x y` → `Input.touch = Some(Touch { x, y })` (raw 0–255
per axis, the resolution the script carries); `lift` → `None`. The two
tables are cited, not shared (the core keeps zero dependencies), and
`engine.rs`'s unit test locks them name by name.

The `rtc` line is `YYYY-MM-DDTHH:MM:SS` (the oracle's `--rtc` form) and
becomes `RtcDateTime` field for field, with the SDK's `RTCWeek` day of
week derived (`RTC_WEEK_SUNDAY` = 0).

## Regions: pin names → engine state

The oracle hashes memory at the pinned addresses of `regions.conf`; the
engine has no memory image, so it resolves each region by its **pin
name** to the engine state that models that static, serialized as the
ROM lays it out (`src/math_util.c:9-11`), so the hashes are comparable
byte for byte:

| region | engine source | bytes |
|---|---|---|
| `sLCRNG_State` | `Game::lcrng().seed()` | 4 — one little-endian `u32` |
| `sMTRNG_State` | `Game::mtrng().state_words()` | 2496 — the 624 little-endian `u32` words; the cursor is *not* part of it |
| `sMTRNG_Cycles` | `Game::mtrng().cycles()` | 4 — one little-endian `int` (`.data`, initializer 625) |

Any other name — or a known name with a size other than its layout's —
is refused before the ROM is opened, with the region named
(`engine::check_regions`). The table grows with the engine.

The engine samples after `Game::tick`, as the oracle samples after
`NDS::RunFrame()`: every region whose `frame % sample == 0`, regions in
file order — the same record sequence. The trace header is the
oracle's: `producer engine-apricorn-core`, `rom-sha1` over the whole
ROM file, `input-sha1` / `regions-sha1` over the canonical texts,
`frames`, `frame-rate 59.8268`, `rtc`. `tests/engine_hg.rs` asserts
the gates and the schedule equal the committed case's.

## The seed investigation (corpus/boot-idle)

The question: the engine's `Game::new` seeds both generators with
`RngSeedFromRTC()` at vblank counter 0 (`InitializeMainRNG`,
`src/main.c:90`, before the loop) — is that what the ROM does, and
when? Method: brute-force the seed model against the oracle's
committed hashes (SHA-1 of the four LE bytes of `sLCRNG_State`, of the
2496 bytes of `sMTRNG_State`), for vblank counter values 0..=600 and
RTC seconds 0..=15, then a second oracle run watching all three RNG
statics every frame for 1200 frames.

What the oracle's boot-idle trace actually contains:

| frames (VBlanks from power-on) | `sLCRNG_State` | `sMTRNG_State` | `sMTRNG_Cycles` |
|---|---|---|---|
| 0–184 | zero (bss) | zero (bss) | 625 (`.data` initializer) |
| 135–184 | — | fresh-image first draw (625 → `SetMTRNGSeed(5489)`, one twist) | 2 |
| 185–242 | **`0x0309000A`**, no draw | **`SetMTRNGSeed(0x0309000A)`** | 624 |
| 223 | — | — | 625 again: the image was reloaded |
| 243– | zero (bss cleared again) | zero | — |
| 407–464 | `0x0709000A` | `SetMTRNGSeed(0x0709000A)` | 624 |
| 629, 851, 1073, … | seeds 3–4 RTC seconds apart, period 222 frames | | |

Findings:

1. **The seed value is exactly the engine's model.** `0x0309000A` is
   `RngSeedFromRTC()` for `2010-03-01T09:00:00` with the vblank counter
   at **0** — `10 + 3·0x100·1·0x10000 + 9·0x10000 + 0`. No other
   counter value in 0..=600 matches either hash; the LCG holds the raw
   seed (no draw) and the MT holds `SetMTRNGSeed(seed)` with its cursor
   at 624. `Game::new`'s seed is correct to the bit and stays as it is.
2. **The seed frame is VBlank 185 on the first boot** (power-on →
   crt0 decompression → `SaveData_New` → `InitializeMainRNG`), 407 on
   the second, 629 on the third — one boot every 222 frames. The two
   `MTRandom` draws at ~135 are `SaveData_New`'s blank-card
   `Save_InitDynamicRegion` (the roamer init) on the fresh generator,
   erased by the seed that follows; the engine draws them at
   `NewGameInit` instead, where they matter.
3. **The baseline is a soft-reset loop, not an idle title screen.**
   `sMTRNG_Cycles` returning to its `.data` initializer 625 and both
   bss regions re-zeroing 20 frames later is an ARM9 image reload —
   `OS_ResetSystem`. The cause is at the oracle's edge: melonDS's
   `KeyInput` is active-low (reset value `0x007F03FF`; its frontend
   keeps the mask at `0xFFF` and *clears* a bit on press), but
   `apricorn-oracle.cpp` passes the harness's bit-set-equals-held
   mask straight into `NDS::SetKeyMask`. An idle script therefore
   holds all twelve buttons, and L+R+START+SELECT is `NitroMain`'s
   soft-reset combo (`src/main.c`, the loop's first check) —
   `DoSoftReset` → `OS_ResetSystem` about 38 frames after the seed,
   every boot. The intro overlay's init (which would set the LCG to 0,
   `intro_movie.c:72`) never runs. The fix belongs to the oracle patch
   (`oracle/patches`: invert the mask, `0xFFF & ~keymask`), after
   which `expected.trace` must be regenerated with `--update` and the
   verdict below re-measured. Phase 2's `equivalence_hg.rs` probes run
   at frame 20, before the loop reads keys, so they were unaffected.
4. **Engine vs oracle on boot-idle: DIVERGED at frame 0,
   `sLCRNG_State`** — the oracle holds zeroed bss (its frame axis
   starts at power-on, 185 VBlanks before the seed), the engine holds
   the seed (its frame 0 is the intro's first tick). The engine's
   frame-0 MT hash equals the oracle's frames 210–269 hash. No clean
   change at `Game::new` or the re-seed points can make this case
   EQUIVALENT: the frame axes differ by the boot latency, and the
   baseline never enters the game. `tests/engine_hg.rs` pins exactly
   this divergence (and guards the pin against a regenerated
   baseline), together with ROM-less locks that the engine's region
   bytes hash to the oracle's recorded digests for the same values.

What this leaves open, honestly: a boot-idle baseline captured by a
key-mask-corrected oracle would hold zero through ~184, the seed at
185, then the intro's `SetLCRNGSeed(0)` on the intro overlay's init
and its restore at exit (`intro_movie.c:71-72`, `:160`) — a frame
offset (the boot latency) and one intro-overlay RNG effect the engine
does not yet model. Both are engine-side follow-ups once the oracle
edge is fixed; neither is a seeding bug.
