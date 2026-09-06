//! The rasterizer — one logical frame to two RGBA screens.
//!
//! For each engine ([`EngineFrame`]) this walks every output pixel
//! and composites the enabled text BG layers top-down by
//! `BGxCNT` priority (ties to the lower BG index, the GBA
//! convention), over the backdrop color. A layer's pixel comes from
//! its screen entry at the scrolled position (scroll wraps within
//! the layer's map size), the 8×8 tile in its char block, and the
//! layer's placed palette — 4bpp entries pick a 16-color bank,
//! 8bpp addresses the palette directly, and color 0 is transparent
//! (`docs/nds-2d.md`, "Text-layer screen entries" and "Palettes").
//!
//! Compositing is exact and integer end to end:
//!
//! * **Alpha blend** (`BLDCNT`/`BLDALPHA`): if the *displayed*
//!   (topmost) pixel's plane is in the first-target mask and the
//!   effect is alpha, the rasterizer looks below it for the topmost
//!   pixel whose plane is in the second-target mask — the backdrop
//!   is the bottom-most plane — and emits
//!   `(first·EVA + second·EBV) >> 4` per channel, clamped (the
//!   hardware's 5-bit weights over 16). With no second target the
//!   first target shows unblended; a topmost pixel outside the
//!   first-target mask always passes through unblended.
//! * **Master brightness** applies last, to the whole screen:
//!   up is `c + (255 - c)·value >> 4`, down is `c - c·value >> 4`
//!   (the register's 5-bit weight, value 16 reaches the limit).
//!
//! Deferred per `docs/nds-2d.md` and honored here: the OBJ plane is
//! empty (contributes no pixels, but its blend-mask bit stays
//! inert), engine A's 3D framebuffer-as-BG0 renders as absent, and
//! no VRAM banking exists — layers name char-block slots. A layer
//! whose referenced asset is missing renders transparent, which is
//! how the deferred 3D BG0 is expressed.

use apricorn_core::assets::AssetStore;
use apricorn_core::cache;
use apricorn_core::frame::{
    AssetId, BgLayer, BlendEffect, BrightnessMode, ColorMode, EngineFrame, LogicalFrame, plane,
};

/// The handle-resolution seam between the asset store and the
/// rasterizer.
///
/// The store owns the decoded chunks; the rasterizer only needs to
/// resolve a frame's [`AssetId`] handles back to them. Tests bring
/// their own implementation with hand-built chunks, and the
/// [`AssetStore`] itself implements this, so ROM loading and pixel
/// math stay separable.
pub trait AssetSource {
    /// The tiles chunk a `char_base` slot or tile handle names.
    fn tiles(&self, id: AssetId) -> Option<&cache::Tiles>;
    /// The screen chunk a layer's `screen` handle names.
    fn screen(&self, id: AssetId) -> Option<&cache::Screen>;
    /// The placed palette colors (PMCP applied) a layer's `palette`
    /// handle names.
    fn placed_palette(&self, id: AssetId) -> Option<&[[u8; 4]]>;
}

impl AssetSource for AssetStore {
    fn tiles(&self, id: AssetId) -> Option<&cache::Tiles> {
        AssetStore::tiles(self, id)
    }

    fn screen(&self, id: AssetId) -> Option<&cache::Screen> {
        AssetStore::screen(self, id)
    }

    fn placed_palette(&self, id: AssetId) -> Option<&[[u8; 4]]> {
        AssetStore::placed_palette(self, id)
    }
}

/// One engine's rendered screen: 256×192 pixels of RGBA8 (red,
/// green, blue, alpha; alpha is always 255 — transparency is a
/// compositing fact, not an output value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenBuffer {
    pixels: Box<[[u8; 4]]>,
}

impl ScreenBuffer {
    /// The screen width in pixels.
    pub const WIDTH: usize = 256;
    /// The screen height in pixels.
    pub const HEIGHT: usize = 192;

    /// A black screen.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pixels: vec![[0, 0, 0, 255]; Self::WIDTH * Self::HEIGHT].into_boxed_slice(),
        }
    }

    /// The pixel at `(x, y)`.
    ///
    /// # Panics
    /// Panics when `x`/`y` fall outside the screen.
    #[must_use]
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        self.pixels[self.index(x, y)]
    }

    /// The pixels as a flat RGBA8 row-major image, ready for a
    /// texture upload.
    #[must_use]
    pub fn as_rgba(&self) -> &[[u8; 4]] {
        &self.pixels
    }

    fn index(&self, x: usize, y: usize) -> usize {
        assert!(x < Self::WIDTH && y < Self::HEIGHT, "pixel off the screen");
        y * Self::WIDTH + x
    }
}

impl Default for ScreenBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Renders both engines' screens for `frame` — engine A (MAIN) then
/// engine B (SUB), in that order. The rasterizer is display-agnostic:
/// [`LogicalFrame::display`] says which LCD each drives, and the
/// caller (presenter, dump tool) maps them.
#[must_use]
pub fn render<S: AssetSource + ?Sized>(frame: &LogicalFrame, store: &S) -> [ScreenBuffer; 2] {
    [
        render_engine(&frame.main, store),
        render_engine(&frame.sub, store),
    ]
}

/// Renders one engine: 256×192 pixels composited from its BG layers,
/// blend unit, backdrop, and brightness.
fn render_engine<S: AssetSource + ?Sized>(engine: &EngineFrame, store: &S) -> ScreenBuffer {
    // The draw order: lower priority on top, ties to the lower BG
    // index. The OBJ plane is empty in Phase 3 and the backdrop is
    // handled as the bottom-most plane below.
    let mut order: [usize; 4] = [0, 1, 2, 3];
    order.sort_by_key(|&i| (engine.bgs[i].priority, i));

    let mut out = ScreenBuffer::new();
    for y in 0..ScreenBuffer::HEIGHT {
        for x in 0..ScreenBuffer::WIDTH {
            let [r, g, b, _] = composite_pixel(engine, store, &order, x, y);
            out.pixels[out.index(x, y)] = [r, g, b, 255];
        }
    }
    apply_brightness(engine, &mut out);
    out
}

/// Composites one pixel: the topmost opaque layer pixel (by draw
/// order) or the backdrop, alpha-blended per the engine's blend unit.
fn composite_pixel<S: AssetSource + ?Sized>(
    engine: &EngineFrame,
    store: &S,
    order: &[usize; 4],
    x: usize,
    y: usize,
) -> [u8; 4] {
    let blending = matches!(engine.blend.effect, BlendEffect::Alpha) && engine.blend.plane1 != 0;
    // The displayed pixel (topmost opaque), and — when it is a blend
    // first target — the second target found below it.
    let mut top: Option<([u8; 4], bool)> = None; // (color, plane in plane1)
    let mut second: Option<[u8; 4]> = None;

    for &i in order {
        let layer = &engine.bgs[i];
        if !layer.enabled {
            continue;
        }
        let Some(color) = sample_layer(store, &engine.char_blocks, layer, x, y) else {
            continue;
        };
        let bit = plane::BG0 << i;
        if top.is_none() {
            let blend_top = blending && engine.blend.plane1 & bit != 0;
            top = Some((color, blend_top));
            // A topmost pixel outside plane1 passes through
            // unblended — nothing below can change it.
            if !blend_top {
                break;
            }
        } else if second.is_none() && engine.blend.plane2 & bit != 0 {
            second = Some(color);
            break;
        }
    }

    let backdrop = backdrop_rgba(engine.backdrop);
    let (color, blend_top) = match top {
        Some((color, blend_top)) => (color, blend_top),
        None => (backdrop, false),
    };
    // The backdrop is the bottom-most plane: it completes a second
    // target search (as plane BD) but never starts one here — there
    // is nothing below it to blend with.
    if blend_top && second.is_none() && engine.blend.plane2 & plane::BD != 0 {
        second = Some(backdrop);
    }

    match (blend_top, second) {
        (true, Some(second)) => alpha_blend(color, second, engine.blend.eva, engine.blend.ebv),
        _ => color,
    }
}

/// Samples one layer at output pixel `(x, y)` — `None` when the
/// layer is transparent there (color 0, a missing asset, or a tile
/// index beyond the char block).
fn sample_layer<S: AssetSource + ?Sized>(
    store: &S,
    char_blocks: &[Option<AssetId>; 8],
    layer: &BgLayer,
    x: usize,
    y: usize,
) -> Option<[u8; 4]> {
    let screen_id = layer.screen?;
    let tiles_id = char_blocks
        .get(usize::from(layer.char_base))
        .copied()
        .flatten()?;
    let palette_id = layer.palette?;
    let screen = store.screen(screen_id)?;
    let tiles = store.tiles(tiles_id)?;
    let palette = store.placed_palette(palette_id)?;

    // The scrolled position, wrapped within the layer's map.
    let map_w = usize::from(layer.size.tiles_wide()) * 8;
    let map_h = usize::from(layer.size.tiles_tall()) * 8;
    let mx = (x + usize::from(layer.scroll_x)) % map_w;
    let my = (y + usize::from(layer.scroll_y)) % map_h;

    // The screen chunk covers the map from its top-left; tiles the
    // chunk does not cover read as entry 0 (the plan's rule for
    // maps shorter than the 256×256 layer).
    let chunk_w = usize::from(screen.width() / 8);
    let chunk_h = usize::from(screen.height() / 8);
    let (tx, ty) = (mx / 8, my / 8);
    let entry = if tx < chunk_w && ty < chunk_h {
        let at = (ty * chunk_w + tx) * 2;
        let entries = screen.entries();
        if at + 1 >= entries.len() {
            return None;
        }
        u16::from_le_bytes([entries[at], entries[at + 1]])
    } else {
        0
    };

    let tile = usize::from(entry & 0x3FF);
    let h_flip = entry & 0x0400 != 0;
    let v_flip = entry & 0x0800 != 0;
    let bank = usize::from(entry >> 12);
    if tile >= usize::try_from(tiles.tile_count()).expect("u32 fits usize") {
        return None;
    }

    let mut px = mx % 8;
    let mut py = my % 8;
    if h_flip {
        px = 7 - px;
    }
    if v_flip {
        py = 7 - py;
    }
    let value = tiles.pixels()[tile * 64 + py * 8 + px];
    if value == 0 {
        return None; // color 0 is transparent
    }
    let index = match layer.color_mode {
        ColorMode::Bpp4 => bank * 16 + usize::from(value),
        ColorMode::Bpp8 => usize::from(value),
    };
    palette.get(index).copied()
}

/// The alpha blend itself: `(first·EVA + second·EBV) >> 4` per
/// channel, clamped — the hardware's 5-bit weights over 16
/// (`BLDALPHA`).
fn alpha_blend(first: [u8; 4], second: [u8; 4], eva: u8, ebv: u8) -> [u8; 4] {
    let mix = |a: u8, b: u8| {
        let v = (u32::from(a) * u32::from(eva) + u32::from(b) * u32::from(ebv)) >> 4;
        u8::try_from(v.min(0xFF)).expect("clamped to u8")
    };
    [
        mix(first[0], second[0]),
        mix(first[1], second[1]),
        mix(first[2], second[2]),
        255,
    ]
}

/// The backdrop as RGBA8. The frame stores the raw `GX_RGB` BGR555
/// value (r bits 0–4, g 5–9, b 10–14); the expansion is the SDK's
/// exact `v << 3 | v >> 2`.
fn backdrop_rgba(color: u16) -> [u8; 4] {
    let expand = |v: u16| (v << 3 | v >> 2) as u8;
    [
        expand(color & 0x1F),
        expand(color >> 5 & 0x1F),
        expand(color >> 10 & 0x1F),
        255,
    ]
}

/// Applies the engine's master brightness to the whole screen, last.
fn apply_brightness(engine: &EngineFrame, screen: &mut ScreenBuffer) {
    let value = u32::from(engine.brightness.value);
    let adjust = |c: u8| match engine.brightness.mode {
        BrightnessMode::Disabled => c,
        // Up: toward white; down: toward black; both by value/16ths.
        BrightnessMode::Up => {
            let v = u32::from(c) + (((0xFF - u32::from(c)) * value) >> 4);
            u8::try_from(v.min(0xFF)).expect("clamped to u8")
        }
        BrightnessMode::Down => {
            // value 16 reaches black; larger weights saturate there.
            let sub = (u32::from(c) * value) >> 4;
            u8::try_from(u32::from(c).saturating_sub(sub)).expect("saturating keeps it in u8")
        }
    };
    for pixel in screen.pixels.iter_mut() {
        *pixel = [adjust(pixel[0]), adjust(pixel[1]), adjust(pixel[2]), 255];
    }
}
