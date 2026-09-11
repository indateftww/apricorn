//! ROM asset loader — the boot screens' assets, straight from the dump.
//!
//! [`AssetStore::open`] reads the pinned retail dump and hands out
//! [`AssetId`](crate::frame::AssetId) handles as apps load members. A
//! member's bytes travel the same path the converter wrote to disk —
//! LZ77-10 when compressed, the NCGR/NCLR/NSCR parsers, then
//! [`cache::encode_tiles`]/[`cache::encode_palette`]/[`cache::encode_screen`]
//! and the decoded chunk readers — so the runtime sees exactly what
//! `apicorn-tools convert` produced (no cache-format skew), without the
//! cache ever needing to exist on disk.
//!
//! The [`copyright_beat`] and [`title_screen`] tables pin *which* members
//! the boot scenes load, with pret citations;
//! `tests/assets_hg.rs` decodes them against the ROM so member drift
//! fails loudly. The rasterizer (`apicorn-gfx`, Phase 3 step 5) is the
//! only consumer that resolves the handles back to chunk data.
//!
//! Placement notes: the model defers extended palettes
//! (`docs/nds-2d.md`), and both boot scenes load their palettes into the
//! regular engine BG palette RAM at slot 0 — so a palette handle alone
//! names a layer's colors. A PMCP patch table would be applied at
//! placement ([`AssetStore::placed_palette`]); every boot palette carries
//! none (pinned by the ROM test).

use std::borrow::Cow;
use std::fmt;
use std::path::Path;

use crate::cache;
use crate::font::Font;
use crate::formats::{MsgBank, Nanr, Narc, Ncer, Ncgr, Nclr, Nscr};
use crate::frame::AssetId;
use crate::nds::{NdsError, NdsRom, Overlay, blz, lz10};

/// The copyright beat's assets — pret `src/intro_movie_scene_1.c`,
/// `IntroMovie_Scene1_LoadBgGfx` (HeartGold arm), members of
/// `NARC_demo_opening_gs_opening`.
pub mod copyright_beat {
    /// The scene's NARC in NitroFS (`a/2/6/2`).
    pub const NARC: &str = "a/2/6/2";

    /// MAIN BG0 (the copyright text): `_00000005_NCGR_lz`.
    pub const MAIN_BG0_CHAR: usize = 5;
    /// MAIN BG0's screen: `_00000013_NSCR` (the one uncompressed map).
    pub const MAIN_BG0_SCREEN: usize = 13;
    /// SUB BG1 (the Game Freak logo): `_00000004_NCGR_lz`.
    pub const SUB_BG1_CHAR: usize = 4;
    /// SUB BG1's screen: `_00000012_NSCR_lz`.
    pub const SUB_BG1_SCREEN: usize = 12;
    /// SUB BG0's screen (`_00000014_NSCR_lz`) — the blank cover that
    /// shares SUB BG1's char block.
    pub const SUB_BG0_SCREEN: usize = 14;

    /// SUB BG palette: `_00000000_NCLR` → SUB BG slot 0, `szByte` 0x140.
    pub const SUB_PALETTE: usize = 0;
    /// MAIN BG palette: `_00000001_NCLR` → MAIN BG slot 0, `szByte` 0x140.
    pub const MAIN_PALETTE: usize = 1;
    /// The scene's palette load size (both engines): 0x140 bytes —
    /// 160 colors, banks 0–9. The screens the beat shows never
    /// reference a bank beyond it (pinned by the ROM test), so the
    /// partial load is observationally a full one.
    pub const PAL_LOAD_BYTES: usize = 0x140;

    // Loaded by the scene but first *shown* only at APPEAR_BG_IMAGE —
    // after the Game Freak hold, beyond the Phase 3 copyright-beat
    // scope. Kept in the table for fidelity; nothing references them.
    /// SUB BG3 char: `_00000006_NCGR_lz` (sunrise layers, out of scope).
    pub const SUB_BG3_CHAR: usize = 6;
    /// SUB BG3 screen: `_00000015_NSCR_lz`.
    pub const SUB_BG3_SCREEN: usize = 15;
    /// MAIN BG3 char: `_00000007_NCGR_lz`.
    pub const MAIN_BG3_CHAR: usize = 7;
    /// MAIN BG3 screen: `_00000018_NSCR_lz`.
    pub const MAIN_BG3_SCREEN: usize = 18;
    /// MAIN BG2 screen: `_00000017_NSCR_lz`.
    pub const MAIN_BG2_SCREEN: usize = 17;
    /// MAIN BG1 screen: `_00000016_NSCR_lz`.
    pub const MAIN_BG1_SCREEN: usize = 16;
}

/// The title screen's assets — pret `src/title_screen.c`,
/// `TitleScreenAnim_Load2dBgGfx` (HeartGold arm), members of
/// `NARC_demo_title_titledemo`.
///
/// Engine A BG0 (the Ho-Oh NSBMD 3D model) and the "TOUCH TO START"
/// text window are Phase 6/4 deferrals and have no members here.
pub mod title_screen {
    /// The screen's NARC in NitroFS (`a/0/4/6`).
    pub const NARC: &str = "a/0/4/6";

    /// SUB BG1 (static art, 4bpp): `NARC_titledemo_titledemo_00000015_NCGR`.
    pub const SUB_BG1_CHAR: usize = 15;
    /// SUB BG1's screen: `_00000017_NSCR`.
    pub const SUB_BG1_SCREEN: usize = 17;
    /// SUB BG2 (the game logo, 8bpp): `_00000003_NCGR`.
    pub const SUB_BG2_CHAR: usize = 3;
    /// SUB BG2's screen: `_00000000_NSCR`.
    pub const SUB_BG2_SCREEN: usize = 0;
    /// SUB BG3 (version art, 8bpp): `_00000034_NCGR`.
    pub const SUB_BG3_CHAR: usize = 34;
    /// SUB BG3's screen: `_00000035_NSCR`.
    pub const SUB_BG3_SCREEN: usize = 35;

    /// SUB BG palette: `NARC_titledemo_titledemo_00000004_NCLR` → SUB BG
    /// slot 0. pret also loads it into the SUB BG *extended* palette RAM
    /// at 0x4000 for the logo; extended palettes are deferred
    /// (`docs/nds-2d.md`), and with the ext mode off the logo reads the
    /// regular load — which this is.
    pub const SUB_PALETTE: usize = 4;
    /// MAIN BG palette: `NARC_titledemo_titledemo_00000013_NCLR` → MAIN
    /// BG slot 0.
    pub const MAIN_PALETTE: usize = 13;
    /// The screen's palette load size (both engines): `szByte` 0 — the
    /// whole file, 0x200 bytes.
    pub const PAL_LOAD_BYTES: usize = 0x200;
}

/// The font archive — pret `src/font.c`'s `sFontArcParam` over
/// `NARC_graphic_font`. Members 0–4 are fonts 0–4, member 0xA is
/// font 5; every HGSS font is variable-width (the FALSE column).
pub mod font_narc {
    /// The archive in NitroFS (`a/0/1/6`).
    pub const NARC: &str = "a/0/1/6";
    /// Font 0 — the button/menu font (main menu, Oak full-screen text).
    pub const FONT0: usize = 0;
    /// Font 1 — the dialog font (message windows).
    pub const FONT1: usize = 1;
    /// Font 2 — the naming screen's entry font.
    pub const FONT2: usize = 2;
    /// Font 3.
    pub const FONT3: usize = 3;
    /// Font 4 — the Oak speech's multichoice-button font.
    pub const FONT4: usize = 4;
    /// Font 5 — the PBR menu font.
    pub const FONT5: usize = 0xA;
    /// The {YESNO} focus-indicator NCGR — pret `text.c`
    /// `LoadScreenFocusIndicatorGraphics`
    /// (`GfGfxLoader_GetCharData(NARC_graphic_font, 6)`): 48 4bpp
    /// tiles, four 384-byte frames of twelve.
    pub const FOCUS_INDICATOR: usize = 6;
    /// `LoadFontPal0`'s palette: member 7, 0x20 bytes (16 colors).
    pub const PAL0: usize = 7;
    /// `LoadFontPal1`'s palette: member 8, 0x20 bytes.
    pub const PAL1: usize = 8;
    /// The palette load size (`LoadFontPal0/1`): 16 colors.
    pub const PAL_LOAD_BYTES: usize = 0x20;
}

/// The message banks the boot-to-new-game flow reads — all members
/// of `NARC_msgdata_msg` (`a/0/2/7`), one [`crate::cache::Text`] per
/// bank, each id pinned by the flow's pret source:
///
/// | Bank | pret source |
/// |---|---|
/// | 17 | `ov74` main-menu new-game warning (`src/application/main_menu/main_menu.c`) |
/// | 442 | `ov74` main-menu buttons (`src/application/main_menu/main_menu.c`) |
/// | 197 | naming-screen battle message (`src/naming_screen.c`) |
/// | 219 | Oak speech (`src/oaks_speech.c`) |
/// | 229 | save-corruption warnings (`src/check_savedata.c`) |
/// | 249 | naming-screen UI (`src/naming_screen.c`) |
/// | 254 | naming-screen default names (`src/naming_screen.c`) |
pub mod msg_narc {
    /// The archive in NitroFS (`a/0/2/7`).
    pub const NARC: &str = "a/0/2/7";
}

/// The user-frame graphics — pret `asm/render_window.s`
/// `LoadUserFrameGfx1/2`. Gfx1 picks NCGR member 0 for frame id 0,
/// member 1 for any other, NCLR member 0x19 (member 0x2E for frame
/// id 2); Gfx2 decodes through the tiny helpers `sub_0200E63C`
/// (`:364`, member = frame + 2) and `sub_0200E640` (`:370`, member
/// = frame + 0x1A), so frame id 0's NCGR is member 2 and its NCLR
/// member 0x1A. Both load 0x20 bytes of palette into the caller's
/// bank.
pub mod frame_narc {
    /// The archive in NitroFS (`a/0/3/8`).
    pub const NARC: &str = "a/0/3/8";
    /// The default frame's tiles (`LoadUserFrameGfx1`, frame id 0).
    pub const FRAME0_CHAR: usize = 0;
    /// Any other frame's tiles (`LoadUserFrameGfx1`).
    pub const FRAME1_CHAR: usize = 1;
    /// Frame id 0's tiles through `LoadUserFrameGfx2` — member
    /// frame + 2 (`sub_0200E63C`).
    pub const GFX2_FRAME0_CHAR: usize = 2;
    /// Three 16x16 down-arrow poses (`sub_0200EB68`, member 0x16).
    pub const DOWN_ARROW_CHAR: usize = 0x16;
    /// The frames' 16-color palette (`LoadUserFrameGfx1`, frame
    /// id 2 uses member 0x2E).
    pub const PALETTE: usize = 0x19;
    /// Frame id 2's palette (`LoadUserFrameGfx1`).
    pub const PALETTE2: usize = 0x2E;
    /// Frame id 0's palette through `LoadUserFrameGfx2` — member
    /// frame + 0x1A (`sub_0200E640`).
    pub const GFX2_FRAME0_PALETTE: usize = 0x1A;
}

/// The Oak-speech intro graphics — pret `src/oaks_speech.c`, members
/// of `NARC_demo_intro_intro` (`a/1/2/0`).
///
/// The SoulSilver siblings (`LoadButtonTutorialGfx`'s version branch:
/// MAIN NCLR 2, SUB NCLR 31) are not loaded — this port pins
/// HeartGold, as every scene before it does.
pub mod intro_narc {
    /// The archive in NitroFS (`a/1/2/0`).
    pub const NARC: &str = "a/1/2/0";

    /// MAIN BG3's char: the button-tutorial NCGR
    /// (`LoadButtonTutorialGfx`, `oaks_speech.c:1162`).
    pub const BUTTON_TUTORIAL_MAIN_CHAR: usize = 0;
    /// SUB BG3's char (`oaks_speech.c:1164`).
    pub const BUTTON_TUTORIAL_SUB_CHAR: usize = 32;
    /// MAIN BG palette: HG branch, `@MAIN 0` size `0x60` — 48 colors
    /// (`oaks_speech.c:1166`, `:1172`).
    pub const MAIN_PALETTE: usize = 1;
    /// SUB BG palette: HG branch, `@SUB 0` size `0xA0` — 80 colors
    /// (`oaks_speech.c:1167`, `:1173`).
    pub const SUB_PALETTE: usize = 30;
    /// The button-tutorial screen layouts `sButtonTutorialNSCR`
    /// (`oaks_speech.c:309-316`), MAIN BG3 — six of them; the scene's
    /// `SetButtonTutorialScreenLayout` picks by index.
    pub const BUTTON_TUTORIAL_SCREENS: [usize; 6] = [3, 4, 5, 6, 7, 8];
    /// The Oak-pic screen (`DrawPicOnBgLayer`'s screen member 9,
    /// `oaks_speech.c:1210`), MAIN BG1/BG2.
    pub const PIC_SCREEN: usize = 9;

    /// Oak's char (`sBgPicNCGR_NCLR` row 1, `oaks_speech.c:496-503`).
    pub const OAK_CHAR: usize = 10;
    /// Oak's palette.
    pub const OAK_PALETTE: usize = 11;
    /// Ethan's chars, rows 2–5 (`oaks_speech.c:505-523`).
    pub const ETHAN_CHARS: [usize; 4] = [12, 13, 14, 15];
    /// Ethan's palette.
    pub const ETHAN_PALETTE: usize = 16;
    /// Lyra's chars, rows 6–9 (`oaks_speech.c:525-543`).
    pub const LYRA_CHARS: [usize; 4] = [17, 18, 19, 20];
    /// Lyra's palette.
    pub const LYRA_PALETTE: usize = 21;

    /// The shrink anim's post-pic frames, male
    /// (`sPlayerPicShrinkGfx_Male`, `oaks_speech.c:333-340` — index 0
    /// is member 12, Ethan 1, already on screen from `DrawPic`).
    pub const SHRINK_MALE_CHARS: [usize; 4] = [22, 23, 24, 25];
    /// The shrink anim's post-pic frames, female
    /// (`sPlayerPicShrinkGfx_Female`, `oaks_speech.c:342-349` —
    /// index 0 is member 17, Lyra 1).
    pub const SHRINK_FEMALE_CHARS: [usize; 4] = [26, 27, 28, 29];

    /// SUB BG3's screens `ov53_021E8558` (`oaks_speech.c:301-307`),
    /// indexed by the layout id `ov53_021E67C4`'s callers pass:
    /// 0→44, 1→43, 2→43, 3→45, 4→51.
    pub const SUB3_SCREENS: [usize; 5] = [44, 43, 43, 45, 51];
    /// SUB BG2's char (member 37 — `ov53_021E8584`'s every row,
    /// `oaks_speech.c:318-331`, and the multichoice flash's member
    /// 42 below).
    pub const SUB2_CHAR: usize = 37;
    /// SUB BG2's screens `ov53_021E8584` (`ov53_021E6824`'s rows):
    /// 0→47, 1→48, 2→46.
    pub const SUB2_SCREENS: [usize; 3] = [47, 48, 46];
    /// SUB BG2's palette: NCLR member 33 `@SUB 0xE0` size `0x60` —
    /// bank 7 (`ov53_021E6824`, `oaks_speech.c`).
    pub const SUB2_PALETTE: usize = 33;
    /// The multichoice cursor-flash char (`InitMultichoiceMenuWithFrameFlash`,
    /// SUB BG1 member 42).
    pub const MULTICHOICE_FLASH_CHAR: usize = 42;
    /// The multichoice screens `sMultichoiceMenuParam`
    /// (`oaks_speech.c:477-494`), SUB BG1, by menu id: menu 0 the
    /// 3-choice tutorial button list, menus 1–2 the 2-choice info
    /// lists, menu 3 the 2-choice gender-adjacent pair.
    pub const MULTICHOICE_SCREENS: [usize; 4] = [49, 50, 50, 52];
}

/// The Oak-speech yes/no menu's graphics — pret
/// `src/oaks_speech_yesnomenu.c:55-68`, members of `NARC_a_2_3_7`
/// (`a/2/3/7`): palette 0 (`@SUB 32*palette`, 16 colors), char 1,
/// screen 10.
pub mod yesno_narc {
    /// The archive in NitroFS (`a/2/3/7`).
    pub const NARC: &str = "a/2/3/7";
    /// The menu's 16-color palette.
    pub const PALETTE: usize = 0;
    /// The menu's char (SUB BG2).
    pub const CHAR: usize = 1;
    /// The menu's screen (SUB BG2).
    pub const SCREEN: usize = 10;
}

/// Failures while opening the dump or loading an asset.
#[derive(Debug)]
pub enum AssetsError {
    /// Reading the ROM file from disk failed.
    Io(std::io::Error),
    /// The dump's SHA-1 is not [`AssetStore::PINNED_SHA1`] — the asset
    /// tables are only valid for the pinned retail ROM.
    WrongRom {
        /// The SHA-1 actually found, lowercase hex.
        got: String,
    },
    /// A table lookup missed (no such NitroFS path, no such member).
    Missing(String),
    /// A parse or decompression failed; `what` names the member.
    Corrupt {
        /// The failing member, as `narc#id`.
        what: String,
        /// The underlying parse error.
        source: NdsError,
    },
}

impl fmt::Display for AssetsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "reading the ROM: {err}"),
            Self::WrongRom { got } => {
                write!(f, "not the pinned retail dump (SHA-1 {got})")
            }
            Self::Missing(what) => write!(f, "no such asset: {what}"),
            Self::Corrupt { what, source } => write!(f, "{what}: {source}"),
        }
    }
}

impl std::error::Error for AssetsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Corrupt { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// One loaded asset, indexed by its [`AssetId`](crate::frame::AssetId).
enum Asset {
    /// Expanded tile pixels (a decoded Tiles chunk).
    Tiles(cache::Tiles),
    /// A palette: the decoded chunk plus its colors in VRAM slot order
    /// (PMCP applied — identity when the source carried no table).
    Palette {
        /// The decoded chunk (provenance).
        chunk: cache::Palette,
        /// The placed colors the rasterizer indexes against.
        placed: Vec<[u8; 4]>,
    },
    /// Raw screen map entries (a decoded Screen chunk).
    Screen(cache::Screen),
    /// A decoded font (glyph levels + width table).
    Font(Font),
    /// A decoded message bank (a Text chunk).
    Text(cache::Text),
    /// Sprite cell geometry.
    Cells(cache::Cells),
    /// Sprite animation sequences.
    Animation(cache::Animation),
}

/// The boot scenes' asset source: a parsed retail dump plus the assets
/// loaded from it so far.
///
/// Handles are handed out sequentially in load order —
/// [`AssetId::FIRST`], then `next()` — and resolve through the store
/// only; a frame carries the handles and never the bytes.
pub struct AssetStore {
    /// The whole cart image. `NdsRom` borrows its input, so the store
    /// re-parses the (cheap, metadata-only) header and filesystem per
    /// load call rather than holding a self-referential struct.
    rom: Vec<u8>,
    /// The assets loaded so far, in [`AssetId`](crate::frame::AssetId)
    /// order.
    assets: Vec<Asset>,
}

impl AssetStore {
    /// Expanded executable data for ROM-resident lookup tables. The store's
    /// SHA-1 gate ensures that callers' retail addresses match this image.
    pub fn arm9_image(&self) -> Result<Vec<u8>, AssetsError> {
        let decode = || -> Result<Vec<u8>, NdsError> {
            Ok(NdsRom::parse(&self.rom)?.arm9_image()?.into_owned())
        };
        decode().map_err(|source| AssetsError::Corrupt {
            what: "ARM9 lookup tables".into(),
            source,
        })
    }
    /// RAM address the ARM9 image loads to (0x02000000 on HeartGold) —
    /// the base that pinned table addresses such as
    /// [`crate::field::map_header::MAP_HEADERS_ADDRESS`] are relative to.
    pub fn arm9_base(&self) -> u32 {
        // `open` validated the header, so re-parsing cannot fail.
        NdsRom::parse(&self.rom).map_or(0x0200_0000, |rom| rom.header.arm9.ram_address)
    }
    /// Loads the default dialogue frame and builds its down-arrow tiles.
    ///
    /// `sub_0200EA68` repeats frame tiles 10/11 under each 16x16 pose,
    /// then blits member 22 with source X=3, width=13 and color key 0.
    /// The three resulting poses occupy tiles 18..30 after the border.
    ///
    /// # Errors
    /// Returns an error if either ROM member is missing or malformed.
    pub fn load_default_dialogue_frame(&mut self) -> Result<AssetId, AssetsError> {
        let narc = frame_narc::NARC;
        let bytes = self.member(narc, frame_narc::GFX2_FRAME0_CHAR)?;
        let mut border = decode_tiles(&bytes, narc, frame_narc::GFX2_FRAME0_CHAR)?;
        let bytes = self.member(narc, frame_narc::DOWN_ARROW_CHAR)?;
        let arrow = decode_tiles(&bytes, narc, frame_narc::DOWN_ARROW_CHAR)?;
        if !border.is_4bpp()
            || border.tile_count() != 18
            || !arrow.is_4bpp()
            || arrow.tile_count() < 12
        {
            return Err(AssetsError::Corrupt {
                what: "default dialogue frame / down arrow".to_owned(),
                source: NdsError::Invalid {
                    what: "expected 18 border tiles and three 16x16 arrow poses",
                },
            });
        }
        let mut poses = vec![0; 12 * 64];
        for y in 0..48 {
            for x in 0..16 {
                let at = ((y / 8) * 2 + x / 8) * 64 + (y % 8) * 8 + x % 8;
                let background = border.pixels()[(10 + x / 8) * 64 + (y % 8) * 8 + x % 8];
                let ink = if x < 13 {
                    let source_x = x + 3;
                    arrow.pixels()[((y / 8) * 2 + source_x / 8) * 64 + (y % 8) * 8 + source_x % 8]
                } else {
                    0
                };
                poses[at] = if ink == 0 { background } else { ink };
            }
        }
        border.append_tiles(&poses);
        Ok(self.push(Asset::Tiles(border)))
    }

    /// Loads a sprite's NCER cell bank from the ROM.
    ///
    /// # Errors
    /// Returns an error for missing or malformed cell data.
    pub fn load_cells(&mut self, narc: &str, member: usize) -> Result<AssetId, AssetsError> {
        let bytes = self.member(narc, member)?;
        let cells = Ncer::parse(&bytes)
            .and_then(|data| cache::Cells::parse(&cache::encode_cells(&data)))
            .map_err(|source| AssetsError::Corrupt {
                what: format!("{narc}#{member}"),
                source,
            })?;
        Ok(self.push(Asset::Cells(cells)))
    }

    /// Loads a sprite's NANR animation bank from the ROM.
    ///
    /// # Errors
    /// Returns an error for missing or malformed animation data.
    pub fn load_animation(&mut self, narc: &str, member: usize) -> Result<AssetId, AssetsError> {
        let bytes = self.member(narc, member)?;
        let animation = Nanr::parse(&bytes)
            .and_then(|data| cache::encode_animation(&data))
            .and_then(|data| cache::Animation::parse(&data))
            .map_err(|source| AssetsError::Corrupt {
                what: format!("{narc}#{member}"),
                source,
            })?;
        Ok(self.push(Asset::Animation(animation)))
    }

    /// Resolves a sprite cell bank.
    #[must_use]
    pub fn cells(&self, id: AssetId) -> Option<&cache::Cells> {
        match self.assets.get(id.index()) {
            Some(Asset::Cells(cells)) => Some(cells),
            _ => None,
        }
    }

    /// Resolves a sprite animation bank.
    #[must_use]
    pub fn animation(&self, id: AssetId) -> Option<&cache::Animation> {
        match self.assets.get(id.index()) {
            Some(Asset::Animation(animation)) => Some(animation),
            _ => None,
        }
    }
    /// The SHA-1 of the retail dump the asset tables are pinned to
    /// (HeartGold US) — the same constant `apicorn-harness` traces and
    /// `apicorn-tools verify` carry.
    pub const PINNED_SHA1: &str = "4fcded0e2713dc03929845de631d0932ea2b5a37";

    /// Opens the retail dump at `path`, refusing any other image: the
    /// member tables are only valid for the pinned ROM.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the file cannot be read, its
    /// SHA-1 does not match [`Self::PINNED_SHA1`], or it does not parse
    /// as an NDS cart.
    pub fn open(path: &Path) -> Result<Self, AssetsError> {
        let rom = std::fs::read(path).map_err(AssetsError::Io)?;
        let got = sha1::hex(&rom);
        if got != Self::PINNED_SHA1 {
            return Err(AssetsError::WrongRom { got });
        }
        // Fail fast on a structurally broken dump, not on first load.
        NdsRom::parse(&rom).map_err(|source| AssetsError::Corrupt {
            what: "ROM".to_owned(),
            source,
        })?;
        Ok(Self {
            rom,
            assets: Vec::new(),
        })
    }

    /// Loads NARC member `member` of the NitroFS archive at `narc_path`
    /// as tile data (an NCGR, LZ77-10-compressed or not).
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the path or member is missing or
    /// the member is not a parseable NCGR.
    pub fn load_tiles(&mut self, narc_path: &str, member: usize) -> Result<AssetId, AssetsError> {
        let bytes = self.member(narc_path, member)?;
        let tiles = decode_tiles(&bytes, narc_path, member)?;
        Ok(self.push(Asset::Tiles(tiles)))
    }

    /// Loads NARC member `member` of the NitroFS archive at `narc_path`
    /// as a palette (an NCLR), applying its PMCP table at placement.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the path or member is missing or
    /// the member is not a parseable NCLR.
    pub fn load_palette(&mut self, narc_path: &str, member: usize) -> Result<AssetId, AssetsError> {
        let bytes = self.member(narc_path, member)?;
        let (chunk, placed) = decode_palette(&bytes, narc_path, member)?;
        Ok(self.push(Asset::Palette { chunk, placed }))
    }

    /// Loads NARC member `member` of the NitroFS archive at `narc_path`
    /// as a screen map (an NSCR).
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the path or member is missing or
    /// the member is not a parseable NSCR.
    pub fn load_screen(&mut self, narc_path: &str, member: usize) -> Result<AssetId, AssetsError> {
        let bytes = self.member(narc_path, member)?;
        let screen = decode_screen(&bytes, narc_path, member)?;
        Ok(self.push(Asset::Screen(screen)))
    }

    /// Loads NARC member `member` of the NitroFS archive at `narc_path`
    /// as a font — the raw `FontHeader` format `font::Font` decodes
    /// (fonts are not Nitro containers, so there is no cache-chunk
    /// step: this parse *is* the conversion).
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the path or member is missing or
    /// the member does not parse as a font.
    pub fn load_font(&mut self, narc_path: &str, member: usize) -> Result<AssetId, AssetsError> {
        let bytes = self.member(narc_path, member)?;
        let font = Font::parse(&bytes).map_err(|source| AssetsError::Corrupt {
            what: format!("{narc_path}#{member}"),
            source,
        })?;
        Ok(self.push(Asset::Font(font)))
    }

    /// Loads NARC member `member` of the NitroFS archive at `narc_path`
    /// as a message bank — a MAT bank through the converter's exact
    /// encode/parse path into a Text chunk.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the path or member is missing or
    /// the member is not a parseable MAT bank.
    pub fn load_msg_bank(
        &mut self,
        narc_path: &str,
        member: usize,
    ) -> Result<AssetId, AssetsError> {
        let bytes = self.member(narc_path, member)?;
        let bank = MsgBank::parse(&bytes).map_err(|source| AssetsError::Corrupt {
            what: format!("{narc_path}#{member}"),
            source,
        })?;
        let chunk = cache::encode_text(&bank);
        let text = cache::Text::parse(&chunk).map_err(|source| AssetsError::Corrupt {
            what: format!("{narc_path}#{member} (chunk)"),
            source,
        })?;
        Ok(self.push(Asset::Text(text)))
    }

    /// The font behind `id`, if it names a Font asset.
    #[must_use]
    pub fn font(&self, id: AssetId) -> Option<&Font> {
        match self.assets.get(id.index()) {
            Some(Asset::Font(font)) => Some(font),
            _ => None,
        }
    }

    /// The message bank behind `id`, if it names a Text asset.
    #[must_use]
    pub fn msg_bank(&self, id: AssetId) -> Option<&cache::Text> {
        match self.assets.get(id.index()) {
            Some(Asset::Text(text)) => Some(text),
            _ => None,
        }
    }

    /// The tile data behind `id`, if it names a Tiles asset.
    #[must_use]
    pub fn tiles(&self, id: AssetId) -> Option<&cache::Tiles> {
        match self.assets.get(id.index()) {
            Some(Asset::Tiles(tiles)) => Some(tiles),
            _ => None,
        }
    }

    /// The palette chunk behind `id`, if it names a Palette asset.
    #[must_use]
    pub fn palette(&self, id: AssetId) -> Option<&cache::Palette> {
        match self.assets.get(id.index()) {
            Some(Asset::Palette { chunk, .. }) => Some(chunk),
            _ => None,
        }
    }

    /// The palette behind `id` *as placed in VRAM*: colors in slot order,
    /// the PMCP patch applied — a 4bpp layer's bank `b` reads slots
    /// `b * 16..b * 16 + 16`, an 8bpp layer indexes all of it directly.
    /// Slots a PMCP table leaves unmapped read transparent (hardware
    /// palette RAM after a clear). Identity when the source carried no
    /// table, which is every Phase 3 boot palette.
    #[must_use]
    pub fn placed_palette(&self, id: AssetId) -> Option<&[[u8; 4]]> {
        match self.assets.get(id.index()) {
            Some(Asset::Palette { placed, .. }) => Some(placed),
            _ => None,
        }
    }

    /// The screen map behind `id`, if it names a Screen asset.
    #[must_use]
    pub fn screen(&self, id: AssetId) -> Option<&cache::Screen> {
        match self.assets.get(id.index()) {
            Some(Asset::Screen(screen)) => Some(screen),
            _ => None,
        }
    }

    /// How many assets the store holds (also the next handle's index).
    #[must_use]
    pub fn len(&self) -> usize {
        self.assets.len()
    }

    /// Whether the store holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    /// The bytes of NARC `narc_path`'s member `member`, LZ77-10-expanded
    /// when the member is compressed.
    ///
    /// The NitroFS carries no compression flag — the game knows per
    /// request (`isCompressed=TRUE`, pret's `FSLoadMemberByPath` calls) —
    /// so the store sniffs, and the sniff is only the `0x10` magic byte
    /// (see [`lz10::is_lz10`]). A sniff hit whose expansion fails, or
    /// expands to nothing, is a false positive rather than a corruption:
    /// every font member starts with `FontHeader.headerSize = 0x10`
    /// (`font_data.c`'s `FontData_Init` reads the header raw from the
    /// member image), which reads as a stream declaring a zero-byte image
    /// — a decomposition that *succeeds*, empty. No stored member is an
    /// empty file, so both verdicts hand back the raw bytes.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the path or member is missing.
    pub(crate) fn member(
        &self,
        narc_path: &str,
        member: usize,
    ) -> Result<Cow<'_, [u8]>, AssetsError> {
        let rom = NdsRom::parse(&self.rom).map_err(|source| AssetsError::Corrupt {
            what: "ROM".to_owned(),
            source,
        })?;
        let narc_bytes = rom
            .file_by_path(narc_path)
            .map_err(|_| AssetsError::Missing(format!("no NitroFS archive {narc_path}")))?;
        let narc = Narc::parse(narc_bytes).map_err(|source| AssetsError::Corrupt {
            what: narc_path.to_owned(),
            source,
        })?;
        narc_member(&narc, narc_path, member)
    }

    /// Parses the NitroFS archive at `narc_path` once so a caller that
    /// walks many members (the field loader reads hundreds of land and
    /// prop members per map) pays the container parse a single time.
    /// Read members through [`narc_member`].
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the path is missing or the file is
    /// not a NARC.
    pub fn narc(&self, narc_path: &str) -> Result<Narc<'_>, AssetsError> {
        let bytes = self.nitrofs_file(narc_path)?;
        Narc::parse(bytes).map_err(|source| AssetsError::Corrupt {
            what: narc_path.to_owned(),
            source,
        })
    }

    /// The number of members in the NitroFS archive at `narc_path` — the
    /// bound a map header's bank fields must respect.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the path is missing or the file is
    /// not a NARC.
    pub fn member_count(&self, narc_path: &str) -> Result<usize, AssetsError> {
        Ok(self.narc(narc_path)?.file_count())
    }

    /// The raw bytes of the NitroFS file at `path` (a loose file such as
    /// `fielddata/maptable/mapname.bin`, or a whole archive).
    ///
    /// # Errors
    /// Returns [`AssetsError::Missing`] when no file has that path.
    pub fn nitrofs_file(&self, path: &str) -> Result<&[u8], AssetsError> {
        let rom = NdsRom::parse(&self.rom).map_err(|source| AssetsError::Corrupt {
            what: "ROM".to_owned(),
            source,
        })?;
        rom.file_by_path(path)
            .map_err(|_| AssetsError::Missing(format!("no NitroFS file {path}")))
    }

    /// An ARM9 overlay as it sits in RAM after loading: the overlay-table
    /// entry (for its load address) and its bytes, BLZ-decompressed when
    /// the table marks it compressed (retail HeartGold compresses every
    /// overlay but 35 and 124 — see [`crate::nds::blz`]). The `.bss`
    /// tail is not appended; callers read `.text`/`.rodata` tables only.
    ///
    /// # Errors
    /// Returns [`AssetsError::Missing`] when the overlay table has no
    /// entry `id`, or [`AssetsError::Corrupt`] when its payload fails
    /// BLZ validation.
    pub fn overlay(&self, id: u32) -> Result<(Overlay, Vec<u8>), AssetsError> {
        let rom = NdsRom::parse(&self.rom).map_err(|source| AssetsError::Corrupt {
            what: "ROM".to_owned(),
            source,
        })?;
        let entry = rom
            .overlays()
            .iter()
            .find(|o| o.id == id)
            .copied()
            .ok_or_else(|| AssetsError::Missing(format!("no ARM9 overlay {id}")))?;
        let corrupt = |source| AssetsError::Corrupt {
            what: format!("ARM9 overlay {id}"),
            source,
        };
        let stored = rom.file(entry.fat_id).map_err(corrupt)?;
        let image = if entry.is_compressed() {
            blz::decompress(stored, entry.raw_size as usize).map_err(corrupt)?
        } else {
            stored.to_vec()
        };
        Ok((entry, image))
    }

    /// The raw bytes of the NitroFS file at `path` (e.g.
    /// `data/area00light.txt`) — pret's `Sys_AllocAndReadFile` by
    /// path: no member indirection and no compression sniff, the file
    /// exactly as the cart stores it.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when no NitroFS file has that path.
    pub fn nitrofs_file(&self, path: &str) -> Result<Vec<u8>, AssetsError> {
        let rom = NdsRom::parse(&self.rom).map_err(|source| AssetsError::Corrupt {
            what: "ROM".to_owned(),
            source,
        })?;
        rom.file_by_path(path)
            .map(<[u8]>::to_vec)
            .map_err(|_| AssetsError::Missing(format!("no NitroFS file {path}")))
    }

    /// Stores `asset`, returning its handle.
    fn push(&mut self, asset: Asset) -> AssetId {
        let id = AssetId::from_index(self.assets.len());
        self.assets.push(asset);
        id
    }
}

/// Member `member` of an already-parsed `narc` (see [`AssetStore::narc`]),
/// expanded exactly as [`AssetStore::member`] would: LZ77-10 sniffed on
/// the `0x10` magic byte, with a failed or empty expansion handing back
/// the raw bytes (see that method's notes on the sniff's false
/// positives). `narc_path` only names the archive in errors.
///
/// # Errors
/// Returns [`AssetsError::Missing`] when the archive has no such member.
pub fn narc_member<'a>(
    narc: &Narc<'a>,
    narc_path: &str,
    member: usize,
) -> Result<Cow<'a, [u8]>, AssetsError> {
    let raw = narc
        .file(member)
        .map_err(|_| AssetsError::Missing(format!("{narc_path} has no member {member}")))?;
    if lz10::is_lz10(raw) {
        match lz10::decompress(raw) {
            Ok(image) if !image.is_empty() => Ok(Cow::Owned(image)),
            _ => Ok(Cow::Borrowed(raw)),
        }
    } else {
        Ok(Cow::Borrowed(raw))
    }
}

/// Decodes an (expanded) member as tile data, through the converter's
/// exact encode/parse path.
fn decode_tiles(bytes: &[u8], narc_path: &str, member: usize) -> Result<cache::Tiles, AssetsError> {
    let ncgr = Ncgr::parse(bytes).map_err(|source| AssetsError::Corrupt {
        what: format!("{narc_path}#{member}"),
        source,
    })?;
    let chunk = cache::encode_tiles(&ncgr);
    cache::Tiles::parse(&chunk).map_err(|source| AssetsError::Corrupt {
        what: format!("{narc_path}#{member} (chunk)"),
        source,
    })
}

/// Decodes an (expanded) member as a palette plus its placed colors.
fn decode_palette(
    bytes: &[u8],
    narc_path: &str,
    member: usize,
) -> Result<(cache::Palette, Vec<[u8; 4]>), AssetsError> {
    let nclr = Nclr::parse(bytes).map_err(|source| AssetsError::Corrupt {
        what: format!("{narc_path}#{member}"),
        source,
    })?;
    let chunk = cache::encode_palette(&nclr);
    let palette = cache::Palette::parse(&chunk).map_err(|source| AssetsError::Corrupt {
        what: format!("{narc_path}#{member} (chunk)"),
        source,
    })?;
    let placed = placed_colors(&palette).map_err(|source| AssetsError::Corrupt {
        what: format!("{narc_path}#{member} (PMCP)"),
        source,
    })?;
    Ok((palette, placed))
}

/// Decodes an (expanded) member as a screen map.
fn decode_screen(
    bytes: &[u8],
    narc_path: &str,
    member: usize,
) -> Result<cache::Screen, AssetsError> {
    let nscr = Nscr::parse(bytes).map_err(|source| AssetsError::Corrupt {
        what: format!("{narc_path}#{member}"),
        source,
    })?;
    let chunk = cache::encode_screen(&nscr);
    cache::Screen::parse(&chunk).map_err(|source| AssetsError::Corrupt {
        what: format!("{narc_path}#{member} (chunk)"),
        source,
    })
}

/// A palette's colors in VRAM slot order — placement with the PMCP
/// patch applied.
///
/// The chunk's table is in slot order ([`cache::Palette::pmcp`]):
/// VRAM slot `i` reuses the stored sub-palette `pmcp[i]`. Slots the
/// table leaves unmapped read `[0, 0, 0, 0]` (transparent black — the
/// state of cleared palette RAM). Without a table (or for a 256-color
/// source, where PMCP does not apply) this is the identity.
fn placed_colors(palette: &cache::Palette) -> Result<Vec<[u8; 4]>, NdsError> {
    let colors = palette.rgba();
    let pmcp = palette.pmcp();
    if !palette.is_16_color() || pmcp.is_empty() {
        return Ok(colors.to_vec());
    }
    apply_pmcp(colors, pmcp)
}

/// The PMCP application itself: `colors` is the stored palette in slot
/// order (16 colors per stored sub-palette), `pmcp` the chunk's patch
/// table in VRAM-slot order.
///
/// The placed palette covers every slot the table patches plus every
/// stored slot — whichever is larger. Slots the table does not name
/// stay cleared, which is what the engine's VRAM holds at boot.
///
/// # Errors
/// Returns an [`NdsError`] when the stored colors are not a whole
/// number of 16-color slots, when the table names more slots than the
/// 16 the VRAM palette has, or when an entry reuses a stored
/// sub-palette beyond the file.
fn apply_pmcp(colors: &[[u8; 4]], pmcp: &[u16]) -> Result<Vec<[u8; 4]>, NdsError> {
    const SLOT: usize = 16;
    if !colors.len().is_multiple_of(SLOT) {
        return Err(NdsError::Invalid {
            what: "stored palette is not a whole number of 16-color slots",
        });
    }
    if pmcp.len() > 16 {
        return Err(NdsError::Invalid {
            what: "PMCP table patches more slots than the 16-color VRAM has",
        });
    }
    let slots = (colors.len() / SLOT).max(pmcp.len());
    let mut placed = vec![[0u8, 0, 0, 0]; slots * SLOT];
    for (slot, &stored) in pmcp.iter().enumerate() {
        let stored = usize::from(stored) * SLOT;
        if stored + SLOT > colors.len() {
            return Err(NdsError::Invalid {
                what: "PMCP table reuses a stored palette beyond the file",
            });
        }
        placed[slot * SLOT..slot * SLOT + SLOT].copy_from_slice(&colors[stored..stored + SLOT]);
    }
    Ok(placed)
}

/// The store's SHA-1, hand-rolled so the headless core stays
/// dependency-free (see `Cargo.toml`); used only for the dump gate.
mod sha1 {
    /// The lowercase hex SHA-1 digest of `data`.
    pub(crate) fn hex(data: &[u8]) -> String {
        let mut h = State::new();
        let whole = data.len() / 64 * 64;
        h.blocks(&data[..whole]);
        // The tail plus the 0x80 bit, zero-padded to ≡ 56 (mod 64),
        // then the message's bit count — always 1 or 2 whole blocks.
        let mut tail = data[whole..].to_vec();
        tail.push(0x80);
        while tail.len() % 64 != 56 {
            tail.push(0);
        }
        tail.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
        h.blocks(&tail);
        h.digest().iter().map(|w| format!("{w:08x}")).collect()
    }

    /// The streaming compression state (FIPS 180-1).
    struct State {
        h: [u32; 5],
    }

    impl State {
        const fn new() -> Self {
            Self {
                h: [
                    0x6745_2301,
                    0xEFCD_AB89,
                    0x98BA_DCFE,
                    0x1032_5476,
                    0xC3D2_E1F0,
                ],
            }
        }

        fn blocks(&mut self, data: &[u8]) {
            debug_assert!(data.len().is_multiple_of(64));
            for block in data.chunks_exact(64) {
                let mut w = [0u32; 80];
                for (i, word) in block.chunks_exact(4).enumerate() {
                    w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
                }
                for i in 16..80 {
                    w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
                }
                let [mut a, mut b, mut c, mut d, mut e] = self.h;
                for (i, &wi) in w.iter().enumerate() {
                    let (f, k) = match i / 20 {
                        0 => ((b & c) | (!b & d), 0x5A82_7999),
                        1 => (b ^ c ^ d, 0x6ED9_EBA1),
                        2 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                        _ => (b ^ c ^ d, 0xCA62_C1D6),
                    };
                    let tmp = a
                        .rotate_left(5)
                        .wrapping_add(f)
                        .wrapping_add(e)
                        .wrapping_add(k)
                        .wrapping_add(wi);
                    e = d;
                    d = c;
                    c = b.rotate_left(30);
                    b = a;
                    a = tmp;
                }
                for (h, v) in self.h.iter_mut().zip([a, b, c, d, e]) {
                    *h = h.wrapping_add(v);
                }
            }
        }

        fn digest(&self) -> [u32; 5] {
            self.h
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_known_vectors() {
        // FIPS 180-1 A.1/A.2 plus the block/padding edges.
        assert_eq!(sha1::hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            sha1::hex(b"abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        // 56 × 'a': the tail plus its bit count spills into a second block.
        assert_eq!(
            sha1::hex(&[b'a'; 56]),
            "c2db330f6083854c99d4b5bfb6e8f29f201be699"
        );
        // 64 × 'a': one whole block, empty tail after it.
        assert_eq!(
            sha1::hex(&[b'a'; 64]),
            "0098ba824b5c16427bd7a1122a5a442a25ec644d"
        );
        assert_eq!(
            sha1::hex(&[b'a'; 128]),
            "ad5b3fdbcb526778c2839d2f151ea753995e26a0"
        );
    }

    #[test]
    fn pmcp_application_reorders_slots() {
        // Two stored sub-palettes (16 colors each, values 0–31); the
        // table maps VRAM slot 0 → stored 0, slot 1 → stored 0, slot 2 →
        // stored 1 — the shape of the cache fixture's [0, 0, 1] table.
        let colors: Vec<[u8; 4]> = (0..32).map(|i| [i as u8, 0, 0, 255]).collect();
        let placed = apply_pmcp(&colors, &[0, 0, 1]).expect("applies");
        assert_eq!(placed.len(), 48, "the table patches 3 slots");
        assert_eq!(&placed[..16], &colors[0..16], "slot 0 ← stored 0");
        assert_eq!(&placed[16..32], &colors[0..16], "slot 1 ← stored 0");
        assert_eq!(&placed[32..48], &colors[16..32], "slot 2 ← stored 1");
        // A table shorter than the storage leaves the unpatched slots
        // cleared — the engine's VRAM state at boot.
        let placed = apply_pmcp(&colors, &[1]).expect("applies");
        assert_eq!(placed.len(), 32);
        assert_eq!(&placed[..16], &colors[16..32], "slot 0 ← stored 1");
        assert_eq!(placed[16..], vec![[0, 0, 0, 0]; 16]);

        // Out-of-range table entries are corruption, not clamping.
        assert!(apply_pmcp(&colors, &[5]).is_err());
        // More slots than the 16-color VRAM palette has.
        assert!(apply_pmcp(&colors, &[0u16; 17]).is_err());
        // Not even one whole slot.
        assert!(apply_pmcp(&colors[..8], &[0]).is_err());
    }

    #[test]
    fn decode_rejects_foreign_bytes() {
        // A member whose magic is not the table's format is a drift,
        // and the decoder must say so rather than guess.
        assert!(decode_tiles(b"RCSN....", "n", 0).is_err());
        assert!(decode_palette(b"RGCN....", "n", 0).is_err());
        assert!(decode_screen(b"RLCN....", "n", 0).is_err());
    }

    #[test]
    fn store_assigns_sequential_handles() {
        let mut store = AssetStore {
            rom: Vec::new(),
            assets: Vec::new(),
        };
        assert!(store.is_empty());
        // The push order is the handle order: FIRST, then index 1, 2…
        // (The kinds do not matter here — only the bookkeeping does.)
        let first = store.push(Asset::Screen(fake_screen()));
        let second = store.push(Asset::Tiles(fake_tiles()));
        assert_eq!(first, AssetId::FIRST);
        assert_eq!(second.index(), 1);
        assert_eq!(store.len(), 2);
        // Resolution is kind-checked: the screen handle finds the
        // screen, not the tiles.
        assert!(store.screen(first).is_some());
        assert!(store.screen(second).is_none());
        assert!(store.tiles(second).is_some());
        assert!(store.tiles(first).is_none());
        assert!(store.placed_palette(second).is_none());
        // An unassigned handle resolves to nothing.
        assert!(store.screen(AssetId::from_index(2)).is_none());
    }

    /// A minimal decoded Screen chunk (8 + 12 header bytes plus one
    /// text entry), built by hand — only the store's bookkeeping reads
    /// it here, so the payload stays tiny.
    fn fake_screen() -> cache::Screen {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&cache::MAGIC);
        chunk.extend_from_slice(&cache::VERSION.to_le_bytes());
        chunk.extend_from_slice(&cache::ChunkKind::Screen.code().to_le_bytes());
        chunk.extend_from_slice(&8u16.to_le_bytes()); // width
        chunk.extend_from_slice(&8u16.to_le_bytes()); // height
        chunk.extend_from_slice(&0u16.to_le_bytes()); // 4bpp
        chunk.extend_from_slice(&0u16.to_le_bytes()); // text format
        chunk.extend_from_slice(&2u32.to_le_bytes()); // one entry
        chunk.extend_from_slice(&[0x01, 0x00]);
        cache::Screen::parse(&chunk).expect("minimal chunk parses")
    }

    /// A minimal decoded Tiles chunk: one 8×8 4bpp tile of color 1.
    fn fake_tiles() -> cache::Tiles {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&cache::MAGIC);
        chunk.extend_from_slice(&cache::VERSION.to_le_bytes());
        chunk.extend_from_slice(&cache::ChunkKind::Tiles.code().to_le_bytes());
        chunk.extend_from_slice(&[0u8]); // 4bpp
        chunk.extend_from_slice(&[0u8]); // 2D mapping
        chunk.extend_from_slice(&[0u8]); // flags
        chunk.extend_from_slice(&[0u8]); // pad
        chunk.extend_from_slice(&1u32.to_le_bytes()); // tile count
        chunk.extend_from_slice(&64u32.to_le_bytes()); // pixel bytes
        chunk.extend_from_slice(&[1u8; 64]);
        cache::Tiles::parse(&chunk).expect("minimal chunk parses")
    }
}
