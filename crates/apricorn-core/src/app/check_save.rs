//! The save-check app — pret `src/application/check_savedata.c`, the
//! overlay between the title screen and the main menu, Phase 4,
//! step 5.
//!
//! `gApplication_CheckSave` runs one `BgTemplate` (MAIN BG0, 4bpp
//! 256×256, char base `0x18000`, priority 1) with a single 27×4
//! message window and walks `CheckSavedataApp_DoMainTask`'s state
//! chain over `Save_GetStatusFlags`: each set bit prints its
//! warning (`msg_0229`), the chain looping until every flag pair is
//! consumed, then the overlay registers the main menu
//! (`RegisterMainOverlay(OVY_74, gApp_MainMenu)`,
//! `check_savedata.c:188`).
//!
//! The per-frame order is the overlay triad's:
//!
//! * **Init** (the construction): `TextFlags_SetCanTouchSpeedUpPrint(TRUE)`
//!   — the touch LCD can fast-forward the warning print.
//! * **Main** (`CheckSavedataApp_Main`, one pass per tick): the
//!   overlay states — SETUP (the black write, the BG, the window),
//!   MAIN_TASK (the `DoMainTask` chain), EXIT (free) — and `ret
//!   TRUE` from the EXIT pass finishes the app.
//! * **`DoMainTask`** (`check_savedata.c:242`): `CHECK_SAVE_FLAGS`
//!   reads the flags (0 → EXIT); `GET_ERROR_MESSAGE` picks by the
//!   priority chain and consumes flag pairs; none left → EXIT, else
//!   FADE_IN; `FADE_IN` sets both backdrops to `RGB(1,1,27)` and
//!   begins the 6×1 brightness-in; `WAIT_FOR_FADE_IN` polls;
//!   `PRINT` runs [`PrintMessage`](Self::print_message); on its
//!   TRUE the same tick begins the 6×1 brightness-out;
//!   `WAIT_FOR_FADE_OUT` polls, then restores the black backdrops
//!   and loops to `GET_ERROR_MESSAGE`; `EXIT` returns TRUE.
//! * **`PrintMessage`** — the nested sub-machine
//!   (`CheckSavedataApp_PrintMessage`, `:316`): PRINT_TEXT
//!   re-fills the window, draws the frame
//!   (`DrawFrameAndWindow2(0x1E2, 2)`), and constructs the
//!   printer (font 1, speed 4 — its first glyph prints on the
//!   construction tick, the sys task running after exec);
//!   WAIT_FOR_PRINTER polls `TextPrinterCheckActive` — the
//!   exec-side check sees the *previous* tick's print — then drops
//!   the printer; EXIT waits for a new A or touch and returns TRUE,
//!   resetting printState for the loop's next message.
//!
//! The fades step through the scene's [`BrightnessFade`] — the
//! update after exec, the poll during it — so the standard 6×1 fade
//! begun on tick B reports finished first at B+7, and the begin
//! tick's frame already shows step 1. The frame carries the
//! last-written brightness on both engines.
//!
//! Deferrals, each honest in the frame: `SetKeyRepeatTimers(4, 8)`
//! (no key-repeat model), the heap accounting, and audio. The
//! status flags' frontier pairs (bits 2–5,
//! `Save_CheckFrontierData`) are computed by
//! [`Game`](super::game::Game)'s save parse and land here through
//! the same field the C reads.

use std::sync::Mutex;

use crate::app::fade::{BrightnessFade, FadeColor, FadeType};
use crate::app::text::{TextFlags, TextPrinter, font_color};
use crate::app::{App, ChainNext};
use crate::assets::{AssetStore, AssetsError, font_narc, frame_narc, msg_narc};
use crate::frame::{
    AssetId, BgLayer, ColorMode, DisplaySelect, LogicalFrame, PaletteLoad, ScreenSize, TilePlacement,
    Window, WindowFrame,
};
use crate::font::Font;
use crate::input::{Input, Keys, key};
use crate::text::string::GameString;

/// `NARC_msg_msg_0229_bin` — the warning bank's member in
/// `NARC_msgdata_msg` (`msg_0229_00000` through `msg_0229_00005`).
const MSG_BANK: usize = 229;
/// The warning bank's six messages, one per `Save_GetStatusFlags`
/// bit pair.
const MSG_COUNT: usize = 6;

/// `sCheckSave_BgTemplate` (`check_savedata.c:73`): MAIN BG0, 4bpp
/// 256×256, char base `0x18000` — block 3 of the model's eight
/// (`0x18000 / 0x8000`).
const BG_CHAR_BLOCK: u8 = 3;
/// The template's priority.
const BG_PRIORITY: u8 = 1;

/// `sCheckSave_WindowTemplate` (`check_savedata.c:56`).
const WINDOW_BG: u8 = 0;
const WINDOW_LEFT: u8 = 2;
const WINDOW_TOP: u8 = 19;
const WINDOW_WIDTH: u8 = 27;
const WINDOW_HEIGHT: u8 = 4;
const WINDOW_PALETTE: u8 = 1;
const WINDOW_BASE_TILE: u16 = 0x16D;
/// `FillWindowPixelRect`'s fill — the dialog background, also the
/// font triple's bg index.
const WINDOW_FILL: u8 = 0xF;

/// `LoadUserFrameGfx2(…, 0x1E2, 2, 0, …)` — frame id 0's NCGR is
/// member `frame + 2` (`sub_0200E63C`, `asm/render_window.s:364`),
/// its NCLR member `frame + 0x1A` (`sub_0200E640`, `:370`) — 0x1A
/// into bank 2 (`:205`).
const GFX2_TILE: u16 = 0x1E2;
/// `LoadUserFrameGfx1(…, 0x1D9, 3, 0, …)` — FRAME0's tiles at `0x1D9`,
/// its NCLR (`frame_narc::PALETTE`) into bank 3 (`:206`).
const GFX1_TILE: u16 = 0x1D9;
/// The frame banks' palette-RAM offsets, in colors (bank n = n·16).
const GFX2_BANK: u16 = 2 * 16;
const GFX1_BANK: u16 = 3 * 16;
/// `LoadFontPal0(MAIN_BG, PAL_SLOT_1_OFFSET)` — the font NCLR
/// (`font_narc::PAL0`) into bank 1 (`:207`).
const FONT_BANK: u16 = 1 * 16;
/// One 16-color palette load — one bank.
const BANK_COLORS: u16 = 16;

/// The message-print speed — `PrintMessage(…, textSpeed 4)`.
const TEXT_SPEED: u32 = 4;
/// `RGB(1, 1, 27)` — the FADE_IN backdrops' mask color
/// (`check_savedata.c:285`), raw BGR555.
const FADE_IN_BACKDROP: u16 = (27 << 10) | (1 << 5) | 1;

/// `CheckSavedataApp_MainState` (`check_savedata.c:24`) — the
/// `DoMainTask` chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainState {
    /// `MAINSTATE_CHECK_SAVE_FLAGS`: read the flags; 0 → EXIT.
    CheckSaveFlags,
    /// `MAINSTATE_GET_ERROR_MESSAGE`: the priority pick, consuming
    /// flag pairs.
    GetErrorMessage,
    /// `MAINSTATE_FADE_IN`: backdrops + the brightness-in begin.
    FadeIn,
    /// `MAINSTATE_WAIT_FOR_FADE_IN`: poll for the fade's end.
    WaitForFadeIn,
    /// `MAINSTATE_PRINT_ERROR_MESSAGE_FADE_OUT`: run the print; on
    /// TRUE begin the brightness-out.
    Print,
    /// `MAINSTATE_WAIT_FOR_FADE_OUT`: poll, then mask black and
    /// loop to `GET_ERROR_MESSAGE`.
    WaitForFadeOut,
    /// `MAINSTATE_EXIT`: return TRUE.
    Exit,
}

/// `CheckSavedataApp_PrintState` (`check_savedata.c:34`) — the
/// nested print machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrintState {
    /// `PRINTSTATE_PRINT_TEXT`: fill, frame, construct the printer.
    PrintText,
    /// `PRINTSTATE_WAIT_FOR_PRINTER`: poll `TextPrinterCheckActive`.
    WaitForPrinter,
    /// `PRINTSTATE_EXIT`: wait for A/touch, return TRUE.
    Exit,
}

/// The overlay states of `CheckSavedataApp_Main`'s switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayState {
    /// `STATE_SETUP` — the black write, the BG, the window.
    Setup,
    /// `STATE_MAIN_TASK` — the `DoMainTask` chain.
    MainTask,
    /// `STATE_EXIT` — free the graphics, `ret TRUE`.
    Exit,
}

/// The save-check app.
pub struct CheckSave {
    /// The logical frame the last tick produced.
    frame: LogicalFrame,
    /// The fade pair — `BeginNormalPaletteFade`'s both-screen works.
    fade: BrightnessFade,
    /// The scene's printer policy — the touch speedup Init enables
    /// and Exit disables.
    flags: TextFlags,
    /// Font 1, cloned at load so ticks need no store lock
    /// (`AddTextPrinterParameterized(…, 1, …)`).
    font: Font,
    /// The font-1 asset the pushed glyphs reference.
    font_asset: AssetId,
    /// The focus-indicator asset every printer carries.
    focus_asset: AssetId,
    /// Frame id 0's tiles through `LoadUserFrameGfx2` — NCGR
    /// member 2 (`sub_0200E63C`).
    gfx2_tiles: AssetId,
    /// FRAME0's tiles (`LoadUserFrameGfx1`'s NCGR).
    gfx1_tiles: AssetId,
    /// Frame id 0's palette through `LoadUserFrameGfx2` — NCLR
    /// member 0x1A (`sub_0200E640`).
    gfx2_pal: AssetId,
    /// FRAME0's palette (`frame_narc::PALETTE`).
    gfx1_pal: AssetId,
    /// The font's palette (`font_narc::PAL0`).
    font_pal: AssetId,
    /// The warning bank's messages, cloned at load (the C reads
    /// them lazily; the content is identical).
    messages: Vec<GameString>,
    /// The working copy of `Save_GetStatusFlags` —
    /// `GET_ERROR_MESSAGE` consumes it pair by pair.
    save_status_flags: u32,
    /// The overlay state (`CheckSavedataApp_Main`'s switch).
    overlay_state: OverlayState,
    /// The `DoMainTask` chain's state.
    main_state: MainState,
    /// The `PrintMessage` sub-machine's state.
    print_state: PrintState,
    /// The picked warning (`data->msgNum`).
    msg_num: usize,
    /// The running printer (`data->textPrinterId`'s object).
    printer: Option<TextPrinter>,
    /// The keys held on the previous tick, for press edges.
    prev_keys: Keys,
    /// Whether the stylus was down on the previous tick.
    prev_touch: bool,
    /// Whether the EXIT pass has run (`ret TRUE`).
    done: bool,
}

impl CheckSave {
    /// Constructs the app — `CheckSavedataApp_Init` plus the asset
    /// loads the model resolves through the store.
    ///
    /// `save_status_flags` is `Save_GetStatusFlags`'s value — the
    /// [`Game`](super::game::Game) machine's parse outcome, bit by
    /// bit as `save.c` sets them.
    ///
    /// The construction frame is the register state inherited from
    /// the title: the screens flipped (`DisplaySelect::SubOnTop` —
    /// the C never writes `GX_SetDispSelect`), everything else
    /// cleared. The first tick is the SETUP pass.
    ///
    /// # Errors
    /// Returns the store's [`AssetsError`] when a member is missing
    /// or corrupt — unreachable in practice against the pinned dump.
    pub fn load(store: &Mutex<AssetStore>, save_status_flags: u32) -> Result<Self, AssetsError> {
        let mut store = store
            .lock()
            .expect("the asset store is only locked at app construction");
        // The graphics the SETUP pass draws from: both frames and
        // their palettes, the font, the message bank.
        let gfx2_tiles = store.load_tiles(frame_narc::NARC, frame_narc::GFX2_FRAME0_CHAR)?;
        let gfx1_tiles = store.load_tiles(frame_narc::NARC, frame_narc::FRAME0_CHAR)?;
        let gfx2_pal = store.load_palette(frame_narc::NARC, frame_narc::GFX2_FRAME0_PALETTE)?;
        let gfx1_pal = store.load_palette(frame_narc::NARC, frame_narc::PALETTE)?;
        let font_asset = store.load_font(font_narc::NARC, font_narc::FONT1)?;
        let font_pal = store.load_palette(font_narc::NARC, font_narc::PAL0)?;
        let focus_asset = store.load_tiles(font_narc::NARC, font_narc::FOCUS_INDICATOR)?;
        let bank = store.load_msg_bank(msg_narc::NARC, MSG_BANK)?;
        let text = store.msg_bank(bank).expect("the just-loaded bank");
        let mut messages = Vec::with_capacity(MSG_COUNT);
        for m in 0..MSG_COUNT {
            let units = text.message(m).expect("the warning bank's six messages");
            messages.push(GameString::from_units(units));
        }
        let font = store
            .font(font_asset)
            .expect("the just-loaded font")
            .clone();

        Ok(Self {
            frame: LogicalFrame {
                display: DisplaySelect::SubOnTop,
                ..LogicalFrame::default()
            },
            fade: BrightnessFade::default(),
            // TextFlags_SetCanTouchSpeedUpPrint(TRUE) — Init.
            flags: make_flags(),
            font,
            font_asset,
            focus_asset,
            gfx2_tiles,
            gfx1_tiles,
            gfx2_pal,
            gfx1_pal,
            font_pal,
            messages,
            save_status_flags,
            overlay_state: OverlayState::Setup,
            main_state: MainState::CheckSaveFlags,
            print_state: PrintState::PrintText,
            msg_num: 0,
            printer: None,
            prev_keys: Keys::IDLE,
            prev_touch: false,
            done: false,
        })
    }

    /// The message window's index in the engine's window list —
    /// `data->window`, the one `AddWindow` added at SETUP.
    const WINDOW: usize = 0;

    /// `FillWindowPixelRect(&window, 0xF, 0, 0, 216, 32)` — the
    /// full-window fill erases the pixel buffer, so the glyphs and
    /// the focus state go with it.
    fn fill_window(window: &mut Window) {
        window.fill = WINDOW_FILL;
        window.glyphs.clear();
        window.focus = None;
    }

    /// The SETUP pass — `STATE_SETUP` +
    /// `CheckSavedataApp_SetupBgConfig`/`SetupTextAndWindow`: the
    /// black write, the BG (its template toggling the plane on),
    /// both frames' tiles and palettes, the font palette, and the
    /// message window.
    fn setup(&mut self) {
        // sub_0200FBF4 both LCDs black.
        self.fade.write(-16);
        // The template's layer: 4bpp 256x256 at char block 3,
        // priority 1, the cleared tilemap (BgClearTilemapBufferAndCommit).
        self.frame.main.bgs[usize::from(WINDOW_BG)] = BgLayer {
            enabled: true,
            char_base: BG_CHAR_BLOCK,
            screen: None,
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH256,
            scroll_x: 0,
            scroll_y: 0,
            priority: BG_PRIORITY,
        };
        // LoadUserFrameGfx2/1's char data, into the layer's block at
        // the template tiles.
        self.frame.main.char_blocks[usize::from(BG_CHAR_BLOCK)]
            .push(TilePlacement {
                asset: self.gfx2_tiles,
                tile: GFX2_TILE,
            });
        self.frame.main.char_blocks[usize::from(BG_CHAR_BLOCK)]
            .push(TilePlacement {
                asset: self.gfx1_tiles,
                tile: GFX1_TILE,
            });
        // The three palette loads, pret's order: Gfx2's NCLR (member
        // 0x1A) into bank 2, Gfx1's into bank 3, the font's into
        // bank 1.
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.gfx2_pal,
            offset: GFX2_BANK,
            colors: BANK_COLORS,
        });
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.gfx1_pal,
            offset: GFX1_BANK,
            colors: BANK_COLORS,
        });
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.font_pal,
            offset: FONT_BANK,
            colors: BANK_COLORS,
        });
        // BG_SetMaskColor black, both engines — the default backdrop.
        self.frame.main.backdrop = 0;
        self.frame.sub.backdrop = 0;
        // AddWindow + the fill.
        self.frame.main.windows.push(Window {
            bg: WINDOW_BG,
            left: WINDOW_LEFT,
            top: WINDOW_TOP,
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
            palette: WINDOW_PALETTE,
            base_tile: WINDOW_BASE_TILE,
            fill: WINDOW_FILL,
            glyphs: Vec::new(),
            scroll: 0,
            frame: None,
            arrow: None,
            focus: None,
        });
    }

    /// The EXIT pass — `STATE_EXIT`: `FreeTextAndWindow`
    /// (`RemoveWindow`) and `FreeBgConfig` (all eight layers
    /// toggled off), then `ret TRUE`.
    fn free_graphics(&mut self) {
        self.frame.main.windows.clear();
        for bg in &mut self.frame.main.bgs {
            bg.enabled = false;
        }
        for bg in &mut self.frame.sub.bgs {
            bg.enabled = false;
        }
        // TextFlags_SetCanTouchSpeedUpPrint(FALSE) — the app exit's
        // global restore, on the scene's own policy object.
        self.flags.set_can_touch_speed_up_print(false);
        self.done = true;
    }

    /// `CheckSavedataApp_DoMainTask` — the chain, one pass per tick;
    /// TRUE finishes the MAIN_TASK pass (the next tick is EXIT).
    fn do_main_task(&mut self, input: Input, new_keys: Keys, touch_new: bool) -> bool {
        match self.main_state {
            MainState::CheckSaveFlags => {
                // The flags came in with the construction; 0 → EXIT.
                if self.save_status_flags == 0 {
                    self.main_state = MainState::Exit;
                } else {
                    self.main_state = MainState::GetErrorMessage;
                }
                false
            }
            MainState::GetErrorMessage => {
                match pick_warning(&mut self.save_status_flags) {
                    Some(msg) => {
                        self.msg_num = msg;
                        self.main_state = MainState::FadeIn;
                    }
                    // The chain consumed every pair: EXIT.
                    None => self.main_state = MainState::Exit,
                }
                false
            }
            MainState::FadeIn => {
                // BG_SetMaskColor(RGB(1,1,27)) both, then the 6x1
                // brightness-in begins — the begin tick's frame
                // shows step 1.
                self.frame.main.backdrop = FADE_IN_BACKDROP;
                self.frame.sub.backdrop = FADE_IN_BACKDROP;
                self.fade.begin(FadeType::BrightnessIn, FadeColor::Black, 6, 1);
                self.main_state = MainState::WaitForFadeIn;
                false
            }
            MainState::WaitForFadeIn => {
                if self.fade.is_finished() {
                    self.main_state = MainState::Print;
                }
                false
            }
            MainState::Print => {
                if self.print_message(input, new_keys, touch_new) {
                    // The same tick the print finished: the 6x1
                    // brightness-out.
                    self.fade.begin(FadeType::BrightnessOut, FadeColor::Black, 6, 1);
                    self.main_state = MainState::WaitForFadeOut;
                }
                false
            }
            MainState::WaitForFadeOut => {
                if self.fade.is_finished() {
                    // The black masks restored, then the loop.
                    self.frame.main.backdrop = 0;
                    self.frame.sub.backdrop = 0;
                    self.main_state = MainState::GetErrorMessage;
                }
                false
            }
            MainState::Exit => true,
        }
    }

    /// `CheckSavedataApp_PrintMessage` — the nested sub-machine, one
    /// pass per tick; TRUE is the A-press that releases the message
    /// (and re-arms PRINT_TEXT for the loop's next one).
    fn print_message(&mut self, input: Input, new_keys: Keys, touch_new: bool) -> bool {
        match self.print_state {
            PrintState::PrintText => {
                {
                    let window = &mut self.frame.main.windows[Self::WINDOW];
                    Self::fill_window(window);
                    window.frame = Some(WindowFrame {
                        base_tile: GFX2_TILE,
                        palette: 2,
                    });
                }
                let units = self.messages[self.msg_num].clone();
                self.printer = Some(TextPrinter::new(
                    1,
                    self.font_asset,
                    self.focus_asset,
                    units,
                    0,
                    0,
                    font_color(1),
                    TEXT_SPEED,
                    0,
                ));
                // The construction-frame print: the sys task runs
                // after this exec, so the first glyph lands now.
                self.render_printer(input);
                self.print_state = PrintState::WaitForPrinter;
                false
            }
            PrintState::WaitForPrinter => {
                // TextPrinterCheckActive reads the registry the
                // previous tick's print step updated: exec sees the
                // print through tick N-1. Finished → String_Delete
                // and the EXIT wait; otherwise this tick's print
                // step runs.
                if self.printer.as_ref().is_some_and(TextPrinter::is_finished) {
                    self.printer = None;
                    self.print_state = PrintState::Exit;
                } else {
                    self.render_printer(input);
                }
                false
            }
            PrintState::Exit => {
                // The release: a new A or touch. The printState
                // re-arms for the loop's next message.
                if new_keys.any(key::A) || touch_new {
                    self.print_state = PrintState::PrintText;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// This tick's print step — the sys task's `RunTextPrinter`,
    /// which the main loop runs after the scene's exec. The
    /// printer samples its own input edges; the scene's flags are
    /// the shared policy.
    fn render_printer(&mut self, input: Input) {
        let Some(printer) = self.printer.as_mut() else {
            return;
        };
        printer.render(
            &self.font,
            &mut self.frame.main.windows[Self::WINDOW],
            input,
            &mut self.flags,
        );
    }
}

/// `TextFlags_SetCanTouchSpeedUpPrint(TRUE)` — Init's policy, the
/// power-on defaults otherwise.
fn make_flags() -> TextFlags {
    let mut flags = TextFlags::default();
    flags.set_can_touch_speed_up_print(true);
    flags
}

/// `MAINSTATE_GET_ERROR_MESSAGE`'s priority chain
/// (`check_savedata.c:254-282`): the highest-priority set bit picks
/// its message and consumes its flag pair, so the loop prints each
/// warning once. The frontier pairs' conditions are not computed
/// yet — the bits simply never set.
fn pick_warning(flags: &mut u32) -> Option<usize> {
    if *flags & (1 << 1) != 0 {
        *flags &= !((1 << 1) | (1 << 0));
        Some(1) // msg_0229_00001
    } else if *flags & (1 << 0) != 0 {
        *flags ^= 1 << 0;
        Some(0) // msg_0229_00000
    } else if *flags & (1 << 3) != 0 {
        *flags &= !((1 << 3) | (1 << 2));
        Some(5) // msg_0229_00005
    } else if *flags & (1 << 2) != 0 {
        *flags ^= 1 << 2;
        Some(4) // msg_0229_00004
    } else if *flags & (1 << 5) != 0 {
        *flags &= !((1 << 5) | (1 << 4));
        Some(3) // msg_0229_00003
    } else if *flags & (1 << 4) != 0 {
        *flags ^= 1 << 4;
        Some(2) // msg_0229_00002
    } else {
        None
    }
}

impl App for CheckSave {
    fn tick(&mut self, _frame: crate::Frame, input: Input) {
        let new_keys = input.keys.pressed(self.prev_keys);
        self.prev_keys = input.keys;
        let touch_new = input.touch.is_some() && !self.prev_touch;
        self.prev_touch = input.touch.is_some();

        // CheckSavedataApp_Main's switch — one pass per tick.
        match self.overlay_state {
            OverlayState::Setup => {
                self.setup();
                self.overlay_state = OverlayState::MainTask;
            }
            OverlayState::MainTask => {
                if self.do_main_task(input, new_keys, touch_new) {
                    self.overlay_state = OverlayState::Exit;
                }
            }
            OverlayState::Exit => self.free_graphics(),
        }

        // The post-vblank fade step, then the frame carries the
        // last-written brightness on both engines.
        self.fade.update();
        let brightness = self.fade.brightness();
        self.frame.main.brightness = brightness;
        self.frame.sub.brightness = brightness;
    }

    fn frame(&self) -> &LogicalFrame {
        &self.frame
    }

    fn next(&self) -> ChainNext {
        if self.done {
            ChainNext::Advance
        } else {
            ChainNext::Stay
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_priority_chain_prints_each_warning_once() {
        // The full flag set consumes in pret's priority order —
        // bit 1 first (clearing 1|0), then the frontier pairs
        // high-first, each loop printing one message.
        let mut flags = (1 << 5) | (1 << 4) | (1 << 3) | (1 << 2) | (1 << 1) | (1 << 0);
        let picks = [
            pick_warning(&mut flags),
            pick_warning(&mut flags),
            pick_warning(&mut flags),
            pick_warning(&mut flags),
            pick_warning(&mut flags),
        ];
        // Bit 1's pick consumes bits 1|0 (msg 1); bit 3's consumes
        // 3|2 (msg 5); bit 5's consumes 5|4 (msg 3); then nothing.
        assert_eq!(picks, [Some(1), Some(5), Some(3), None, None]);
        assert_eq!(flags, 0, "every pair consumed");
    }

    #[test]
    fn the_degraded_slot_prints_bit_zero_after_the_erased_one() {
        // Bits 1|0 set together: the TOTAL_FAIL warning wins (its
        // pick clears both) — the degraded slot never prints alone.
        let mut flags = (1 << 1) | (1 << 0);
        assert_eq!(pick_warning(&mut flags), Some(1));
        assert_eq!(pick_warning(&mut flags), None);
        // Bit 0 alone: the degraded-slot warning.
        let mut flags = 1 << 0;
        assert_eq!(pick_warning(&mut flags), Some(0));
        assert_eq!(flags, 0);
    }

    #[test]
    fn the_mask_colors_restate_the_raw_values() {
        // RGB(1,1,27): the FADE_IN backdrop; RGB_BLACK is 0.
        assert_eq!(FADE_IN_BACKDROP, 0x6C21);
    }
}