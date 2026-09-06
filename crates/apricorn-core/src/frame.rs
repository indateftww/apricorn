//! The logical frame model — the 2D hardware state as pure data.
//!
//! One [`LogicalFrame`] is the complete observable video state at one
//! tick: both engines' BG layer configuration, their blend and
//! brightness units, the backdrop colors, and which engine drives
//! which LCD. No pixel data lives here — layers *reference* loaded
//! assets through [`AssetId`] handles that the asset store (see
//! `apicorn_core::assets`, Phase 3 step 4) resolves at raster time,
//! so a frame is plain integers end to end: comparable with `==`,
//! hashable, and serializable for the harness without touching a ROM.
//!
//! The field geometry mirrors the hardware study (`docs/nds-2d.md`,
//! which cites the vendored NitroSDK headers and pret usage per fact):
//! engines A (MAIN, `0x04000000`) and B (SUB, `0x04001000`), four text
//! BG layers each under `BGxCNT`, one blend unit (`BLDCNT`/`BLDALPHA`)
//! and one master-brightness unit per engine.

/// A handle to one loaded asset in the engine's asset store.
///
/// Opaque by design: the store assigns these when a layer's tiles,
/// screen, or palette is loaded (Phase 3 step 4), and the rasterizer
/// (Phase 3 step 5) is the only consumer that resolves them. A frame
/// carries the handle and not the bytes, which keeps [`LogicalFrame`]
/// pure data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetId(u32);

impl AssetId {
    /// The first handle the store hands out.
    pub const FIRST: Self = Self(0);

    /// The next handle in sequence (the store's assignment order).
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// The raw store index.
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// The handle at `index` — the inverse of [`AssetId::index`], used
    /// by the store to hand out sequential handles in load order.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self(index as u32)
    }
}

/// Which engine drives which LCD — `GX_SetDispSelect` (`reg_GX_POWCNT`
/// bit 15), the register the game's `screensFlipped` idiom writes.
///
/// The rasterizer is display-agnostic: it renders engine A and engine
/// B; the presenter (or dump tool) maps them to LCDs using this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplaySelect {
    /// `GX_DISP_SELECT_MAIN_SUB` (1): engine A on top, engine B on
    /// the touch LCD. The console's resting state.
    #[default]
    MainOnTop,
    /// `GX_DISP_SELECT_SUB_MAIN` (0): engine B on top, engine A on the
    /// touch LCD — HeartGold's title and intro state (`GfGfx_SwapDisplay`
    /// with `screensFlipped`).
    SubOnTop,
}

/// A text BG layer's color mode — `BGxCNT` bit 7
/// (`GX_BG_COLORMODE_16` / `GX_BG_COLORMODE_256`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// 4bpp: 16-color banks, the screen entry's bits 12–15 select one.
    #[default]
    Bpp4,
    /// 8bpp: the full 256-color engine palette, entry palette bits ignored.
    Bpp8,
}

/// A text BG layer's map size — `BGxCNT` bits 13–14
/// (`GX_BG_SCRSIZE_TEXT_*`).
///
/// Sizes above 256×256 store the map as 2 KiB 32×32 blocks: 512-wide
/// sizes read their left/right block pairs per row; 512-tall sizes
/// stack top/bottom block pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScreenSize {
    /// 256×256 (`GX_BG_SCRSIZE_TEXT_256x256`).
    #[default]
    W256xH256,
    /// 512×256 (`GX_BG_SCRSIZE_TEXT_512x256`).
    W512xH256,
    /// 256×512 (`GX_BG_SCRSIZE_TEXT_256x512`).
    W256xH512,
    /// 512×512 (`GX_BG_SCRSIZE_TEXT_512x512`).
    W512xH512,
}

impl ScreenSize {
    /// The layer's map width in 8×8 tiles (32 or 64).
    #[must_use]
    pub fn tiles_wide(self) -> u16 {
        match self {
            Self::W256xH256 | Self::W256xH512 => 32,
            Self::W512xH256 | Self::W512xH512 => 64,
        }
    }

    /// The layer's map height in 8×8 tiles (32 or 64).
    #[must_use]
    pub fn tiles_tall(self) -> u16 {
        match self {
            Self::W256xH256 | Self::W512xH256 => 32,
            Self::W256xH512 | Self::W512xH512 => 64,
        }
    }
}

/// One text BG layer — the parts of `BGxCNT` plus the scroll registers
/// (`BGxHOFS`/`BGxVOFS`) that the rasterizer needs.
///
/// `char_base` indexes [`EngineFrame::char_blocks`], not VRAM: the
/// model names slots, so layers that share a block (the copyright
/// beat's SUB BG0 and BG1 share one charbase) see the same tile data
/// without emulating banking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BgLayer {
    /// Whether the layer is shown (`G2_BG0_ON`/…/`G2S_BG3_ON`).
    pub enabled: bool,
    /// Which char-block slot (0–7) holds this layer's 8×8 tiles.
    pub char_base: u8,
    /// The layer's screen (map) asset.
    pub screen: Option<AssetId>,
    /// The palette the layer decodes against (a per-layer reference:
    /// both Phase 3 apps point every layer at one shared palette).
    pub palette: Option<AssetId>,
    /// 4bpp or 8bpp (`BGxCNT` bit 7).
    pub color_mode: ColorMode,
    /// Map size (`BGxCNT` bits 13–14).
    pub size: ScreenSize,
    /// Horizontal scroll (raw `BGxHOFS`, 0–511).
    pub scroll_x: u16,
    /// Vertical scroll (raw `BGxVOFS`, 0–511).
    pub scroll_y: u16,
    /// Draw priority 0–3 (`BGxCNT` bits 0–1): lower draws on top;
    /// ties go to the lower-numbered BG.
    pub priority: u8,
}

impl Default for BgLayer {
    fn default() -> Self {
        Self {
            enabled: false,
            char_base: 0,
            screen: None,
            palette: None,
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH256,
            scroll_x: 0,
            scroll_y: 0,
            priority: 0,
        }
    }
}

/// The blend plane mask bits — `BLDCNT` bits 0–5 (first target) and
/// 8–13 (second target); the SDK's `GXBlendPlaneMask` values.
pub mod plane {
    /// Engine BG0.
    pub const BG0: u8 = 0x01;
    /// Engine BG1.
    pub const BG1: u8 = 0x02;
    /// Engine BG2.
    pub const BG2: u8 = 0x04;
    /// Engine BG3.
    pub const BG3: u8 = 0x08;
    /// The OBJ (sprite) plane — always empty in Phase 3 (no sprites).
    pub const OBJ: u8 = 0x10;
    /// The backdrop.
    pub const BD: u8 = 0x20;
}

/// The blend effect in progress — `BLDCNT` bits 6–7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendEffect {
    /// No blending.
    #[default]
    None,
    /// Alpha blend: first target weighted EVA, second EBV.
    Alpha,
}

/// One engine's alpha-blend unit — `BLDCNT`/`BLDALPHA`.
///
/// HeartGold's modeled beats only ever use the alpha effect; the
/// fade-to-white/black effects of bits 6–7 are driven through master
/// brightness ([`MasterBrightness`]) in both apps, so the model
/// carries just `Alpha` for now — extending the enum is additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Blend {
    /// The first-target plane mask (`plane::*` bits).
    pub plane1: u8,
    /// The effect (none, or alpha blend).
    pub effect: BlendEffect,
    /// The second-target plane mask (`plane::*` bits).
    pub plane2: u8,
    /// First-target weight, `BLDALPHA` bits 0–4 (0–31).
    pub eva: u8,
    /// Second-target weight, `BLDALPHA` bits 8–12 (0–31).
    pub ebv: u8,
}

/// One engine's master-brightness unit — `MASTER_BRIGHT`
/// (`0x0400006C` engine A, `0x0400106C` engine B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MasterBrightness {
    /// The direction of the effect (`E_MOD` bits 14–15): disabled,
    /// toward white (up), or toward black (down).
    pub mode: BrightnessMode,
    /// The weight, `value` bits 0–4 — 0–16 in practice (31 max).
    pub value: u8,
}

/// The master-brightness mode — `E_MOD` bits 14–15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrightnessMode {
    /// Mode 0: no effect (`SetMasterBrightnessNeutral`).
    #[default]
    Disabled,
    /// Mode 1: fade toward white.
    Up,
    /// Mode 2: fade toward black.
    Down,
}

/// One 2D engine's state — four text BG layers over the blend and
/// brightness units, plus the named char-block slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineFrame {
    /// The engine's four BG layers, BG0 through BG3. Engine A's BG0
    /// doubles as the 3D core's framebuffer when bound to a model
    /// (`GX_BG0_AS_3D`) — Phase 3 renders it absent (transparent),
    /// which a disabled layer already expresses.
    pub bgs: [BgLayer; 4],
    /// The char-block slots (indices 0–7) that `char_base` names —
    /// each an optional tile-set asset. Layers share a slot by naming
    /// the same index.
    pub char_blocks: [Option<AssetId>; 8],
    /// The engine's alpha-blend unit.
    pub blend: Blend,
    /// The engine's master-brightness unit.
    pub brightness: MasterBrightness,
    /// The backdrop ("mask") color — raw BGR555 (`GX_RGB` layout:
    /// r bits 0–4, g 5–9, b 10–14); the color shown wherever no layer
    /// covers the pixel.
    pub backdrop: u16,
}

/// The complete observable video state at one tick.
///
/// Engine A (MAIN) and engine B (SUB) each render a 256×192 screen;
/// [`LogicalFrame::display`] says which LCD each drives. The frame
/// after a tick is what the rasterizer turns into pixels and what
/// frame-indexed tests (and, later, the harness) compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogicalFrame {
    /// Engine A (MAIN, register base `0x04000000`).
    pub main: EngineFrame,
    /// Engine B (SUB, register base `0x04001000`).
    pub sub: EngineFrame,
    /// The engine→LCD mapping (`GX_SetDispSelect`).
    pub display: DisplaySelect,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_frame_is_both_engines_dark() {
        // The power-on shape: everything off, black backdrop, MAIN on
        // top — the state every app starts its layer setup from.
        let frame = LogicalFrame::default();
        assert_eq!(frame.display, DisplaySelect::MainOnTop);
        for engine in [&frame.main, &frame.sub] {
            assert_eq!(engine.backdrop, 0, "black backdrop");
            assert_eq!(engine.blend, Blend::default());
            assert_eq!(engine.brightness, MasterBrightness::default());
            assert!(engine.bgs.iter().all(|bg| !bg.enabled));
            assert!(engine.char_blocks.iter().all(|block| block.is_none()));
        }
    }

    #[test]
    fn screen_sizes_cover_the_text_layouts() {
        let cases = [
            (ScreenSize::W256xH256, 32, 32),
            (ScreenSize::W512xH256, 64, 32),
            (ScreenSize::W256xH512, 32, 64),
            (ScreenSize::W512xH512, 64, 64),
        ];
        for (size, wide, tall) in cases {
            assert_eq!(size.tiles_wide(), wide);
            assert_eq!(size.tiles_tall(), tall);
        }
    }

    #[test]
    fn asset_ids_sequence_like_the_store_assigns() {
        // The store hands out FIRST then next() in load order; frames
        // compare and hash by handle value.
        let a = AssetId::FIRST;
        let b = a.next().next();
        assert_eq!(b.index(), 2);
        assert_ne!(a, b);
    }

    #[test]
    fn layers_sharing_a_char_base_share_the_slot() {
        // The copyright beat's SUB BG0 and BG1 read the same tiles:
        // same char_base, one slot, two screen references.
        let mut sub = EngineFrame::default();
        sub.char_blocks[4] = Some(AssetId::FIRST);
        sub.bgs[0].char_base = 4;
        sub.bgs[0].screen = Some(AssetId::FIRST.next());
        sub.bgs[1].char_base = 4;
        sub.bgs[1].screen = Some(AssetId::FIRST.next().next());
        assert_eq!(sub.bgs[0].char_base, sub.bgs[1].char_base);
        assert_eq!(
            sub.char_blocks[4],
            sub.char_blocks[sub.bgs[1].char_base as usize]
        );
    }

    #[test]
    fn plane_mask_bits_match_the_sdk() {
        // GXBlendPlaneMask (docs/nds-2d.md): the six bits, in order.
        assert_eq!(plane::BG0, 0x01);
        assert_eq!(plane::BG1, 0x02);
        assert_eq!(plane::BG2, 0x04);
        assert_eq!(plane::BG3, 0x08);
        assert_eq!(plane::OBJ, 0x10);
        assert_eq!(plane::BD, 0x20);
    }
}
