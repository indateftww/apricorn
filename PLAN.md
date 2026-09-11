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
- [x] Game-state machine: boot → title → new game → Oak intro → name entry.
      Functional milestone accepted after user regression testing on
      2026-09-11. Exact parity follow-ups are tracked in Phase 8.
      - [x] Pinned RTC clock (`GF_InitRTCWork` — the deterministic
            time Oak's greeting and `InitializeMainRNG` read).
            → `apricorn-core::rtc`
      - [x] The state machine itself: the main-overlay chain
            (`NitroMain`'s `RegisterMainOverlay` hand-offs), the ov36
            re-seed points, the card-parse routing onto the status
            flags. → `apricorn-core::app::game`
      - [x] Font asset + message-window text rendering in the frame
            model (TextPrinter: per-frame speeds, wait states, down
            arrows, `{YESNO 0}` focus blocks) and the palette-fade
            manager (the master-brightness fades the scenes run).
            → `apricorn-core::font`, `apricorn-core::app::text`,
            `apricorn-core::app::fade`
      - [x] Save-check scene: the status-flag warnings ("corrupted",
            "erased") and the fade to the menu.
            → `apricorn-core::app::check_save`
      - [x] Main menu scene: the save-aware button list, key/touch
            input, the screen scroll, and the new-game confirmation
            dialog. → `apricorn-core::app::main_menu`
      - [x] Oak intro speech (`src/oaks_speech.c`): the info tutorial
            menu, control/adventure info screens, the time-of-day
            greeting, the Oak-pic slide, the gender pick, the naming
            handoff, the shrink anim. → `apricorn-core::app::oak_speech`
      - [x] Naming screen (`src/naming_screen.c`): the on-screen
            keyboard name entry Oak launches (nested overlay).
            → `apricorn-core::app::naming`: ROM-loaded uppercase/lowercase/
            symbol pages, pad/touch input, deletion and seven-character
            limit, page slides, default-name RNG selection; confirmed
            identity retained by `Game`. `tests/naming_hg.rs` in core/gfx.
      - [x] Landing: desktop runs `Game` with mouse-to-stylus input,
            `docs/game-flow.md`, machine-walk tests through real Oak/name
            entry (no injected result), and renderer regressions for OBJ
            palettes, transparent text blits and the name-box window mask.
      - [x] Naming's palette glow, bar wiggle and BACK/OK press effects;
            ROM animation/palette data with behavior regressions.
      - [x] Structured new-game and post-Oak save initialization:
            all 42 block defaults, money/position/flags, trainer ID/avatar,
            Safari areas, friend mail and Pokewalker seeds. Defaults are
            checked against original ARM initializers under fixed external
            inputs; post-Oak RNG order and card round trips are tested.
            → `save::new_game`, core/harness `tests/new_game_hg.rs`.
      - [x] Rendered bedroom landing: map 64's real land model, eight
            furniture placements, textures and the chosen player character.
            → `core::field`, `gfx::field`, `tests/bedroom_hg.rs`.
      - [x] User regression playtest and Phase 4 acceptance before commit.
            Oak's missing Poké Ball corrected and its render verified;
            save validation covered by automated tests.

**Exit:** new-game flow up to landing in the player's bedroom; original `.sav`
files load and save back byte-identically.
— Flow initializes the new save state and reaches a rendered static bedroom
with the confirmed name/gender. Movement, room scripts and field menus are
Phase 5. Phase 4's functional milestone is accepted; this is not a claim of
full original-scene equivalence. Retail save-container round trips pass,
and exact parity follow-ups remain explicit in Phase 8.

---

## Phase 5 — Overworld

Parallel workstreams (2026-09-11 →): each lands as its own reviewed
branch with ROM-gated tests, then the orchestrator wires them into
`app::game`. Sub-items are checked when merged, not when started.

**Status 2026-09-11 (midday):** the foundations are merged and green
(data layer, 3D field rendering, movement machine, script VM core, start
menu, day/night model, engine runner, oracle screenshots). The bedroom is
still a frozen frame in `app::game`: the **field system integration** —
the per-frame loop that moves the player against the terrain, follows
with the camera, animates the map-object billboard and runs the
stairs/door warps (bedroom ↔ house 1F ↔ New Bark Town) — is in progress
on its own branch (`field::system`, `docs/field-system.md`) and is the
next objective to finish before stopping to assess. After it: NPC objects
and the script host (dialogue boxes, init scripts, mom's cutscene), the
start-menu/day-night hookups, then the user's regression test of Phase 5.
The text-box "defects" seen in engine screenshots (page-wait boxes, grey
right column) were measured against retail frames and are retail
behavior; they are pinned by tests now (`docs/game-flow.md`). Phase 6.1
(Pokémon data) and Phase 7 (audio) have partial branches parked unmerged.

- [ ] Map engine: HGSS's map/BG layers, collision, warp/door transitions,
      camera.
      - [x] Field data layer: map headers (ARM9 table, pinned), matrices,
            land data (attributes/props/model/BDHC/extra), area data,
            terrain attributes + collision bits, map events, script
            headers, overlay-1 tables (camera presets, sprite→model).
            → `apricorn-core::field::{map_header,matrix,land,area,terrain,
            events,script_header,ov01}`, `docs/field-data.md`
      - [x] Field rendering: the 3D field composited as engine A's BG0
            under the 2D layers/OBJ/windows, camera presets (perspective
            and orthographic, SDK fixed-point angles), prop transforms,
            map-object billboards with the original projection shear.
            → `apricorn-gfx::field`, `docs/gfx.md`
      - [ ] Field system integration (in progress): the live per-frame
            field in `app::game`, warps/doors between maps, the camera
            follow, the animated player billboard, and the 3 × 3 cell
            window of `FieldScene::load`. → `field::system`,
            `docs/field-system.md`
- [ ] Player movement (grid + HGSS's smooth sub-tile animation), running shoes,
      bicycle.
      - [x] Movement command machine (113 commands; linear steps at
            0x800/0x1000/0x2000/0x4000/0x8000 per frame, turns, END) and
            `PlayerAvatar_MoveControl`, differential-tested against the
            original ARM9 step functions via arm-runner.
            → `apricorn-core::field::{map_object,avatar,input}`,
            `docs/field-movement.md`
      - [ ] Bicycle, ledges/jumps, surf — later slices.
- [ ] NPC system, interaction radius, dialogue UI boxes.
- [ ] Scripting/event engine: HGSS's script VM (flags, vars, triggers) —
      reverse from asm where pret is incomplete; differential-test with
      `arm-runner`.
      - [x] VM core: 3 contexts, 20-deep stack, u16 opcodes, bank
            mapping, init-script dispatch, typed flags/vars over save
            block 4, host trait; the opcode subset used by the early
            game (std init, bedroom, house, New Bark, Route 29, Elm).
            → `apricorn-core::script`, `save::vars_flags`,
            `docs/script-vm.md`
- [ ] Menus: start menu, bag, party screens — including touch-screen versions.
      - [x] Start menu port (`src/start_menu.c` + overlay 27's touch-LCD
            icon grid) as a host-driven scene component with ROM-gated
            render goldens; the launched apps and the unselected-icon
            OAM dimming are deferred. → `apricorn-core::app::start_menu`,
            `docs/menus.md`
- [ ] Day/night cycle & palette tinting (pinned-clock aware).
      - [x] Advancing frame-indexed RTC model (melonDS's 32768 Hz tick
            over 560190-cycle frames), time-of-day buckets (differential
            vs `GF_RTC_GetTimeOfDayByHour` and the SDK date converters),
            the five area-light text archives, prop time-of-day visual
            state. → `apricorn-core::rtc`, `field::{lighting,time_state}`,
            `docs/day-night.md`
      - [ ] Apply the area-light template to the field renderer.

Test infrastructure landing with this phase (Phase 2's promise):

- [x] Engine trace producer + headless scripted runner (`apricorn-run`:
      replay an `.apin` through `Game`, dump PNGs, emit a trace the
      comparator checks against the oracle; `apricorn-replay --engine`).
      → `apricorn-harness::engine`, `docs/engine-runner.md`.
      Finding: the engine's boot seed value is exact (vblank counter 0,
      matching the oracle's post-seed LCRNG/MT hashes), but the
      committed boot-idle baseline was a soft-reset loop — the oracle
      passed the harness's bit-set-equals-held mask into melonDS's
      active-low `SetKeyMask`, so an idle script held L+R+START+SELECT.
      The polarity fix and regenerated baseline land with the oracle
      screenshot work below; the boot-latency frame offset (retail
      seeds at VBlank 185) is then the remaining engine-side gap.
- [x] Oracle screenshots (`--shots`) and `corpus/new-game`: the real ROM
      driven from boot to the bedroom, with milestone frames recorded
      for engine-vs-ROM visual comparison. The oracle's key-mask polarity
      is fixed and `corpus/boot-idle` regenerated (a single clock seed at
      frame 185, the intro's `SetLCRNGSeed(0)` at 186); both cases replay
      EQUIVALENT. → `corpus/new-game/README.md`, `docs/oracle.md`

**Exit:** free-roam Johto with NPCs, doors, dialogue, and correct day/night —
validated by replay traces against the oracle.

---

## Phase 6 — Pokémon & battle systems

The largest single phase; break into sub-milestones.

- [ ] Party, PC boxes, bag/items, Pokédex data structures.
      - [ ] Pokémon encryption/shuffle/checksum, party, player data,
            bag and Pokédex views over the real save blocks, verified
            on the retail save and against the original segment
            crypt via arm-runner. → `apricorn-core::pokemon`,
            `save::{player_data,bag,pokedex}`, `docs/pokemon.md`
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
      - [ ] Offline deterministic renderer (SSEQ sequencer + SBNK/SWAR
            synth + 16-channel mixer, hash-pinned PCM) as the new
            `apricorn-audio` crate. → `docs/audio.md`
- [ ] Jingle/mixer behavior matching original channel usage so audio traces
      (sequence position per frame) can join the equivalence harness.

**Exit:** soundtrack, cries, and SFX play correctly throughout the flows of
Phases 5–6.

---

## Phase 8 — Fidelity hardening & completion

- [ ] Phase 4 parity follow-ups: original-ROM frame/state comparisons for
      Oak/naming, including nested overlay/fade timing, outstanding Oak
      sprite waits/yes-no cursor, and post-Oak whole-region comparison.

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