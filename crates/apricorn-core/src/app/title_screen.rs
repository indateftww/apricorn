//! The title screen — pret `src/title_screen.c`'s statics and its
//! 2D timeline, through the play state's timeout.
//!
//! Port of the observable 2D state, Phase 3 scope:
//!
//! * **Statics** (`TitleScreen_InitBgs` +
//!   `TitleScreenAnim_Load2dBgGfx`, HeartGold arm): BG mode 0 both
//!   engines, MAIN BG0 bound to the 3D core (deferred — renders
//!   absent) at priority 1 (`G2_SetBG0Priority(1)`), SUB BG1 the
//!   4bpp static art (priority 0), SUB BG2 the 8bpp game logo
//!   (priority 0), SUB BG3 the 8bpp version art (priority 3). MAIN
//!   BG1/BG2 configure but load nothing (cleared, transparent); MAIN
//!   BG3 is the "TOUCH TO START" window — its content is Phase 4
//!   text, so it renders transparent while its plane toggles
//!   faithfully with the flash. `G2S_SetBlendAlpha(4, 0x39, 0, 0x1F)`
//!   puts the logo in the SUB blend unit's first target with BG0,
//!   BG3, OBJ, and the backdrop as second targets.
//! * **The logo fade-in** (`TitleScreenAnim_FadeInGameTitleLayer`):
//!   after the 30-frame initial delay plus a 3-frame title delay,
//!   one step per tick — SUB BG2's y offset `t/2`, blend `EVA = t`,
//!   `EBV = 31 - t`, `t` capped at 31 (the offset rests at 15, as in
//!   the game).
//! * **The flash** (`TitleScreenAnim_Run`): the window's plane goes
//!   on at flash-timer 0 and off at 30, the timer wrapping at 45.
//! * **The exits** (`TitleScreen_Main`): A, START, or a new stylus
//!   contact once the flash is enabled exits to the save-check menu
//!   (`TITLESCREEN_EXIT_MENU`); the play timer passing 2340 exits
//!   back to the intro movie (`TITLESCREEN_EXIT_TIMEOUT`). The app
//!   reports which through [`TitleScreen::exit`] — pret's
//!   `data->exitMode` — so the game-state machine can route the
//!   restart chain (menu) versus the boot chain's loop (timeout).
//!
//! The frame at construction is the cleared state after
//! `TitleScreen_Init`: every layer configured, all planes off, black
//! backdrops, `screensFlipped` (`DisplaySelect::SubOnTop`). The first
//! tick is `TitleScreenAnim_Run`'s SETUP — the planes come on.
//!
//! Deferrals, each honest in the frame: the Ho-Oh NSBMD model, its
//! sparkles, and the camera pan (Phase 6 3D); the "TOUCH TO START"
//! text and its palette colors (Phase 4 text); the top-screen glow
//! (a palette-content fade — the model has no mutable palette RAM);
//! the exit palette fades and the BGM (audio); the SUB logo's
//! extended-palette load, which the off ext-palette mode makes
//! observationally the regular load.

use std::sync::Mutex;

use crate::app::App;
use crate::assets::{AssetStore, AssetsError, title_screen};
use crate::frame::{
    BgLayer, Blend, BlendEffect, ColorMode, DisplaySelect, LogicalFrame, PaletteLoad, ScreenSize,
    TilePlacement, plane,
};
use crate::input::{Input, Keys, key};

/// The initial-delay length, pret's `data->initialDelay = 30`.
const INITIAL_DELAY: u8 = 30;
/// The flash's frame counts: on at 0, off at 30, wrapping at 45
/// (`startInstructionFlashTimer`).
const FLASH_OFF_AT: u8 = 30;
const FLASH_PERIOD: u8 = 45;
/// The logo fade's start delay (`gameTitleDelayTimer > 3`).
const TITLE_DELAY: u16 = 3;
/// The logo fade's cap (`gameTitleFadeInTimer` pinned at 31).
const TITLE_FADE_CAP: u8 = 31;
/// The play timer's timeout, pret's `TITLE_SCREEN_DURATION 2340`.
const PLAY_DURATION: u32 = 2340;

/// How the title screen finished — pret's
/// `TitleScreenOverlayData::exitMode` values the state machine
/// routes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleExit {
    /// `TITLESCREEN_EXIT_MENU`: a press chose the save-check menu.
    Menu,
    /// `TITLESCREEN_EXIT_TIMEOUT`: the play timer expired; back to
    /// the intro movie.
    Timeout,
}

/// The title screen app.
pub struct TitleScreen {
    /// The logical frame the last tick produced.
    frame: LogicalFrame,
    /// Whether the SETUP tick has run.
    setup_done: bool,
    /// The remaining initial-delay frames.
    initial_delay: u8,
    /// The flash timer (pret's `startInstructionFlashTimer`).
    flash_timer: u8,
    /// The play timer (pret's `data->timer`), timeout at
    /// `> 2340`.
    play_timer: u32,
    /// Whether the initial delay is over and the exits are live
    /// (`enableStartInstructionFlash`).
    flash_enabled: bool,
    /// The logo fade's start-delay counter (`gameTitleDelayTimer`).
    title_delay: u16,
    /// The logo fade's step counter, capped at 31.
    fade_timer: u8,
    /// The keys held on the previous tick, for press edges.
    prev_keys: Keys,
    /// Whether the stylus was down on the previous tick.
    prev_touch: bool,
    /// The screen's exit, once it has finished (pret's `exitMode`).
    exit: Option<TitleExit>,
}

impl TitleScreen {
    /// Loads the screen's members and builds the cleared initial
    /// frame.
    ///
    /// The load order is pret's `TitleScreenAnim_Load2dBgGfx`
    /// (HeartGold arm); the layer geometry is `TitleScreen_InitBgs`'
    /// templates, with each template's `charBase` naming the
    /// char-block slot.
    ///
    /// # Errors
    /// Returns the store's [`AssetsError`] when a table member is
    /// missing or corrupt — unreachable in practice, since the store
    /// only opens the SHA-1-pinned dump the table is valid for.
    pub fn load(store: &Mutex<AssetStore>) -> Result<Self, AssetsError> {
        let mut store = store
            .lock()
            .expect("the asset store is only locked at app construction");
        let narc = title_screen::NARC;
        let sub_bg3_char = store.load_tiles(narc, title_screen::SUB_BG3_CHAR)?;
        let sub_bg3_screen = store.load_screen(narc, title_screen::SUB_BG3_SCREEN)?;
        let sub_palette = store.load_palette(narc, title_screen::SUB_PALETTE)?;
        let main_palette = store.load_palette(narc, title_screen::MAIN_PALETTE)?;
        let sub_bg2_char = store.load_tiles(narc, title_screen::SUB_BG2_CHAR)?;
        let sub_bg2_screen = store.load_screen(narc, title_screen::SUB_BG2_SCREEN)?;
        let sub_bg1_char = store.load_tiles(narc, title_screen::SUB_BG1_CHAR)?;
        let sub_bg1_screen = store.load_screen(narc, title_screen::SUB_BG1_SCREEN)?;

        // Engine A (MAIN, the touch LCD here): BG0 is the 3D model's
        // viewport — deferred, so no screen; BG1/BG2 configure but
        // never load; BG3 is the cleared window. Engine B (SUB, the
        // top LCD): the three art layers.
        let main_layer = |char_base: u8, priority: u8| BgLayer {
            enabled: false,
            char_base,
            screen: None,
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH256,
            scroll_x: 0,
            scroll_y: 0,
            priority,
        };
        let sub_layer = |char_base: u8, screen, color_mode: ColorMode, priority: u8| BgLayer {
            enabled: false,
            char_base,
            screen,
            color_mode,
            size: ScreenSize::W256xH256,
            scroll_x: 0,
            scroll_y: 0,
            priority,
        };

        let mut frame = LogicalFrame {
            display: DisplaySelect::SubOnTop,
            ..LogicalFrame::default()
        };
        // GXLoadPal's whole-file loads at slot offset 0: both title
        // NCLRs are 256-color files.
        frame.sub.palette_loads.push(PaletteLoad {
            asset: sub_palette,
            offset: 0,
            colors: 0x200 / 2,
        });
        frame.main.palette_loads.push(PaletteLoad {
            asset: main_palette,
            offset: 0,
            colors: 0x200 / 2,
        });
        // G2_SetBG0Priority(1) after the 3D VRAM manager's creation.
        frame.main.bgs[0] = main_layer(0, 1);
        frame.main.bgs[1] = main_layer(1, 1);
        frame.main.bgs[2] = main_layer(4, 3);
        frame.main.bgs[3] = main_layer(0, 0);
        frame.sub.char_blocks[3].push(TilePlacement {
            asset: sub_bg1_char,
            tile: 0,
        });
        frame.sub.char_blocks[0].push(TilePlacement {
            asset: sub_bg2_char,
            tile: 0,
        });
        frame.sub.char_blocks[4].push(TilePlacement {
            asset: sub_bg3_char,
            tile: 0,
        });
        frame.sub.bgs[1] = sub_layer(3, Some(sub_bg1_screen), ColorMode::Bpp4, 0);
        frame.sub.bgs[2] = sub_layer(0, Some(sub_bg2_screen), ColorMode::Bpp8, 0);
        frame.sub.bgs[3] = sub_layer(4, Some(sub_bg3_screen), ColorMode::Bpp8, 3);
        // G2S_SetBlendAlpha(4, 0x39, 0, 0x1F): the logo (BG2) over
        // BG0, BG3, OBJ, and the backdrop — EVA 0/EBV 31, the logo
        // fully replaced by its second target until the fade-in.
        frame.sub.blend = Blend {
            plane1: plane::BG2,
            effect: BlendEffect::Alpha,
            plane2: plane::BG0 | plane::BG3 | plane::OBJ | plane::BD,
            eva: 0,
            ebv: 31,
            evy: 0,
        };

        Ok(Self {
            frame,
            setup_done: false,
            initial_delay: INITIAL_DELAY,
            flash_timer: 0,
            play_timer: 0,
            flash_enabled: false,
            title_delay: 0,
            fade_timer: 0,
            prev_keys: Keys::IDLE,
            prev_touch: false,
            exit: None,
        })
    }

    /// The flash half of `TitleScreenAnim_Run`'s RUN case — runs on
    /// every play tick, before the exit checks.
    fn run_flash(&mut self) {
        if self.flash_enabled {
            if self.flash_timer == 0 {
                self.frame.main.bgs[3].enabled = true;
            } else if self.flash_timer == FLASH_OFF_AT {
                self.frame.main.bgs[3].enabled = false;
            }
        } else {
            self.frame.main.bgs[3].enabled = false;
        }
        self.flash_timer += 1;
        if self.flash_timer >= FLASH_PERIOD {
            self.flash_timer = 0;
        }
    }
}

impl App for TitleScreen {
    fn tick(&mut self, _frame: crate::Frame, input: Input) {
        let new_keys = input.keys.pressed(self.prev_keys);
        self.prev_keys = input.keys;
        let touch_new = input.touch.is_some() && !self.prev_touch;
        self.prev_touch = input.touch.is_some();

        if !self.setup_done {
            // TitleScreenAnim_Run's SETUP — which in pret runs inside
            // the first delay-decrementing play call: every plane on
            // (MAIN BG3 waits for the flash), brightness neutral, and
            // the initial delay counting its first frame.
            self.frame.main.bgs[0].enabled = true;
            self.frame.main.bgs[1].enabled = true;
            self.frame.main.bgs[2].enabled = true;
            self.frame.sub.bgs[1].enabled = true;
            self.frame.sub.bgs[2].enabled = true;
            self.frame.sub.bgs[3].enabled = true;
            self.initial_delay -= 1;
            self.flash_enabled = self.initial_delay == 0;
            self.setup_done = true;
            return;
        }

        if !self.flash_enabled {
            // The initial-delay branch: the flash stays off and the
            // delay counts down; the exits are not live yet.
            self.run_flash();
            self.initial_delay -= 1;
            self.flash_enabled = self.initial_delay == 0;
            return;
        }

        // The play branch: the flash, the play timer, then the exits
        // or the logo fade.
        self.run_flash();
        self.play_timer += 1;
        if new_keys.any(key::A | key::START) || touch_new {
            // TITLESCREEN_EXIT_MENU: the state machine routes this to
            // the save-check menu.
            self.exit = Some(TitleExit::Menu);
            return;
        }
        if self.play_timer > PLAY_DURATION {
            // TITLESCREEN_EXIT_TIMEOUT: the game turns MAIN BG3 off
            // on its way out, back to the intro movie.
            self.frame.main.bgs[3].enabled = false;
            self.exit = Some(TitleExit::Timeout);
            return;
        }
        // TitleScreenAnim_FadeInGameTitleLayer: a 3-frame delay, then
        // the logo's y offset (t/2) and blend step (EVA t), capped.
        self.title_delay += 1;
        if self.title_delay > TITLE_DELAY {
            self.frame.sub.bgs[2].scroll_y = u16::from(self.fade_timer) / 2;
            self.fade_timer = (self.fade_timer + 1).min(TITLE_FADE_CAP);
            self.frame.sub.blend.eva = self.fade_timer;
            self.frame.sub.blend.ebv = TITLE_FADE_CAP - self.fade_timer;
        }
    }

    fn frame(&self) -> &LogicalFrame {
        &self.frame
    }

    fn next(&self) -> crate::app::ChainNext {
        if self.exit.is_some() {
            crate::app::ChainNext::Advance
        } else {
            crate::app::ChainNext::Stay
        }
    }
}

impl TitleScreen {
    /// How the screen finished — pret's `data->exitMode` — once
    /// [`App::next`](crate::app::App::next) says `Advance`.
    #[must_use]
    pub fn exit(&self) -> Option<TitleExit> {
        self.exit
    }
}
