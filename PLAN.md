# apricorn — Pokémon HeartGold recompilation in Rust

**Goal:** a 1:1, behaviorally-equivalent Rust reimplementation of Pokémon HeartGold
(US) that loads the original `hg_usa.nds` at runtime for all assets, targets
Windows / Linux / macOS / Android, and is architected to be extended beyond the
original game once parity is reached.

**Definition of 1:1 (phase one):** *behavioral equivalence*, not
instruction-matching. Same RNG outcomes, same formulas, same save format,
same logical visuals. Internals are idiomatic Rust.

**Key references:**
- `pret/pokeheartgold` — WIP matching decompilation in C. Builds byte-identical
  ROMs, but much is still labeled assembly. Used as *partial documentation*,
  never as the sole source of truth.
- `hg_usa.nds` — the binary itself is the ultimate oracle.
- Instrumented headless melonDS — the comparison oracle for testing.

---

## Guiding principles

1. **The ROM is the spec.** Anything pret hasn't decompiled, we verify against
   the original binary via the differential-testing harness (Phase 2).
2. **Test infrastructure is a first-class deliverable.** Parity is *demonstrated*,
   not asserted. Every subsystem ships with its equivalence tests.
3. **Engine vs. game vs. platform are strictly separated.** The engine
   (`apricorn-core`) must run headless so the test harness can drive it; platform
   glue (`apricorn-desktop`, `apricorn-android`) contains no game logic.
4. **No copyrighted material in the repo.** The ROM and extracted assets are
   never committed; each developer supplies their own dump (verify SHA1
   `4fcded0e...` per pret's README). Only our code, tooling, and test *traces*
   (hashes, not content) live in git.
5. **Fixed-timestep, deterministic game loop** (59.8268 Hz NDS VBlank timing
   model) from day one — determinism is what makes behavioral testing possible.

---

## Phase 0 — Scoping & research foundation

Get everything in place so later phases don't stall on unknowns.

- [x] Clone `pret/pokeheartgold`; inventory what's decompiled (C) vs. still
      assembly; map which subsystems (battle, scripts, save, RNG) fall in each
      bucket. → `docs/pret-inventory.md`
- [x] Verify our ROM dump matches the known US HeartGold SHA1.
      → `apricorn-tools verify`, asserted in `tests/nds_hg.rs`
- [x] Rust workspace scaffolding: crates `apricorn-core` (headless engine),
      `apricorn-tools` (asset pipeline), `apricorn-desktop`, later
      `apricorn-android`. (CI moved to Phase 9, where the platform targets
      exist.)

**Exit:** workspace builds and tests green; ROM verified; pret coverage
mapped. (The oracle moved to Phase 2 and the hardware study to Phase 3,
where their knowledge is first needed.)

---

## Phase 1 — ROM tooling & asset pipeline

Everything needed to read the ROM into usable engine data. Pure Rust, heavily
unit-tested, no engine dependency.

- [x] NDS container parser: header, file allocation table, NitroFS reading.
      → `apricorn-core::nds`, `docs/nds-container.md`
- [x] Archive & format parsers — the big one (sub-items in rough
      dependency order; cross-check each against Tinke/mkds-style docs
      and pret's headers):
      - [x] NARC (archives) → `apricorn-core::formats`, `docs/narc.md`
      - [x] NCGR/NCLR/NSCR (BG tiles/palettes/maps)
            → `apricorn-core::formats`, `docs/nitro-gfx.md`
      - [x] NANR/NCER (sprites + animation)
            → `apricorn-core::formats`, `docs/nitro-sprite.md`
      - [x] BTX (textures) → `apricorn-core::formats`, `docs/nitro-btx.md`
      - [x] Message/text banks (MAT) → `apricorn-core::formats`,
            `docs/nitro-msg.md`
      - [x] SDAT (audio — parse now, play later) → `apricorn-core::formats`,
            `docs/nitro-sdat.md`
- [x] Extraction tool: ROM → unpacked tree + manifest (hashes, versioning).
      → `apricorn-tools extract`, `docs/extraction.md`
- [x] Conversion step: raw formats → engine-friendly cached formats
      (e.g., tile sheets + palettes + collision metadata) so the runtime
      loader stays fast and simple.
      → `apricorn-core::cache`, `apricorn-tools convert`,
      `docs/conversion.md`
- [x] Round-trip tests: re-serialize and byte-compare against the originals
      (guards against parser bugs).
      → `apricorn-core::formats` (`to_bytes` per format),
      `tests/roundtrip_hg.rs`, `docs/roundtrip.md`

**Exit:** `apricorn-tools extract hg_usa.nds out/` produces a full, verified,
engine-loadable asset tree.

---

## Phase 2 — Behavioral-equivalence harness

The testing methodology that every later phase leans on. Build it *before*
game features.

- [x] Build the headless melonDS oracle (moved from Phase 0): fixed RTC,
      scripted input injection, RAM-watch hooks that emit traces.
- [x] `arm-runner`: ARM9 interpreter harness (test-only, not shipped) that
      loads original overlays and calls original functions with controlled
      inputs — a per-function oracle that works even where pret has no C.
- [x] Trace format + state-dump protocol: every N frames dump hashes of
      game-state regions (RNG state, party, position, flags, map) from both
      oracle and our engine.
- [x] Input-script format (frame N: buttons / stylus) executable by both the
      oracle and headless `apricorn-core`.
- [x] `apricorn-diff` comparator: pinpoints first divergence between two traces,
      with hard-equality vs. may-drift state buckets (animation counters etc.).
- [x] Regression corpus: growing library of input scripts + expected traces,
      run in CI.

**Exit:** a single script replays a test input in both oracle and engine and
prints "EQUIVALENT" or the exact frame/state of divergence. — Done:
`scripts/replay.ps1 corpus/boot-idle` (or `.sh`) prints EQUIVALENT, or
the exact frame/region with both hashes. The "engine" side today is
arm-runner per-function probes (`tests/equivalence_hg.rs`); the real
apricorn-core replays through the same corpus machinery in Phase 4+.

---

## Phase 3 — Engine scaffold & first pixels

Minimal real-time engine; the "it draws something" phase.

- [x] NDS hardware study (moved from Phase 0): 2D engine — BG modes, OAM
      sprites, affine — plus NitroSDK conventions, learned as needed to
      model the layers correctly.
      → `docs/nds-2d.md`; OAM and affine modeled only (rendering deferred),
      extended palettes deferred (both Phase 3 apps configure none).
- [x] winit + wgpu renderer with an NDS-style logical layer model:
      two screens, BG layers, sprite/OAM layer, palettes — so *logical frame*
      comparison (draw-lists, not pixels) is possible from the start.
      → `apricorn-core::frame` (pure-data model), `apricorn-gfx` (deterministic
      CPU rasterizer, no GPU in CI), `apricorn-desktop` (winit 0.30.13 +
      wgpu 30.0.1 presenter) — topology and rationale in `docs/gfx.md`.
      OAM layer is modeled but not rasterized (first sprite scene adds it).
- [x] Fixed-timestep loop, input abstraction (keyboard/gamepad now, touch later).
      → `apricorn-desktop::runner::Pacer` (16,714,917 ns/tick accumulator,
      spiral-of-death clamp, clock-skew immune; paces only, never feeds
      state), `apricorn-core::input` (`REG_KEYXY` bits; touch type present,
      input deferred). Gamepad deferred.
- [x] Decode and display: title screen statics, intro movie, copyright screen —
      first end-to-end milestone using real assets.
      → `apricorn_core::app` (`intro_copyright`, `title_screen` — ports of the
      pret scenes), golden raster hashes in `tests/raster_hg.rs` (hashes
      only), `apricorn-gfx-dump` + `scripts/demo.ps1` for headless review.
      Honest scope: the intro movie is the **copyright beat only** (scenes
      2–5 need sprites/3D — their phases); the title's 3D BG0 renders as the
      deferred layer it is; "TOUCH TO START" text is Phase 4.

**Exit:** engine boots from extracted assets and shows the HGSS title screen.
— Met 2026-09-07: `cargo run -p apricorn-desktop` plays the copyright beat,
Game Freak logo, and the title screen from the real ROM; `tests/raster_hg.rs`
pins the same pixels by SHA-1 (details: `docs/gfx.md`).

---

## Phase 4 — Core game state & data

- [x] Personal/base-stat tables, species/moves/items data (from ROM tables —
      zero hand-typing).
      → `apricorn-core::data` (`GameData::load` parses personal/growtbl/
      waza/item_data/wotbl/evo/pms straight from the ROM, strict-layout
      validated), `tests/data_hg.rs`, `docs/game-data.md`
- [x] Message/text system decoding, with the game's variable formatting.
      → `apricorn-core::text`: committed generation charmap
      (`apricorn-tools gen-charmap`, cross-checked against pret's
      `charmap.txt` by `tests/charmap_hg.rs`), strict decode of all
      49,984 messages (charmap chars, `0xFFFE` control blocks, packed
      TRNAME) with byte-identical reassembly, `GameString` +
      `String16_FormatInteger` quirks, and `MessageFormat` placeholder
      expansion (`tests/text_hg.rs`, `docs/text.md`)
- [x] RNG implementation (LCG) + differential-tested against `arm-runner`.
      → `apricorn-core::rng` (`Lcrng` — LCRandom/PRandom/LCRandRange
      and the mon-encryption recurrence), `tests/rng_hg.rs`
      (apricorn-harness: engine vs the original pinned functions,
      draw by draw, ROM-gated), `docs/rng.md`
- [x] Save format: read + write original HGSS save blobs, checksums included;
      a real retail `.sav` must load correctly.
      → `apricorn-core::save` (the 512-KiB card container: dual slot
      mirrors, 42-block layout computed by the original's own boot
      arithmetic, chunk/footers/block/extra-chunk CRC-16-CCITT,
      `Save_GetSaveFilesStatus` probe with the counter-wraparound
      quirk, `save_game`'s alternating-slot write, `ReadExtraSaveChunk`
      selection), ROM-measured size tables (45 pinned size stubs
      called via arm-runner, `tests/save_hg.rs`), byte-identical
      round-trip + status-matrix tests on synthesized retail-shaped
      blobs plus a retail `hg.sav` gate (`tests/save.rs`),
      `docs/save.md`
- [ ] Game-state machine: boot → title → new game → Oak intro → name entry.

**Exit:** new-game flow up to landing in the player's bedroom; original `.sav`
files load and save back byte-identically.

---

## Phase 5 — Overworld

- [ ] Map engine: HGSS's map/BG layers, collision, warp/door transitions,
      camera.
- [ ] Player movement (grid + HGSS's smooth sub-tile animation), running shoes,
      bicycle.
- [ ] NPC system, interaction radius, dialogue UI boxes.
- [ ] Scripting/event engine: HGSS's script VM (flags, vars, triggers) —
      reverse from asm where pret is incomplete; differential-test with
      `arm-runner`.
- [ ] Menus: start menu, bag, party screens — including touch-screen versions.
- [ ] Day/night cycle & palette tinting (pinned-clock aware).

**Exit:** free-roam Johto with NPCs, doors, dialogue, and correct day/night —
validated by replay traces against the oracle.

---

## Phase 6 — Pokémon & battle systems

The largest single phase; break into sub-milestones.

- [ ] Party, PC boxes, bag/items, Pokédex data structures.
- [ ] Wild encounters: encounter tables, RNG-driven selection, shiny rolls.
- [ ] Battle core: turn order, damage/stat/status formulas (each
      differential-tested against original ARM functions), type chart,
      abilities, held items, AI.
- [ ] Battle UI: the 3D-modeled battle scenes (NDS 3D pipeline usage,
      studied here — moved from Phase 0), move animations, HP bars,
      text flow — via logical-frame comparison.
- [ ] Catching mechanics, experience/leveling, EVs/IVs, evolution.
- [ ] Eggs, breeding basics (as used by main story).

**Exit:** full trainer/wild battles behave identically under replay traces;
stat/damage/RNG tests green in CI.

---

## Phase 7 — Audio

- [ ] SDAT playback: SSEQ (sequences), SWAR/STRM (samples/streams) — either a
      Rust synth or wrap an existing playback core; hooked into game events.
- [ ] Jingle/mixer behavior matching original channel usage so audio traces
      (sequence position per frame) can join the equivalence harness.

**Exit:** soundtrack, cries, and SFX play correctly throughout the flows of
Phases 5–6.

---

## Phase 8 — Fidelity hardening & completion

- [ ] Long-play regression: extended replay scripts covering gyms, rival
      battles, legendaries, Kanto access, Elite 4 → credits.
- [ ] Save byte-equality across every corpus script (the gold-standard test).
- [ ] RNG-state trace equality at every checkpoint.
- [ ] Side content in parity order: Pokegear, radio, Bug Catching Contest,
      following Pokémon, Pokéathlon — each as its own mini decompile-and-verify
      cycle.
- [ ] System-test the "may-drift" buckets down to near-zero divergence.

**Exit:** a full playthrough on `apricorn` mirrors the original end to end,
verifiable from the corpus.

---

## Phase 9 — Platforms & packaging

- [ ] `apricorn-desktop`: Windows/Linux/macOS builds in CI, ROM-file picker.
- [ ] `apricorn-android`: `cargo-ndk` build, Android lifecycle handling
      (pause/resume preserving emulator-style state), scoped-storage ROM
      loading, touch controls + optional on-screen gamepad layout
      (the touch menu UI ports almost 1:1).
- [ ] Release packaging, crash reporting, and save-import/export UX.

**Exit:** installable artifacts for all four targets.

---

## Phase 10 — Beyond 1:1

Only after parity, in whatever direction we choose — QoL options, modern
resolutions/framerates, modding/scripting hooks, new content, multiplayer
experiments. The Phase 2 corpus is what makes these *safe*: every extension
must keep the parity tests green unless deliberately opted out.

---

## Suggested repo layout

```
apricorn/
├─ crates/
│  ├─ apricorn-core/        # headless game engine + logic (no platform code)
│  ├─ apricorn-tools/       # ROM/asset pipeline CLI
│  ├─ apricorn-harness/     # oracle glue, arm-runner, trace diff, replay runner
│  ├─ apricorn-gfx/         # wgpu renderer (logical NDS layer model)
│  ├─ apricorn-audio/       # SDAT playback
│  ├─ apricorn-desktop/     # winit app
│  └─ apricorn-android/     # Android shell
├─ corpus/                  # input scripts + expected trace hashes
├─ docs/                    # reverse-engineering notes per subsystem
└─ PLAN.md
```

## Long-lead risks to watch

- **pret incompleteness** — battle & scripting internals may still be asm;
  budget real time for `arm-runner`-based verification there.
- **Script VM scale** — HGSS's event scripts are huge; parity is per-map
  grind work, not one big unlock.
- **3D battle scenes** — the only place the NDS 3D pipeline matters; consider
  deferring visual (not behavioral) parity for it.
- **Scope creep in Phase 10** — parity first.