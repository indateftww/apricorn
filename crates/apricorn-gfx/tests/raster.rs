//! Rasterizer tests — synthetic fixtures, no ROM.
//!
//! Every fixture is a hand-built cache chunk parsed through the same
//! `cache::parse` readers the asset store produces, wrapped in a
//! [`FixtureStore`] that implements [`AssetSource`]. The assertions
//! are hand-computed pixels (flips, banks, blend weights, brightness)
//! plus one golden-hash test pinning a composite scene by its SHA-1
//! — hashes only, never pixels, per the house rule.

use apricorn_core::cache;
use apricorn_core::frame::{
    AssetId, BgLayer, Blend, BlendEffect, BrightnessMode, ColorMode, EngineFrame, LogicalFrame,
    MasterBrightness, plane,
};
use apricorn_gfx::{AssetSource, ScreenBuffer, render};
use sha1::{Digest, Sha1};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// One fixture asset.
enum Fix {
    Tiles(cache::Tiles),
    Screen(cache::Screen),
    Palette(Vec<[u8; 4]>),
}

/// An in-memory [`AssetSource`]: handles index the added assets in
/// load order, exactly like the real store.
struct FixtureStore {
    items: Vec<Fix>,
}

impl FixtureStore {
    fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Parses and stores a Tiles chunk.
    fn add_tiles(&mut self, fmt: u8, pixels: &[u8]) -> AssetId {
        assert!(pixels.len().is_multiple_of(64), "whole 8×8 tiles");
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&cache::MAGIC);
        chunk.extend_from_slice(&cache::VERSION.to_le_bytes());
        chunk.extend_from_slice(&cache::ChunkKind::Tiles.code().to_le_bytes());
        chunk.push(fmt); // 0 = 4bpp (expanded), 1 = 8bpp
        chunk.push(0); // mapping: TwoD
        chunk.push(0); // flags
        chunk.push(0); // pad
        chunk.extend_from_slice(&((pixels.len() / 64) as u32).to_le_bytes());
        chunk.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
        chunk.extend_from_slice(pixels);
        self.items.push(Fix::Tiles(
            cache::Tiles::parse(&chunk).expect("fixture tiles parse"),
        ));
        AssetId::from_index(self.items.len() - 1)
    }

    /// Parses and stores a Screen chunk — one u16 entry per 8×8 cell.
    fn add_screen(&mut self, width: u16, height: u16, entries: &[u16]) -> AssetId {
        assert_eq!(
            entries.len(),
            usize::from(width / 8) * usize::from(height / 8),
            "entries cover the screen"
        );
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&cache::MAGIC);
        chunk.extend_from_slice(&cache::VERSION.to_le_bytes());
        chunk.extend_from_slice(&cache::ChunkKind::Screen.code().to_le_bytes());
        chunk.extend_from_slice(&width.to_le_bytes());
        chunk.extend_from_slice(&height.to_le_bytes());
        chunk.extend_from_slice(&0u16.to_le_bytes()); // color mode
        chunk.extend_from_slice(&0u16.to_le_bytes()); // text format
        chunk.extend_from_slice(&((entries.len() * 2) as u32).to_le_bytes());
        for entry in entries {
            chunk.extend_from_slice(&entry.to_le_bytes());
        }
        self.items.push(Fix::Screen(
            cache::Screen::parse(&chunk).expect("fixture screen parses"),
        ));
        AssetId::from_index(self.items.len() - 1)
    }

    /// Parses and stores a Palette chunk (no PMCP).
    fn add_palette(&mut self, bpp4: bool, colors: &[[u8; 4]]) -> AssetId {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&cache::MAGIC);
        chunk.extend_from_slice(&cache::VERSION.to_le_bytes());
        chunk.extend_from_slice(&cache::ChunkKind::Palette.code().to_le_bytes());
        chunk.push(u8::from(bpp4)); // flags: bit0 = 16-color
        chunk.push(0); // pad
        chunk.extend_from_slice(&0u16.to_le_bytes()); // pad
        chunk.extend_from_slice(&(colors.len() as u32).to_le_bytes());
        chunk.extend_from_slice(&0u16.to_le_bytes()); // no PMCP
        chunk.extend_from_slice(&0u16.to_le_bytes()); // pad
        for color in colors {
            chunk.extend_from_slice(color);
        }
        let parsed = cache::Palette::parse(&chunk).expect("fixture palette parses");
        self.items.push(Fix::Palette(parsed.rgba().to_vec()));
        AssetId::from_index(self.items.len() - 1)
    }
}

impl AssetSource for FixtureStore {
    fn tiles(&self, id: AssetId) -> Option<&cache::Tiles> {
        match self.items.get(id.index()) {
            Some(Fix::Tiles(tiles)) => Some(tiles),
            _ => None,
        }
    }

    fn screen(&self, id: AssetId) -> Option<&cache::Screen> {
        match self.items.get(id.index()) {
            Some(Fix::Screen(screen)) => Some(screen),
            _ => None,
        }
    }

    fn placed_palette(&self, id: AssetId) -> Option<&[[u8; 4]]> {
        match self.items.get(id.index()) {
            Some(Fix::Palette(colors)) => Some(colors),
            _ => None,
        }
    }
}

/// A text screen entry: tile, flips, palette bank.
fn entry(tile: u16, h_flip: bool, v_flip: bool, bank: u16) -> u16 {
    tile | u16::from(h_flip) << 10 | u16::from(v_flip) << 11 | bank << 12
}

/// A palette of `count` colors where color `i` is `[i, i, i, 255]` —
/// every lookup result is directly readable.
fn gray_palette(count: usize) -> Vec<[u8; 4]> {
    (0..count)
        .map(|i| [i as u8, i as u8, i as u8, 255])
        .collect()
}

/// An enabled 4bpp BG layer.
fn bg_layer(screen: AssetId, palette: AssetId) -> BgLayer {
    BgLayer {
        enabled: true,
        screen: Some(screen),
        palette: Some(palette),
        ..BgLayer::default()
    }
}

/// One engine: `layers` at BG0.., tiles placed per `(slot, asset)`.
fn engine(layers: &[BgLayer], slots: &[(usize, AssetId)]) -> EngineFrame {
    let mut engine = EngineFrame::default();
    for (i, layer) in layers.iter().enumerate() {
        engine.bgs[i] = *layer;
    }
    for &(slot, tiles) in slots {
        engine.char_blocks[slot] = Some(tiles);
    }
    engine
}

// ---------------------------------------------------------------------------
// Screen entries and tile fetch
// ---------------------------------------------------------------------------

#[test]
fn flips_decode_from_the_entry_bits() {
    let mut store = FixtureStore::new();
    // One 4bpp tile: pixel value at (x, y) is x + y + 1 (1..=15).
    let pixels: Vec<u8> = (0..64).map(|i| (i % 8 + i / 8 + 1) as u8).collect();
    let tiles = store.add_tiles(0, &pixels);
    // Four cells: plain, h-flip, v-flip, both.
    let entries = [
        entry(0, false, false, 0),
        entry(0, true, false, 0),
        entry(0, false, true, 0),
        entry(0, true, true, 0),
    ];
    let screen = store.add_screen(32, 8, &entries);
    let palette = store.add_palette(true, &gray_palette(16));
    let frame = LogicalFrame {
        main: engine(&[bg_layer(screen, palette)], &[(0, tiles)]),
        ..LogicalFrame::default()
    };

    let [main, _] = render(&frame, &store);
    let v = |x, y| main.pixel(x, y)[0]; // gray palette: r == value
    assert_eq!(v(3, 2), 3 + 2 + 1, "plain reads (x, y)");
    // x=9..15 is cell 1: the h-flipped tile.
    assert_eq!(v(9, 2), (7 - 1) + 2 + 1, "h-flip mirrors x within 0..8");
    assert_eq!(v(17, 3), 1 + (7 - 3) + 1, "v-flip mirrors y within 0..8");
    assert_eq!(v(25, 4), (7 - 1) + (7 - 4) + 1, "hv mirrors both");
}

#[test]
fn bpp4_picks_the_entry_bank_and_bpp8_reads_the_whole_palette() {
    let mut store = FixtureStore::new();
    // One tile of value 3 everywhere.
    let tiles = store.add_tiles(0, &[3u8; 64]);
    // Entries: bank 0 (cell 0), bank 1 (cell 1), bank 2 with 8bpp.
    let entries = [entry(0, false, false, 0), entry(0, false, false, 1)];
    let screen = store.add_screen(16, 8, &entries);
    let palette = store.add_palette(true, &gray_palette(48));
    let frame = LogicalFrame {
        main: engine(&[bg_layer(screen, palette)], &[(0, tiles)]),
        ..LogicalFrame::default()
    };

    let [main, _] = render(&frame, &store);
    assert_eq!(main.pixel(0, 0)[0], 3, "bank 0: colors 0..15");
    assert_eq!(main.pixel(8, 0)[0], 16 + 3, "bank 1: colors 16..31");

    // 8bpp: the bank bits are ignored, value 200 reads color 200.
    let mut store = FixtureStore::new();
    let tiles = store.add_tiles(1, &[200u8; 64]);
    let screen = store.add_screen(8, 8, &[entry(0, false, false, 15)]);
    let palette = store.add_palette(false, &gray_palette(256));
    let frame = LogicalFrame {
        main: engine(
            &[{
                let mut layer = bg_layer(screen, palette);
                layer.color_mode = ColorMode::Bpp8;
                layer
            }],
            &[(0, tiles)],
        ),
        ..LogicalFrame::default()
    };
    let [main, _] = render(&frame, &store);
    assert_eq!(main.pixel(4, 4)[0], 200, "8bpp: full palette, bank ignored");
}

#[test]
fn color_zero_is_transparent_and_shows_the_layer_below() {
    let mut store = FixtureStore::new();
    // Top tile: value 0 (transparent); bottom tile: value 1.
    let top = store.add_tiles(0, &[0u8; 64]);
    let bottom = store.add_tiles(0, &[1u8; 64]);
    let screen_top = store.add_screen(16, 8, &[entry(0, false, false, 0), 0]);
    let screen_bottom = store.add_screen(16, 8, &[entry(0, false, false, 0); 2]);
    let palette = store.add_palette(true, &gray_palette(16));
    let mut frame = LogicalFrame::default();
    let mut e = EngineFrame::default();
    e.bgs[0] = BgLayer {
        enabled: true,
        screen: Some(screen_top),
        palette: Some(palette),
        priority: 0,
        ..BgLayer::default()
    };
    e.bgs[1] = BgLayer {
        enabled: true,
        screen: Some(screen_bottom),
        palette: Some(palette),
        priority: 0,
        ..BgLayer::default()
    };
    e.char_blocks[0] = Some(bottom);
    e.char_blocks[1] = Some(top);
    e.bgs[0].char_base = 1;
    frame.main = e;

    let [main, _] = render(&frame, &store);
    assert_eq!(
        main.pixel(0, 0)[0],
        1,
        "hole over cell 0 shows BG1's tile 1"
    );
    assert_eq!(
        main.pixel(8, 0)[0],
        1,
        "cell 1: top transparent, bottom tile 1"
    );
}

// ---------------------------------------------------------------------------
// Scroll and map geometry
// ---------------------------------------------------------------------------

#[test]
fn scroll_wraps_within_the_layer_map() {
    let mut store = FixtureStore::new();
    // Tile n is uniform color n+1; the map's entry (tx, ty) = tile tx,
    // so the color at output x names the map column.
    let pixels: Vec<u8> = (0..4).flat_map(|n| [n + 1; 64]).collect();
    let tiles = store.add_tiles(0, &pixels);
    let entries: Vec<u16> = (0..32 * 4)
        .map(|i| entry(u16::try_from(i % 32).unwrap(), false, false, 0))
        .collect();
    let screen = store.add_screen(256, 32, &entries);
    let palette = store.add_palette(true, &gray_palette(16));
    let mut frame = LogicalFrame::default();
    let mut layer = bg_layer(screen, palette);
    layer.scroll_x = 8; // one tile
    frame.main = engine(&[layer], &[(0, tiles)]);

    let [main, _] = render(&frame, &store);
    assert_eq!(
        main.pixel(0, 0)[0],
        2,
        "scroll 8: output 0 reads map column 8"
    );
    assert_eq!(main.pixel(8, 0)[0], 3, "output 8 reads map column 16");

    // Raw scroll is 9-bit: 264 wraps to 8 inside the 256-wide map.
    let mut layer = bg_layer(screen, palette);
    layer.scroll_x = 264;
    frame.main = engine(&[layer], &[(0, tiles)]);
    let [main, _] = render(&frame, &store);
    assert_eq!(main.pixel(0, 0)[0], 2, "264 % 256 = 8: same column");
}

#[test]
fn maps_shorter_than_the_layer_read_entry_zero() {
    let mut store = FixtureStore::new();
    // Tile 0 is color 1; tile 1 is color 2. A 256×192 chunk (24 tile
    // rows) on a 256×256 layer: rows 192..255 read entry 0.
    let pixels: Vec<u8> = [1u8; 64].iter().copied().chain([2u8; 64]).collect();
    let tiles = store.add_tiles(0, &pixels);
    let entries: Vec<u16> = (0..32 * 24).map(|_| entry(1, false, false, 0)).collect();
    let screen = store.add_screen(256, 192, &entries);
    let palette = store.add_palette(true, &gray_palette(16));
    let mut layer = bg_layer(screen, palette);
    layer.scroll_y = 64; // rows 192..255 of the map become rows 128..191
    let frame = LogicalFrame {
        main: engine(&[layer], &[(0, tiles)]),
        ..LogicalFrame::default()
    };

    let [main, _] = render(&frame, &store);
    assert_eq!(
        main.pixel(0, 127)[0],
        2,
        "the chunk's last row shows tile 1"
    );
    assert_eq!(
        main.pixel(0, 128)[0],
        1,
        "beyond the chunk: entry 0 → tile 0"
    );
}

// ---------------------------------------------------------------------------
// Priority and blend
// ---------------------------------------------------------------------------

/// Two full-cover BG layers of uniform tiles: BG0 shows `colors[0]`,
/// BG1 shows `colors[1]`, priorities per `priorities`.
fn cover_engine(store: &mut FixtureStore, priorities: [u8; 2], colors: [u8; 2]) -> LogicalFrame {
    let palette = store.add_palette(true, &gray_palette(16));
    let mut e = EngineFrame::default();
    for i in 0..2 {
        let tiles = store.add_tiles(0, &[colors[i]; 64]);
        let screen = store.add_screen(8, 8, &[entry(0, false, false, 0)]);
        e.bgs[i] = BgLayer {
            enabled: true,
            screen: Some(screen),
            palette: Some(palette),
            priority: priorities[i],
            char_base: i as u8,
            ..BgLayer::default()
        };
        e.char_blocks[i] = Some(tiles);
    }
    LogicalFrame {
        main: e,
        ..LogicalFrame::default()
    }
}

#[test]
fn priority_orders_layers_and_ties_go_to_the_lower_bg() {
    // BG0 color 1, BG1 color 2.
    let mut store = FixtureStore::new();
    let frame = cover_engine(&mut store, [0, 0], [1, 2]);
    let [main, _] = render(&frame, &store);
    assert_eq!(main.pixel(0, 0)[0], 1, "tie: BG0 wins");

    let mut store = FixtureStore::new();
    let frame = cover_engine(&mut store, [1, 0], [1, 2]);
    let [main, _] = render(&frame, &store);
    assert_eq!(main.pixel(0, 0)[0], 2, "BG1 has the lower priority");

    // A disabled higher layer is out of the stack entirely.
    let mut store = FixtureStore::new();
    let mut frame = cover_engine(&mut store, [0, 1], [1, 2]);
    frame.main.bgs[0].enabled = false;
    let [main, _] = render(&frame, &store);
    assert_eq!(main.pixel(0, 0)[0], 2, "BG0 off: BG1 shows");
}

#[test]
fn alpha_blend_mixes_the_first_and_second_targets() {
    let mut store = FixtureStore::new();
    // BG0 red (plane1), BG1 blue (plane2), EVA = EBV = 8.
    let red = store.add_tiles(0, &[1u8; 64]);
    let blue = store.add_tiles(0, &[2u8; 64]);
    let s0 = store.add_screen(8, 8, &[entry(0, false, false, 0)]);
    let s1 = store.add_screen(8, 8, &[entry(0, false, false, 0)]);
    let palette = store.add_palette(true, &[[0, 0, 0, 255], [255, 0, 0, 255], [0, 0, 255, 255]]);
    let mut e = EngineFrame::default();
    e.bgs[0] = BgLayer {
        enabled: true,
        screen: Some(s0),
        palette: Some(palette),
        ..BgLayer::default()
    };
    e.bgs[1] = BgLayer {
        enabled: true,
        screen: Some(s1),
        palette: Some(palette),
        ..BgLayer::default()
    };
    e.char_blocks[0] = Some(red);
    e.char_blocks[1] = Some(blue);
    e.bgs[0].char_base = 0;
    e.bgs[1].char_base = 1;
    e.blend = Blend {
        plane1: plane::BG0,
        effect: BlendEffect::Alpha,
        plane2: plane::BG1,
        eva: 8,
        ebv: 8,
    };
    let frame = LogicalFrame {
        main: e,
        ..LogicalFrame::default()
    };

    let [main, _] = render(&frame, &store);
    // (255·8 + 0·8) >> 4 = 127 for r; b symmetrically; g stays 0.
    assert_eq!(main.pixel(0, 0), [127, 0, 127, 255]);

    // Weights: EVA 31, EBV 0 → the first target alone.
    let mut frame = frame;
    frame.main.blend.eva = 31;
    frame.main.blend.ebv = 0;
    let [main, _] = render(&frame, &store);
    assert_eq!(main.pixel(0, 0), [255, 0, 0, 255], "EBV 0: pure first");
    // EVA 0, EBV 31 → the second target alone.
    frame.main.blend.eva = 0;
    frame.main.blend.ebv = 31;
    let [main, _] = render(&frame, &store);
    // (0 + 255·31) >> 4 = 493 → clamped to 255.
    assert_eq!(main.pixel(0, 0), [0, 0, 255, 255], "EVA 0: pure second");
}

#[test]
fn blending_rules_for_untargeted_and_unmatched_pixels() {
    let mut store = FixtureStore::new();
    // BG0 (top) green, BG1 red, backdrop white; only BG1 blends.
    let green = store.add_tiles(0, &[1u8; 64]);
    let red = store.add_tiles(0, &[2u8; 64]);
    let s0 = store.add_screen(8, 8, &[entry(0, false, false, 0)]);
    let s1 = store.add_screen(8, 8, &[entry(0, false, false, 0)]);
    let palette = store.add_palette(true, &[[0, 0, 0, 255], [0, 255, 0, 255], [255, 0, 0, 255]]);
    let mut e = EngineFrame::default();
    e.bgs[0] = BgLayer {
        enabled: true,
        screen: Some(s0),
        palette: Some(palette),
        priority: 0,
        ..BgLayer::default()
    };
    e.bgs[1] = BgLayer {
        enabled: true,
        screen: Some(s1),
        palette: Some(palette),
        priority: 1,
        ..BgLayer::default()
    };
    e.char_blocks[0] = Some(green);
    e.char_blocks[1] = Some(red);
    e.bgs[0].char_base = 0;
    e.bgs[1].char_base = 1;
    // plane1 = BG1, plane2 = BD (the white backdrop).
    e.blend = Blend {
        plane1: plane::BG1,
        effect: BlendEffect::Alpha,
        plane2: plane::BD,
        eva: 8,
        ebv: 8,
    };
    e.backdrop = 0x7FFF; // BGR555 white
    let frame = LogicalFrame {
        main: e,
        ..LogicalFrame::default()
    };

    // The displayed pixel is BG0's green — not in plane1 — so it
    // passes through unblended even though BG1 below is a target.
    let [main, _] = render(&frame, &store);
    assert_eq!(
        main.pixel(0, 0),
        [0, 255, 0, 255],
        "top outside plane1: unblended"
    );

    // Disable BG0: BG1 is displayed and blends against the backdrop.
    let mut frame = frame;
    frame.main.bgs[0].enabled = false;
    let [main, _] = render(&frame, &store);
    // r: (255·8 + 255·8) >> 4 = 255; g/b: (0 + 255·8) >> 4 = 127.
    assert_eq!(main.pixel(0, 0), [255, 127, 127, 255], "BG1 over white BD");

    // plane2 empty: no second target anywhere — the first target
    // shows unblended.
    frame.main.bgs[0].enabled = true;
    frame.main.bgs[0].priority = 1;
    frame.main.bgs[1].priority = 0;
    frame.main.blend.plane2 = 0;
    let [main, _] = render(&frame, &store);
    assert_eq!(
        main.pixel(0, 0),
        [255, 0, 0, 255],
        "no second target: unblended"
    );
}

// ---------------------------------------------------------------------------
// Backdrop, missing assets, brightness
// ---------------------------------------------------------------------------

#[test]
fn backdrop_covers_unrendered_pixels_with_exact_expansion() {
    let frame = LogicalFrame::default();
    let store = FixtureStore::new();
    let [main, sub] = render(&frame, &store);
    assert_eq!(main.pixel(0, 0), [0, 0, 0, 255], "black default");
    assert_eq!(sub.pixel(255, 191), [0, 0, 0, 255]);

    // GX_RGB white and the 1/31 step (1 → 0x08).
    let mut frame = LogicalFrame::default();
    frame.sub.backdrop = 0x7FFF;
    assert_eq!(render(&frame, &store)[1].pixel(0, 0), [255, 255, 255, 255]);
    frame.sub.backdrop = 0x0001;
    assert_eq!(render(&frame, &store)[1].pixel(0, 0), [8, 0, 0, 255]);

    // A layer whose assets are missing (the deferred 3D BG0) is
    // transparent, not an error: the backdrop shows through.
    let mut frame = LogicalFrame::default();
    frame.sub.backdrop = 0x001F;
    frame.sub.bgs[0] = BgLayer {
        enabled: true,
        ..BgLayer::default()
    };
    assert_eq!(render(&frame, &store)[1].pixel(0, 0), [255, 0, 0, 255]);
}

#[test]
fn master_brightness_applies_last_over_the_whole_screen() {
    let mut store = FixtureStore::new();
    let tiles = store.add_tiles(0, &[1u8; 64]);
    let screen = store.add_screen(8, 8, &[entry(0, false, false, 0)]);
    let palette = store.add_palette(true, &[[0, 0, 0, 255], [255, 0, 0, 255]]);
    let mut frame = LogicalFrame {
        main: engine(&[bg_layer(screen, palette)], &[(0, tiles)]),
        ..LogicalFrame::default()
    };

    // Up 16: the limit — white.
    frame.main.brightness = MasterBrightness {
        mode: BrightnessMode::Up,
        value: 16,
    };
    assert_eq!(render(&frame, &store)[0].pixel(0, 0), [255, 255, 255, 255]);
    // Down 16: the limit — black.
    frame.main.brightness = MasterBrightness {
        mode: BrightnessMode::Down,
        value: 16,
    };
    assert_eq!(render(&frame, &store)[0].pixel(0, 0), [0, 0, 0, 255]);
    // Up 8 on red: r → 255, g/b → (255-0)*8 >> 4 = 127.
    frame.main.brightness = MasterBrightness {
        mode: BrightnessMode::Up,
        value: 8,
    };
    assert_eq!(render(&frame, &store)[0].pixel(0, 0), [255, 127, 127, 255]);
    // Down 8 on red: r → 255 - (255·8 >> 4) = 128, g/b stay 0.
    frame.main.brightness = MasterBrightness {
        mode: BrightnessMode::Down,
        value: 8,
    };
    assert_eq!(render(&frame, &store)[0].pixel(0, 0), [128, 0, 0, 255]);
    // Disabled: untouched, and the unrendered area is the backdrop
    // (black), not the layer.
    frame.main.brightness = MasterBrightness::default();
    // Past the 8-px map the layer still covers: entry 0 → tile 0 (red),
    // not the backdrop.
    assert_eq!(render(&frame, &store)[0].pixel(8, 0), [255, 0, 0, 255]);
}

// ---------------------------------------------------------------------------
// Golden hash
// ---------------------------------------------------------------------------

/// The golden fixture: a scrolled 4bpp layer with a blend over a
/// second layer and master brightness, exercising the whole path.
/// Pinned by SHA-1 only — never pixels.
#[test]
fn golden_composite_scene_hashes_stably() {
    let mut store = FixtureStore::new();
    // Tiles: 0 = red, 1 = green, 2 = blue (palette values 1, 2, 3).
    let pixels: Vec<u8> = [1u8; 64]
        .iter()
        .chain([2u8; 64].iter())
        .chain([3u8; 64].iter())
        .copied()
        .collect();
    let tiles = store.add_tiles(0, &pixels);
    // A 16×16-px map: (0,0)=tile0 bank0, (1,0)=tile1 bank1, (0,1)=tile2.
    let entries = [
        entry(0, false, false, 0),
        entry(1, true, false, 0),
        entry(2, false, true, 0),
        0,
    ];
    let screen = store.add_screen(16, 16, &entries);
    let palette = store.add_palette(
        true,
        &[
            [0, 0, 0, 255],
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
        ],
    );
    let mut e = EngineFrame::default();
    e.bgs[0] = BgLayer {
        enabled: true,
        screen: Some(screen),
        palette: Some(palette),
        scroll_x: 4,
        scroll_y: 2,
        priority: 0,
        ..BgLayer::default()
    };
    e.char_blocks[0] = Some(tiles);
    e.blend = Blend {
        plane1: plane::BG0,
        effect: BlendEffect::Alpha,
        plane2: plane::BD,
        eva: 12,
        ebv: 4,
    };
    e.backdrop = 0x03E0; // GX_RGB green 31
    e.brightness = MasterBrightness {
        mode: BrightnessMode::Down,
        value: 4,
    };
    let frame = LogicalFrame {
        main: e,
        ..LogicalFrame::default()
    };

    let [main, sub] = render(&frame, &store);
    let hex = |screen: &ScreenBuffer| {
        let flat = screen
            .as_rgba()
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<u8>>();
        let mut hasher = Sha1::new();
        hasher.update(&flat);
        format!("{:x}", hasher.finalize())
    };
    // Hand-checked before pinning, e.g. (0,0): red over the green
    // backdrop → (191, 63, 0), dimmed by 1/16 → (144, 48, 0).
    assert_eq!(main.pixel(0, 0), [144, 48, 0, 255]);
    assert_eq!(
        hex(&main),
        "d3835635d347cd6bd0f5bbc22eda17701b1ec6a6",
        "engine A golden hash"
    );
    assert_eq!(
        hex(&sub),
        "31c8daa3770fde1d76332d8aac4f53357adc1ce6",
        "engine B golden hash (empty, undimmed black)"
    );
}
