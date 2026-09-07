# NDS 2D engine — the hardware Phase 3 models

The study backing `apricorn-core::frame` / `apricorn-gfx` (PLAN.md Phase 3).
Sources, in order of authority:

1. The vendored NitroSDK headers in `refs/pokeheartgold/lib/include/nitro/`
   — register layouts (`hw/ARM9/io_reg.h`), enums and inline register
   functions (`gx/gx.h`, `gx/g2.h`, `gx/gx_bgcnt.h`, `gx/g2_oam.h`,
   `gx/gx_vramcnt.h`).
2. pret's own usage (`src/bg_window.c`, `gf_gfx_planes.c`, `gf_gfx_loader.c`,
   `brightness.c`, `screen_fade.c`, `title_screen.c`,
   `intro_movie_scene_1.c`) — how HeartGold actually drives the hardware.
3. GBATEK conventions, only where the SDK header leaves a value implicit,
   and always marked as such.

What Phase 3 needs: **BG mode 0 text layers, palettes, scroll, priority,
alpha blend, master brightness, and the engine→LCD mapping** — the sum
total of what the copyright beat and the title screen use. Everything
else is documented here to the depth needed to keep the model honest
about what it is *not* computing.

## The two engines

Two independent 2D engines, each driving a 256×192 LCD:

| Engine | SDK prefix | Register base | NitroFS-era name |
|---|---|---|---|
| A | `G2_` / `GX_` (e.g. `reg_G2_BG0CNT`) | 0x04000000 | MAIN |
| B | `G2S_` / `GXS_` (e.g. `reg_G2S_DB_BG0CNT`) | 0x04001000 | SUB |

Per engine: 4 BG layers, one OBJ (sprite) layer, 256-color BG palette
RAM, 256-color OBJ palette RAM, one alpha-blend unit, one
master-brightness unit, one backdrop ("mask") color.

**LCD mapping — `GX_SetDispSelect` (`reg_GX_POWCNT` bit 15, DSEL).** The
SDK enum (`gx/gx.h`) names the two states by top-then-bottom engine:

| Value | Enum | Top LCD | Bottom LCD |
|---|---|---|---|
| 0 | `GX_DISP_SELECT_SUB_MAIN` | engine B (SUB) | engine A (MAIN) |
| 1 | `GX_DISP_SELECT_MAIN_SUB` | engine A | engine B |

The polarity is pinned by pret usage, not by the header alone:
mini-apps that borrow the touch LCD restore
`GX_SetDispSelect(GX_DISP_SELECT_MAIN_SUB)` on exit (`src/alph_puzzle.c`,
`src/berry_pots_app.c`) — the console's resting state, main engine on
top. HeartGold's title and intro instead set `gSystem.screensFlipped =
TRUE` and call `GfGfx_SwapDisplay()`, which writes DSEL=0
(`src/gf_gfx_planes.c:86`): **engine B on the top LCD, engine A on the
bottom (touch) LCD**. That is why the title logo/version art — engine B
content — appears on the top screen, while the "TOUCH" prompt window —
engine A BG3 content — sits on the touch screen. Our model calls this
`DisplaySelect::SubOnTop` / `MainOnTop`, and the rasterizer is
display-agnostic: it renders engine A and engine B; the *caller*
(presenter, dump tool) maps them to LCDs.

`GX_SetPower` gates engines (`GX_POWER_2D_MAIN`, `GX_POWER_2D_SUB`,
`GX_POWER_RE`/`GE` for the 3D core); `GfGfx_BothDispOn` is the game's
"turn both 2D engines on" idiom.

## BG control (`BGxCNT`) — text mode

`GXBg01Control` / `GXBg23ControlText` (`gx/gx_bgcnt.h`), 16-bit:

```text
15     14      13   12..8    7          6          5..1        1..0
bgExtPltt  screenSize  …  screenBase  colorMode  mosaic  charBase  priority
```

(field order in the register: `priority:2 | charBase:4 | mosaic:1 |
colorMode:1 | screenBase:5 | [bgExtPltt] | screenSize:2`.)

* **priority** (0–3): lower value draws on top; equal priority →
  lower-numbered BG on top (same convention as GBA; GBATEK). The OBJ
  layer's priority-vs-BG tie rule is documented but unexercised in
  Phase 3 (no sprites in either app).
* **charBase** (0–15): which 16 KiB block of BG VRAM holds this layer's
  8×8 tiles. Multiple layers may share a block — the copyright beat's
  SUB BG0 and BG1 both read block `0x04000`.
* **screenBase** (0–31): which 2 KiB unit holds the map entries.
* **colorMode**: `GX_BG_COLORMODE_16` (4bpp) or `GX_BG_COLORMODE_256`
  (8bpp). 4bpp entries pick a 16-color bank; 8bpp uses the full engine
  BG palette (an entry's palette bits are ignored).
* **screenSize** (`GX_BG_SCRSIZE_TEXT_*`): 256×256, 512×256, 256×512,
  512×512. The map is stored as 2 KiB 32×32 blocks whose order follows
  the size bits (512-wide sizes store left/right block pairs; 512-tall
  store top/bottom).
* **mosaic**, **bgExtPltt**: modeled as ignored fields — both Phase 3
  apps leave them off / `GX_BG_EXTPLTT_NONE`.

BG0 of engine A has a mode flag instead of text/affine variants:
`GX_BG0_AS_2D` or `GX_BG0_AS_3D` — when 3D, the 3D core's framebuffer
*is* layer BG0 of engine A (with its own 3D priority). The title screen
binds the Ho-Oh/Lugia NSBMD there; **Phase 3 defers the 3D core**, so
our BG0 renders as absent (transparent — backdrop/3D-black, see the
deferral table below).

Affine, 256×16-pltt, and bitmap BGs (BG2/BG3 only) exist on hardware
but are unused by the Phase 3 apps; the model does not include them.

## Text-layer screen entries

One u16 per 8×8 cell (this is the bitfield `Nscr::entry` decodes in
`apricorn-core`, and what `cache::Screen` stores raw for the engine):

```text
bits 0–9    tile index within the layer's char block
bit  10     h-flip (0x0400)
bit  11     v-flip (0x0800)
bits 12–15  palette bank (4bpp only; ignored in 8bpp)
```

Cleanly: `tile = e & 0x3FF`, `hflip = e & 0x0400 != 0`, `vflip = e &
0x0800 != 0`, `palette = e >> 12`. Color 0 of a bank is transparent
(shows whatever is beneath in the layer stack). The rasterizer decodes
this bitfield itself — `cache.rs` deliberately stores entries raw.

## Palettes

* Engine BG palette RAM: 256 × BGR555 (`GX_RGB`: r bits 0–4, g 5–9,
  b 10–14). In 4bpp, the entry's bank selects one of 16 banks of 16
  colors; in 8bpp the whole 256 is addressed directly.
* Engine OBJ palette RAM: same shape, OBJ only (unused in Phase 3).
* **Extended palettes** (`bgExtPltt`): 16 banks of 256 colors selected
  by `GX_BG_EXTPLTT_01` (BG0/1) / `GX_BG_EXTPLTT_23` (BG2/3). Both
  Phase 3 apps configure `*_BGEXTPLTT_NONE`; deferred.
* The **backdrop** ("mask color", `BG_SetMaskColor`): the color shown
  wherever no enabled layer covers the pixel and blending doesn't say
  otherwise. The copyright beat sets it to `RGB_BLACK` explicitly.
* Color expansion is exact: 5-bit → 8-bit as `v << 3 | v >> 2` (the
  SDK's convention, already applied in `cache::Palette::rgba()`).

## OAM / OBJ (modeled, not rendered in Phase 3)

`GXOamAttr` (`gx/g2_oam.h`), 8 bytes per sprite, 128 entries: attr0/1
pack `y:8 | rsMode:2 | objMode:2 (NORMAL/XLU/OBJWND/BITMAPOBJ) |
mosaic:1 | colorMode:1 | shape:2` and `x:9 | rsParam:5 | size:2`
(h/v flips live in rsParam's high bits); attr2 packs `charNo:10 |
priority:2 | cParam:4 (palette bank:4)`. Phase 3 carries no OBJ layer in
its logical model beyond a constant "OBJ plane empty" premise; the
plane *bit* exists in blend masks (below) and always contributes
transparent pixels. Real sprite rendering arrives with the Phase 5
overworld.

## Alpha blend and brightness (per engine)

**`BLDCNT`** (`io_reg.h` masks): `plane1` bits 0–5, `effect` bits 6–7,
`plane2` bits 8–13. Plane bits (SDK `GXBlendPlaneMask`):

```text
BG0=0x01  BG1=0x02  BG2=0x04  BG3=0x08  OBJ=0x10  BD=0x20
```

(GBATEK adds a 3D plane bit on engine A; the SDK header models only the
six bits above, and the 3D core is deferred anyway.)

**`BLDALPHA`**: `EVA` bits 0–4, `EBV` bits 8–12 — 5-bit weights.
**`BLDY`**: `EVY` bits 0–4 — 5-bit fade weight.

Effects (`effect` field): 0 none, 1 alpha blend, 2 fade-to-white, 3
fade-to-black (SDK values 2/3 per GBATEK; the header exposes them only
through `G2_SetBlendAlpha` / `G2_SetBlendBrightness`).

Per pixel: find the topmost visible pixel whose *source plane* is in
`plane1` (call it the first target); alpha blend takes the second
target as the next plane-pixel below whose source plane is in `plane2`,
output channel `≈ (first·EVA + second·EBV) / 16` — the 5-bit weights
**clamp to 16** (values 17–31 in the register behave as 16, so
`G2_SetBlendAlpha(…, 31, 0)` is the identity "first target whole"),
and Phase 3 computes it as `(first·EVA + second·EBV + 8) >> 4` with
that clamp — the `+ 8` the round-to-nearest half-step — matching
melonDS's software path (`GPU2D.cpp`'s register clamp, `GPU2D_Soft.h`'s
`ColorBlend4`), which is what the oracle-equivalence story will
compare against. If no plane1 pixel covers, the pixel passes through
unblended. HeartGold's fades drive
this directly: the copyright beat's 60-frame fade is
`ev = counter * 31 / 60` written into EVA/EBV against the OBJ plane
(empty) — i.e. a fade to the backdrop; the title's logo fade ramps
BG2's alpha against `BD | BG0 | BG3 | OBJ`.

**`MASTER_BRIGHT`** (`0x0400006C`, engine A; `0x0400106C`, engine B):
`value` bits 0–4 (0–16 used in practice, 31 max), mode bits 14–15
(`E_MOD`): 0 disabled, 1 up (toward white), 2 down (toward black)
(GBATEK values; the header gives only the field geometry). Applied to
the whole composited screen after blending. `SetMasterBrightnessNeutral`
— the game's "reset the screen to normal" call, used on every scene
enter/exit — is mode 0. pret's slow palette effects additionally run a
software palette-fade system (`src/palette.c`, `src/screen_fade.c`)
that rewrites palette RAM per frame; Phase 3 does not need it (neither
app uses it on the modeled beats).

## VRAM banks (the app-level view)

Nine LCDC banks (A–I) are assigned per-app to engine roles via
`GfGfx_SetBanks` (`src/gf_gfx_planes.c`, enums in `gx/gx_vramcnt.h`).
Phase 3's two apps:

* **Title screen**: `GX_VRAM_BG_128_B` (engine A BG), `SUB_BG_128_C`
  (engine B BG), **OBJ = NONE** (no sprites at all), texture banks A
  and G for the 3D core (deferred with it).
* **Copyright beat**: `BG_128_B`, `SUB_BG_128_C`, `OBJ_32_F`,
  `SUB_OBJ_16_I` (sprite staging for the later sunrise beat — unused in
  ours), tex banks for scene 3 (same).

Our model does not emulate VRAM banking: `EngineFrame` holds named
char-block slots (`char_base` indices 0–7, each an optional tile-set
reference) and per-layer palette references, which carries the one
behavior that matters — layers sharing a char base see the same tiles.
Bank *assignment* is a hardware detail beneath the logical model.

## What Phase 3 models vs. defers

| Modeled | Deferred (and why) |
|---|---|
| Text BG layers (mode 0), 4/8bpp, char/screen bases, size 256×256–512×512, scroll, priority + tie rule, plane enable | Affine/bitmap/extended-palette BG modes (no Phase 3 app uses them) |
| Per-engine 256×BGR555 BG palette, 16 banks, color-0 transparency, exact 5→8 expansion | OBJ palette (no sprites) |
| Backdrop color, `BG_SetMaskColor` | — |
| Alpha blend (BLDCNT/BLDALPHA incl. BD/OBJ plane bits, hardware rounding), master brightness up/down/off | 3D plane in blends (3D core deferred to Phase 6) |
| Engine A/B + `GX_SetDispSelect` mapping | Engine A 3D framebuffer-as-BG0 (title's Ho-Oh/Lugia model: Phase 6) |
| OBJ plane as "empty" in blend math | OAM rendering: shapes/sizes, 1D/2D mapping, affine sprites (Phase 5 overworld) |
| | HW windows (circle wipe etc. — intro scenes 2–5), mosaic, per-line HBlank effects |
| | Software palette-fade system (`palette.c`) — neither modeled beat uses it |

The deferral column is a contract: `apricorn-gfx`'s rasterizer and the
frame-indexed tests may not quietly approximate a deferred feature —
where an app uses one, the app code says so in its doc comment and the
model represents it as absent.