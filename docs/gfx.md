# Graphics pipeline — model, rasterizer, presenters

The Phase 3 architecture doc: how a tick becomes pixels, and why the
pipeline is shaped the way it is. The hardware study behind the layer
model is `docs/nds-2d.md` (which cites the vendored NitroSDK headers
and pret usage per fact); this document covers the software built on
it: `apricorn-core::frame` / `app` / `assets`, the `apricorn-gfx`
rasterizer, and the two presenters — the headless dump CLI and the
`apricorn-desktop` shell.

The Phase 3 exit criterion, met: the engine boots from the ROM and
shows the HGSS title screen in a window, with the same pixels the
committed golden hashes certify.

## Topology: desktop → gfx → core

Three crates, one direction of dependency:

```
apricorn-desktop ──→ apricorn-gfx ──→ apricorn-core
(apricorn-harness ──────────────────────↗)
```

* **`apricorn-core`** owns the *model*: the logical frame, input, the
  app contract, and ROM-direct asset loading. Headless, zero rendering
  dependencies, all-integer state. This is where the harness's
  comparison contract lives, so it must never depend on anything
  GPU-shaped.
* **`apricorn-gfx`** is the deterministic CPU rasterizer: a logical
  frame plus an asset source become two 256×192 RGBA8 buffers. Pure
  function, no floating point, no time, no state — the same frame
  hashes the same pixels everywhere, forever. It depends only on core,
  so CI rasterizes and hashes without any GPU.
* **`apricorn-desktop`** is window + present glue only: winit for the
  window and events, wgpu to upload and draw the two buffers. No game
  logic — the pacer decides *when* ticks happen, never *what* they
  compute.

The obvious alternative — model in `apricorn-gfx`, apps in
`apricorn-core` — creates a dependency cycle (core→gfx for types,
gfx→core for asset chunks). The chosen split keeps the future
`apricorn-android` presenter trivially swappable and lets the GPU
stay out of CI entirely (plan Phase 3 risk table).

## The pipeline, per tick

```
Input (keyboard → REG_KEYXY bits)
   │
   ▼
BootChain.tick(frame, input)        apricorn_core::app — deterministic
   │  produces
   ▼
LogicalFrame                       apricorn_core::frame — pure data
   │  rasterized by
   ▼
render(frame, store) → [ScreenBuffer; 2]   apricorn_gfx — engines A/B
   │  mapped through DisplaySelect by the presenter
   ▼
presenter (wgpu window)  or  apricorn-gfx-dump (PNG + SHA-1)
```

Every stage is a pure function of the frame index and the input
sequence. The wall clock appears exactly once, in the desktop pacer,
and only chooses *when* to advance the tick counter — no timing value
ever reaches engine state. That is what makes the golden-hash tests
meaningful: they hash pixels that the harness could reproduce
frame-index by frame-index.

## The logical frame model (`apricorn-core::frame`)

One `LogicalFrame` is the complete observable video state at one tick:
both engines' four BG layers (`BgLayer`), each engine's blend and
master-brightness units, the backdrop colors, and a `DisplaySelect`
saying which engine drives which LCD. No pixel data lives in the
frame — layers reference loaded assets through opaque `AssetId`
handles that the asset store resolves at raster time — so a frame is
plain integers end to end: comparable with `==`, hashable, and
serializable for the harness without touching a ROM.

The field geometry mirrors the hardware study: engines A (MAIN) and
B (SUB), four text BG layers each under `BGxCNT`, one blend unit
(`BLDCNT`/`BLDALPHA`) and one brightness unit per engine. The display
mapping models pret's `screensFlipped` idiom (`GfGfx_SwapDisplay`,
`src/gf_gfx_planes.c:86`) as `DisplaySelect::SubOnTop` — both Phase 3
apps set it, which is why the title logo (engine B content) appears
on the top LCD. The rasterizer is display-agnostic: it renders
engines A and B; the *caller* maps them to LCDs.

## Apps and their assets (`apricorn-core::app`, `::assets`)

An `App` is one scene: `tick` advances on a frame token, `frame`
returns the logical frame the last tick produced, `next` says when
the scene is done. `BootChain` strings scenes in boot order and
cycles after the last (the title's 2340-frame idle timeout returns to
the intro, as the game's own chain does). Each app module's doc
comment maps it to its pret source and lists deferrals.

### Loading: ROM-direct at runtime

`apricorn_core::assets::AssetStore` opens `hg_usa.nds` (SHA-1 pinned
to the known dump constant, as `apricorn-tools verify` does), resolves
a NitroFS path + NARC member, LZ10-decompresses when the magic is
`0x10`, parses with the format decoders, and converts via
`cache::encode_*` *in memory* — the identical code path to the
converter, so there is no cache-format skew. The on-disk cache stays
a tooling artifact; the runtime never reads it.

### The copyright beat (`app::intro_copyright`)

Port of pret `src/intro_movie_scene_1.c` through `WAIT_GAMEFREAK`.
Engine A BG0 (copyright text) scrolls Y 128→0 at the Game Freak logo;
engine B BG1 carries the GF logo, BG0 the blank cover sharing BG1's
char block. Timings: 30-frame hold, 60-frame two-engine alpha fade
(`ev = counter*31/60`, exact integers), 20-frame gap, GF logo,
110-frame hold, skip on A/START/touch after the logo.

Assets (NARC `a/2/6/2`, table in `assets::copyright_beat` with
member-by-member citations):

| Layer | Char (NCGR) | Screen (NSCR) | Notes |
|---|---|---|---|
| MAIN BG0 | 5 (LZ) | 13 | 4bpp; palette `1.pal` |
| SUB BG1 | 4 (LZ) | 12 (LZ) | 4bpp; palette `0.pal` |
| SUB BG0 | — (shares BG1's) | 14 (LZ) | blank cover |

Palettes load 0x140 bytes (160 colors) into each engine's BG slot 0 —
pinned by the ROM test as observationally full for this beat. The
scene's later-appearing layers (sunrise art, members 6/7 and
15–18) are in the table for fidelity but never shown within
copyright-beat scope.

### The title screen (`app::title_screen`)

Port of the statics in pret `src/title_screen.c`: engine B (SUB)
carries all visible art; engine A's BG0 is the deferred Ho-Oh 3D
model (renders as the absent/black layer it is), BG3 is the runtime
"TOUCH TO START" window (cleared window + the 45-frame flash cycle's
frame-indexed state; the text itself is Phase 4). The logo (BG2)
fades in via y-scroll `t/2` and alpha blend ramping `t→31`; the flash
cycle is 45 frames; the idle timeout at frame 2340 returns to the
intro — an honest loop, not a freeze.

Assets (NARC `a/0/4/6`, uncompressed, table in `assets::title_screen`):

| Layer | Char (NCGR) | Screen (NSCR) | Notes |
|---|---|---|---|
| SUB BG1 | 15 | 17 | 4bpp static art |
| SUB BG2 | 3 | 0 | 8bpp — the game logo |
| SUB BG3 | 34 | 35 | 8bpp — version art |

Palettes: `4.pal` → SUB BG slot 0, `13.pal` → MAIN BG slot 0, full
0x200-byte loads. pret also loads `4.pal` into the SUB *extended*
palette RAM for the logo; extended palettes are deferred
(`docs/nds-2d.md`) and with ext mode off the logo reads the regular
load — which is what the model carries.

## The rasterizer (`apricorn-gfx`)

`render(frame, store) -> [ScreenBuffer; 2]`: per engine, screen-entry
decode, tile fetch, scroll wrap per size (256/512 masks),
priority compositing (tie → lower BG index; pixel 0 transparent),
4bpp bank lookup vs 8bpp full-palette lookup, backdrop color, the
plane-mask alpha blend pass (`(first·EVA + second·EBV + 8) >> 4`,
EVA/EBV clamped to 16 — pinned by tests), master brightness last.
No floating point anywhere; all formulas are the integer ones pret or
the SDK uses.

Verified two ways: synthetic-fixture unit tests (hand-computed
pixels for flips, palette indexing, scroll wrap, priority ordering,
blend math, brightness — no ROM needed in CI) and the ROM-dependent
golden hashes below.

**A channel-order bug shipped and was caught here.** `cache.rs`'s
BGR555→RGBA decoder had R and B swapped — it read the format name's
msb-first channel order as a bit order, while the rasterizer's own
backdrop decode (and GBATEK, and the SDK's `GX_RGB` macro) had it
right. Every ROM-derived palette was channel-swapped; the title logo
rendered with yellow and blue exchanged. The unit fixture shared the
wrong belief, so it passed; grayscale art (the copyright screens) was
invariant under the swap, so those golden hashes survived unchanged.
Found by eyeball in the desktop window (2026-09-07), fixed in the
decoder, and all hashes regenerated. The lesson is baked into the
test layout: the golden hashes are the end-to-end pin, but they need
human eyes on real color at least once per pipeline change — which is
what the demo script and the window are for.

## Presenters

### Headless: `apricorn-gfx-dump`

`apricorn-gfx-dump --rom <rom.nds> --frame N[,N..] --out <dir>` —
boots the chain, ticks inputless, rasterizes each requested frame,
writes top/bottom PNGs honoring the frame's `DisplaySelect`, and
prints each screen's SHA-1 over the raw RGBA. `scripts/demo.ps1`
dumps frames `0,45,100,135,200,320,420,1000` into `out/frames/` —
the testable manual check without a window.

### The desktop shell (`apricorn-desktop`, bin `apricorn`)

`cargo run -p apricorn-desktop [--rom <path>]` — winit window +
wgpu present, the Phase 3 exit criterion. The presenter uploads both
screens to two 256×192 `Rgba8Unorm` textures and draws two quads of a
trivial WGSL blit; the layout is the largest integer scale of the
stacked 256×384 pair that fits, centered with black letterbox bars.
The surface format is deliberately the first *non-sRGB* one: the
rasterizer emits display-ready sRGB bytes, and an sRGB target would
re-encode them. `PresentMode::Fifo` (vsync) throttles redraws;
redraws are only requested when a tick produces a new frame.

The quads' UV mapping carries a pinned invariant: the quad's corner
(u,v)=(0,0) is its top-left in NDC *and* row 0 of the buffer — the
two top-lefts coincide, so no V flip belongs in the shader (a
spurious one is exactly how the screens-upside-down bug first
shipped, caught by the same window session as the palette swap).

## Input

The keyboard maps onto the `REG_KEYXY` bits the engine consumes
(physical-key based, so it survives OS layouts):

| Key | Button |
|---|---|
| A / B / X / Y | A / B / X / Y |
| Arrows | D-pad |
| Enter (main/numpad) | START |
| Shift (left/right) | SELECT |
| L / R | L / R |
| Escape | close window |

Touch is deferred with the touch model: the Phase 3 beats skip on
START/A anyway.

## Timestep

The DS refreshes at ~59.8268 Hz (GBATEK) — one tick every
16,714,917 ns, slightly *under* 60. `apricorn-desktop::runner::Pacer`
is the classic accumulator: elapsed wall time banks in a carry and
pays out one tick per period, clamped to 4 per poll (the
spiral-of-death guard — a stalled window catches up over a few
frames, then the debt is discarded: the chain catches up to *now*, not
to the paused interval, exactly as an emulator does). Backwards polls
(clock skew) owe nothing. The winit loop sleeps to the pacer's next
deadline (`ControlFlow::WaitUntil`) instead of spinning.

The engine never sees the clock: ticks are pure functions of the
frame index, so replaying a frame range reproduces the same logical
frames in the harness as the window showed live.

## Deferred (with where each lands)

* **3D engine / NSBMD** (title's Ho-Oh BG0, camera pan) → Phase 6;
  renders as the absent layer it is in the model.
* **Text renderer** ("TOUCH TO START" window content) → Phase 4.
* **OAM/sprite rendering** (modeled in `docs/nds-2d.md`, not
  rasterized) → with the first scene that needs sprites.
* **Intro movie scenes 2–5** (sprites, circle wipe, 3D) → their
  subsystem phases; Phase 3 ships the copyright beat only, per the
  scope decision.
* **Touch input** → with the touch model (first consumer is the
  title screen's "TOUCH" prompt, Phase 4+).
* **Audio** → the SDAT phases.
* **GPU compositing** — the CPU-rasterize/wgpu-present decision is
  deliberate (determinism first); a GPU renderer can replace the
  rasterizer behind the same `render` contract later, non-breaking.

## Equivalence story

Phase 3's parity instrument is the **golden raster hash**:
`tests/raster_hg.rs` (ROM-dependent, skip-silent) boots the chain and
SHA-1s both screens at the corpus frames — the copyright beat
(0/45/100/135/200) and the title statics (320/420/1000) — committing
**hashes only, never pixels**. The hashes pin the whole pipeline
end-to-end: loader, decoder, frame model, scene logic, rasterizer.
Because a `LogicalFrame` is pure data, the same discipline extends
upward: from Phase 4 the harness can hash logical-frame regions into
the trace corpus alongside the RAM-watch regions, giving
divergence-pointing comparison *below* pixels too.

Oracle visual comparison (rendering the melonDS oracle's frames and
diffing against ours) is explicitly **out of scope for now**: the
hash-pinned deterministic pipeline plus human review of the demo
output and the window is the agreed bar for Phase 3. When visual
diffing becomes worth its cost (GPU rendering, subtle blend cases),
the oracle's framebuffer becomes one more trace source.

## Versions

Pinned in `Cargo.lock` (the plan's wgpu/winit drift risk): wgpu
30.0.1, winit 0.30.13, pollster 1.0.1. The wgpu surface API is small
and isolated in `presenter.rs`; drift lands as compile errors there,
not as silent behavior changes.