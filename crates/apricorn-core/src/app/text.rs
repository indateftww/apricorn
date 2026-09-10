//! The message printer — a port of pret's `RenderText` state
//! machine (`src/render_text.c`) over the frame model's
//! [`Window`](crate::frame::Window), Phase 4, step 5.
//!
//! The original prints into a window's VRAM pixel buffer one
//! `RenderText` call per frame (`RunTextPrinter`'s sys task), with
//! the glyph decode + blit (`FontID_TryLoadGlyph` +
//! `CopyGlyphToWindow`) writing packed 4bpp indices and the wait
//! states writing *tilemap* entries for the down arrow. This port
//! keeps the state machine verbatim and swaps only the output
//! side: a printed glyph becomes a [`WindowGlyph`] push (the raster
//! re-derives the blit), a draw of the down arrow becomes
//! [`WindowArrow`] state, and the `{YESNO 0}` focus blit becomes
//! [`WindowFocus`] state. Printing is therefore idempotent per
//! frame — the window carries the printing state, never pixels.
//!
//! The pieces of pret's printer that don't survive the swap:
//!
//! * **The half-row lookup** (`GenerateFontHalfRowLookupTable`):
//!   the C rebuilds a per-color table each frame; the model's
//!   [`TextColor`] triple *is* the table (level 1 → fg, 2 → shadow,
//!   3 → bg, applied at raster time).
//! * **The VRAM copies** (`CopyWindowToVram` on `RENDER_PRINT` and
//!   after each 4-pixel scroll step, `textSpeedTop`'s copy on the
//!   focus blit): the raster reads the window every frame, so there
//!   is nothing to copy. `textSpeedTop` is not modeled — its only
//!   read is that copy.
//! * **The focus graphics cache** (`printer->unk30`): the scene
//!   loads the NCGR (`NARC_graphic_font` member 6) at construction
//!   and hands the [`AssetId`] to the printer; the raster resolves
//!   it.
//! * **Callbacks** (`printer->callback`, the `unk2D` gate) and the
//!   `PlaySE` on continue: scenes drive their own state machines
//!   off [`TextPrinter::is_finished`] and the flags; audio is
//!   deferred.
//!
//! Two port decisions on pret's own UB, both documented here: the
//! down-arrow animation index *cycles* `% 4`
//! (`sDownArrowTileOffsets` has four entries but the C increments
//! `downArrowYPosIdx` unbounded), and the arrow's base tile is a
//! per-printer field (pret's `sDownArrowBaseTile` global, set by
//! `DrawFrameAndWindow2` to the dialogue frame's tile address).
//! The finished printer carries no
//! pret state number: `is_finished` marks the sentinel the string's
//! EOS sets.
//!
//! Input arrives per frame (`gSystem`'s held/new keys and touch
//! edges are one-frame facts); [`TextPrinter::render`] is one
//! `RunTextPrinter` call: it samples the input edges, then loops
//! `RenderText` while the machine says `Repeat`, as `RenderFont`
//! does. The synchronous speeds sample the edges *once* — pret's
//! instant path loops `RenderFont` inside one frame with a
//! constant `gSystem`.

use crate::frame::{AssetId, TextColor, Window, WindowArrow, WindowFocus, WindowGlyph};
use crate::font::{Font, font_info};
use crate::input::{Input, Keys, Touch, key};
use crate::text::ctrl::{EXT_CTRL_CODE_BEGIN, CHAR_LF, parse_ext_ctrl};
use crate::text::string::GameString;

/// `text.h`'s `TEXT_SPEED_INSTANT` (0): the synchronous print.
pub const TEXT_SPEED_INSTANT: u32 = 0;
/// `text.h`'s `TEXT_SPEED_NOTRANSFER` (0xFF): the synchronous print,
/// minus the VRAM copy — observationally identical in the model.
pub const TEXT_SPEED_NOTRANSFER: u32 = 0xFF;

/// The auto-scroll parameter bits — `render_text.h`'s
/// `AUTO_SCROLL_OFF`/`AUTO_SCROLL_ENABLE`/`AUTO_SCROLL_SPEEDUP`.
pub const AUTO_SCROLL_OFF: u8 = 0;
/// Bit 0 of the auto-scroll parameter: auto-scroll on.
pub const AUTO_SCROLL_ENABLE: u8 = 1 << 0;
/// Bit 1: A/B (and touch) can still speed the print up.
pub const AUTO_SCROLL_SPEEDUP: u8 = 1 << 1;

/// The auto-scroll wait's fixed length — pret's
/// `subStruct->autoScrollDelay == 100`.
const AUTO_SCROLL_WAIT: u8 = 100;

/// The down-arrow frame delay — pret's `subStruct->downArrowDelay
/// = 8`.
const DOWN_ARROW_DELAY: u8 = 8;

/// The arrow animation's four-entry table — pret's
/// `sDownArrowTileOffsets`, here so the `% 4` port of the unbounded
/// index names its bound (the frame model re-exports it as
/// [`ARROW_TILE_OFFSETS`](crate::frame::ARROW_TILE_OFFSETS)).
const DOWN_ARROW_FRAMES: u8 = 4;

/// The state number the finished printer rests at — beyond pret's
/// 0–8 (its own fall-through is dead code).
const STATE_FINISHED: u8 = u8::MAX;

/// `RenderResult` (`font_types_def.h`) — what one `RenderText` call
/// did, in pret's own order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderResult {
    /// A glyph printed (`RENDER_PRINT`).
    Print,
    /// The string is exhausted (`RENDER_FINISH`).
    Finish,
    /// A control code was consumed; run again (`RENDER_REPEAT`).
    Repeat,
    /// A wait or scroll step ran (`RENDER_UPDATE`).
    Update,
}

/// `AddTextPrinterParameterized`'s color — the font's own
/// `sFonts[fontId]` triple (`fgColor`/`shadowColor`/`bgColor`).
#[must_use]
pub fn font_color(font_id: u8) -> TextColor {
    let info = font_info(font_id);
    TextColor::new(info.fg_color, info.shadow_color, info.bg_color)
}

/// pret's file-scope `TextFlags` bitfield (`render_text.h`) plus its
/// touch hitbox — the print-speed and continue policy shared by
/// every printer of a scene.
///
/// Plain data the owning scene holds and passes `&mut`; the getters
/// and setters are pret's `TextFlags_*` API one-to-one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFlags {
    /// A/B (held after a speedup, or new) can skip the per-char
    /// delay (`canABSpeedUpPrint`).
    pub can_ab_speed_up_print: bool,
    /// Suppress the down arrow entirely (`useAlternateDownArrow`).
    pub use_alternate_down_arrow: bool,
    /// The wait states advance on a timer instead of input
    /// (`autoScroll`).
    pub auto_scroll: bool,
    /// `forceMidTextSpeed` — carried for layout parity; the boot
    /// flow never reads it.
    pub force_mid_text_speed: bool,
    /// Touch contact can speed the print up
    /// (`canTouchSpeedUpPrint`).
    pub can_touch_speed_up_print: bool,
    /// A/B also continue an auto-scrolling wait
    /// (`autoScrollCanSpeedUp`).
    pub auto_scroll_can_speed_up: bool,
    /// A/B sped a print up this scene (`hasSpedUpInput`).
    pub has_sped_up_input: bool,
    /// A/B continued a wait this scene (`hasContinuedInput`).
    pub has_continued_input: bool,
    /// The current speeding-up came from touch
    /// (`touchIsSpeedingUpPrint`).
    pub touch_is_speeding_up_print: bool,
    /// The touch speedup is confined to `touch_hitbox`
    /// (`touchHitboxActive`).
    pub touch_hitbox_active: bool,
    /// The fast-forward hitbox's rect — (left, top, right, bottom),
    /// touchscreen coordinates. `Unset…` restores pret's
    /// full-screen default.
    pub touch_hitbox: (u8, u8, u8, u8),
}

impl Default for TextFlags {
    fn default() -> Self {
        // Power-on zeroes everywhere; the hitbox is pret's Unset
        // default (the full screen), though inactive until Set.
        Self {
            can_ab_speed_up_print: false,
            use_alternate_down_arrow: false,
            auto_scroll: false,
            force_mid_text_speed: false,
            can_touch_speed_up_print: false,
            auto_scroll_can_speed_up: false,
            has_sped_up_input: false,
            has_continued_input: false,
            touch_is_speeding_up_print: false,
            touch_hitbox_active: false,
            touch_hitbox: (0, 0, 255, 192),
        }
    }
}

impl TextFlags {
    /// `TextFlags_SetCanABSpeedUpPrint`.
    pub fn set_can_ab_speed_up_print(&mut self, enable: bool) {
        self.can_ab_speed_up_print = enable;
    }

    /// `TextFlags_SetAutoScrollParam`: bit 0 enables auto-scroll,
    /// bit 1 keeps the A/B speedup live.
    pub fn set_auto_scroll_param(&mut self, param: u8) {
        self.auto_scroll = param & AUTO_SCROLL_ENABLE != 0;
        self.auto_scroll_can_speed_up = param & AUTO_SCROLL_SPEEDUP != 0;
    }

    /// `TextFlags_SetCanTouchSpeedUpPrint`.
    pub fn set_can_touch_speed_up_print(&mut self, enable: bool) {
        self.can_touch_speed_up_print = enable;
    }

    /// `TextFlags_SetAlternateDownArrow`.
    pub fn set_alternate_down_arrow(&mut self, enable: bool) {
        self.use_alternate_down_arrow = enable;
    }

    /// `TextFlags_GetHasSpedUpInput`.
    #[must_use]
    pub fn has_sped_up_input(&self) -> bool {
        self.has_sped_up_input
    }

    /// `TextFlags_ResetHasSpedUpInput`.
    pub fn reset_has_sped_up_input(&mut self) {
        self.has_sped_up_input = false;
    }

    /// `TextFlags_GetHasContinuedInput`.
    #[must_use]
    pub fn has_continued_input(&self) -> bool {
        self.has_continued_input
    }

    /// `TextFlags_ResetHasContinuedInput`.
    pub fn reset_has_continued_input(&mut self) {
        self.has_continued_input = false;
    }

    /// `TextFlags_GetIsTouchSpeedUpPrint`.
    #[must_use]
    pub fn is_touch_speeding_up_print(&self) -> bool {
        self.can_touch_speed_up_print && self.touch_is_speeding_up_print
    }

    /// `TextFlags_SetFastForwardTouchButtonHitbox`.
    pub fn set_fast_forward_touch_button_hitbox(&mut self, rect: (u8, u8, u8, u8)) {
        self.touch_hitbox_active = true;
        self.touch_hitbox = rect;
    }

    /// `TextFlags_UnsetFastForwardTouchButtonHitbox`.
    pub fn unset_fast_forward_touch_button_hitbox(&mut self) {
        self.touch_hitbox_active = false;
        self.touch_hitbox = (0, 0, 255, 192);
    }

    /// `TextFlags_BeginAutoScroll`.
    pub fn begin_auto_scroll(&mut self, no_speed_up: bool) {
        if !no_speed_up {
            self.set_can_ab_speed_up_print(true);
            self.set_auto_scroll_param(AUTO_SCROLL_ENABLE | AUTO_SCROLL_SPEEDUP);
            self.set_can_touch_speed_up_print(true);
        } else {
            self.set_auto_scroll_param(AUTO_SCROLL_ENABLE);
            self.set_can_ab_speed_up_print(false);
            self.set_can_touch_speed_up_print(false);
        }
    }

    /// `TextFlags_EndAutoScroll`.
    pub fn end_auto_scroll(&mut self) {
        self.set_can_ab_speed_up_print(false);
        self.set_auto_scroll_param(AUTO_SCROLL_OFF);
        self.set_can_touch_speed_up_print(false);
    }

    /// `TouchscreenHitbox_PointIsInRect`'s unsigned window test —
    /// wrapping subtraction, exactly pret's own arithmetic.
    fn touch_is_in(&self, touch: Touch) -> bool {
        if !self.touch_hitbox_active {
            return false;
        }
        let (left, top, right, bottom) = self.touch_hitbox;
        let (x, y) = (u32::from(touch.x), u32::from(touch.y));
        x.wrapping_sub(u32::from(left)) < u32::from(right).wrapping_sub(u32::from(left))
            && y.wrapping_sub(u32::from(top)) < u32::from(bottom).wrapping_sub(u32::from(top))
    }
}

/// pret's `TextPrinterTemplate` + `TextPrinter` + its sub-struct —
/// the per-message printing state.
///
/// The template fields keep pret's names where they read as such
/// (`unk1b` is the saved-color register of the `{COLOR}` dance,
/// `unk2e` the `{CTRL 202}` payload scenes read). The units are the
/// message's code units without EOS; the walk finishes when the
/// cursor runs off the end (pret's EOS case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPrinter {
    // The template.
    font_id: u8,
    /// The font asset the printed glyphs reference.
    font: AssetId,
    units: GameString,
    cursor: usize,
    x: u16,
    y: u16,
    current_x: u16,
    current_y: u16,
    letter_spacing: u8,
    line_spacing: u8,
    fg_color: u8,
    shadow_color: u8,
    bg_color: u8,
    /// The `{SIZE}` glyph table: 0, or 0xFFFC for double rows.
    glyph_table: u16,
    unk1b: u8,
    // The printer.
    delay_counter: u8,
    text_speed_bottom: u8,
    unk2e: u16,
    state: u8,
    scroll_distance: u8,
    /// The down-arrow graphics' base tile (pret's
    /// `sDownArrowBaseTile`; `TextPrinter_SetDownArrowBaseTile`'s
    /// value is the constructor argument).
    arrow_base_tile: u16,
    /// The focus-indicator NCGR (`NARC_graphic_font` member 6) for
    /// `{YESNO 0}` blocks.
    focus_gfx: AssetId,
    // The sub-struct.
    has_print_been_sped_up: bool,
    down_arrow_delay: u8,
    down_arrow_ypos_idx: u8,
    auto_scroll_delay: u8,
    // The input facts of the frame (gSystem's held/new edges) and
    // the previous frame's, for the new-press edges.
    new_keys: Keys,
    held_keys: Keys,
    touch_held: bool,
    touch_new: bool,
    touch: Option<Touch>,
    prev_keys: Keys,
    prev_touch: bool,
}

impl TextPrinter {
    /// `AddTextPrinterParameterized`: a printer over `units`,
    /// printing at `(x, y)` in `color` (pret's unpacked
    /// `MAKE_TEXT_COLOR` triple — [`font_color`] for the plain
    /// call's font triple) at `speed` (`TEXT_SPEED_*`).
    ///
    /// `font_id` is the game font whose metrics drive the walk
    /// (`sFonts[fontId]`); `font`/`focus_gfx` are the loaded font
    /// and focus-indicator assets the pushed glyphs and focus state
    /// reference. `arrow_base_tile` is
    /// `TextPrinter_SetDownArrowBaseTile`'s global, set to the
    /// frame's base tile when the scene draws a dialogue border.
    ///
    /// Like `AddTextPrinter`, a per-frame speed prints its first
    /// character on the *construction* frame: pret's
    /// `textSpeedBottom--` is applied here.
    ///
    /// # Panics
    /// Panics when a per-frame speed is out of `1..=0xFE` — pret's
    /// 7-bit `textSpeedBottom` field cannot hold another.
    #[must_use]
    pub fn new(
        font_id: u8,
        font: AssetId,
        focus_gfx: AssetId,
        units: GameString,
        x: u16,
        y: u16,
        color: TextColor,
        speed: u32,
        arrow_base_tile: u16,
    ) -> Self {
        let info = font_info(font_id);
        // AddTextPrinter's split: the synchronous speeds print with
        // textSpeedBottom 0; the per-frame sys task carries speed-1.
        let synchronous = speed == TEXT_SPEED_INSTANT || speed == TEXT_SPEED_NOTRANSFER;
        let text_speed_bottom = if synchronous {
            0
        } else {
            u8::try_from(speed - 1).expect("a per-frame text speed is 1..=0xFE")
        };
        Self {
            font_id,
            font,
            units,
            cursor: 0,
            x,
            y,
            current_x: x,
            current_y: y,
            letter_spacing: info.letter_spacing,
            line_spacing: info.line_spacing,
            fg_color: color.fg,
            shadow_color: color.shadow,
            bg_color: color.bg,
            glyph_table: 0,
            unk1b: 0xFF,
            delay_counter: 0,
            text_speed_bottom,
            unk2e: 0,
            state: 0,
            scroll_distance: 0,
            arrow_base_tile,
            focus_gfx,
            has_print_been_sped_up: false,
            down_arrow_delay: 0,
            down_arrow_ypos_idx: 0,
            auto_scroll_delay: 0,
            new_keys: Keys::IDLE,
            held_keys: Keys::IDLE,
            touch_held: false,
            touch_new: false,
            touch: None,
            prev_keys: Keys::IDLE,
            prev_touch: false,
        }
    }

    /// Whether the string is exhausted (pret frees the printer on
    /// `RENDER_FINISH`).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.state == STATE_FINISHED
    }

    /// The `{CTRL 202}` payload of the frame — `unk2E`, cleared at
    /// each frame start and set by a 0x202 block (the main menu's
    /// per-item tag).
    #[must_use]
    pub fn unk2e(&self) -> u16 {
        self.unk2e
    }

    /// Samples the input edges of the frame — `gSystem`'s held/new
    /// facts, computed once per call.
    fn sample_input(&mut self, input: Input) {
        self.new_keys = input.keys.pressed(self.prev_keys);
        self.prev_keys = input.keys;
        self.held_keys = input.keys;
        self.touch_new = input.touch.is_some() && !self.prev_touch;
        self.prev_touch = input.touch.is_some();
        self.touch_held = input.touch.is_some();
        self.touch = input.touch;
    }

    /// One `RunTextPrinter` call: samples the input edges, runs
    /// `RenderText` while it says `Repeat` (`RenderFont`), and
    /// returns the terminal result.
    pub fn render(
        &mut self,
        font: &Font,
        window: &mut Window,
        input: Input,
        flags: &mut TextFlags,
    ) -> RenderResult {
        self.sample_input(input);
        // RunTextPrinter clears unk2E before each render.
        self.unk2e = 0;
        self.run_frame(font, window, flags)
    }

    /// The synchronous speeds' print: pret's instant path loops
    /// `RenderFont` up to 0x400 times within one frame — the input
    /// edges are sampled once, and a message with a wait state
    /// stops at the wait (the game's own behavior).
    pub fn render_instant(
        &mut self,
        font: &Font,
        window: &mut Window,
        input: Input,
        flags: &mut TextFlags,
    ) {
        self.sample_input(input);
        for _ in 0..0x400 {
            self.unk2e = 0;
            if self.run_frame(font, window, flags) == RenderResult::Finish {
                break;
            }
        }
    }

    /// `RenderFont` — `RenderText` until it stops saying `Repeat`.
    fn run_frame(&mut self, font: &Font, window: &mut Window, flags: &mut TextFlags) -> RenderResult {
        loop {
            match self.render_text(font, window, flags) {
                RenderResult::Repeat => continue,
                other => return other,
            }
        }
    }

    /// `RenderText` — the state machine, verbatim.
    fn render_text(&mut self, font: &Font, window: &mut Window, flags: &mut TextFlags) -> RenderResult {
        match self.state {
            0 => self.state_print(font, window, flags),
            1 => {
                // Wait: no arrow, continue on A/B.
                if self.wait(flags) {
                    self.clear_down_arrow(window);
                    self.state = 0;
                }
                RenderResult::Update
            }
            2 => {
                // Wait with down arrow, then restart the page.
                if self.wait_with_down_arrow(window, flags) {
                    self.clear_down_arrow(window);
                    self.fill_window(window);
                    self.current_x = self.x;
                    self.current_y = self.y;
                    self.state = 0;
                }
                RenderResult::Update
            }
            3 => {
                // Wait with down arrow, then scroll and continue.
                if self.wait_with_down_arrow(window, flags) {
                    self.clear_down_arrow(window);
                    self.scroll_distance = self.line_spacing
                        .wrapping_add(font_info(self.font_id).max_letter_height);
                    self.current_x = self.x;
                    self.state = 4;
                }
                RenderResult::Update
            }
            4 => {
                if self.scroll_distance != 0 {
                    // ScrollWindow's 4-pixel steps (or the
                    // remainder), the fill closing the vacated rows.
                    let step = self.scroll_distance.min(4);
                    window.scroll = window.scroll.wrapping_add(u16::from(step));
                    self.scroll_distance -= step;
                } else {
                    self.state = 0;
                }
                RenderResult::Update
            }
            5 => {
                self.state = 0;
                RenderResult::Update
            }
            6 => {
                // The 0x201 delay: pause, then continue printing.
                if self.delay_counter != 0 {
                    self.delay_counter -= 1;
                } else {
                    self.state = 0;
                }
                RenderResult::Update
            }
            7 => {
                // Like 2 (the 0x207 flavor).
                if self.wait_with_down_arrow(window, flags) {
                    self.clear_down_arrow(window);
                    self.fill_window(window);
                    self.current_x = self.x;
                    self.current_y = self.y;
                    self.state = 0;
                }
                RenderResult::Update
            }
            8 => {
                // Like 3 (the 0x208 flavor).
                if self.wait_with_down_arrow(window, flags) {
                    self.clear_down_arrow(window);
                    self.scroll_distance = font_info(self.font_id)
                        .max_letter_height
                        .wrapping_add(self.line_spacing);
                    self.current_x = self.x;
                    self.state = 4;
                }
                RenderResult::Update
            }
            // pret's switch falls through to RENDER_FINISH for
            // states it never reaches; STATE_FINISHED is the
            // printer's own rest state once the string ended.
            _ => RenderResult::Finish,
        }
    }

    /// State 0 — the print loop, every control-code case of pret's
    /// `RenderText` in its order.
    fn state_print(&mut self, font: &Font, window: &mut Window, flags: &mut TextFlags) -> RenderResult {
        // TextPrinter_ContinueInputHeld.
        if self.continue_input_held(flags) {
            self.delay_counter = 0;
            if self.text_speed_bottom != 0 {
                flags.has_sped_up_input = true;
            }
        }

        if self.delay_counter != 0 && self.text_speed_bottom != 0 {
            self.delay_counter -= 1;
            if flags.can_ab_speed_up_print && self.continue_input_new(flags) {
                self.has_print_been_sped_up = true;
                self.delay_counter = 0;
            }
            return RenderResult::Update;
        }
        self.delay_counter = self.text_speed_bottom;

        let Some(&current_char) = self.units.units().get(self.cursor) else {
            // EOS: the walk ran off the string.
            self.state = STATE_FINISHED;
            return RenderResult::Finish;
        };
        self.cursor += 1;

        match current_char {
            CHAR_LF => {
                // The linefeed advance: lineSpacing +
                // maxLetterHeight (GetFontAttribute(fontId, 1)).
                self.current_x = self.x;
                self.current_y = self
                    .current_y
                    .wrapping_add(u16::from(self.line_spacing))
                    .wrapping_add(u16::from(font_info(self.font_id).max_letter_height));
                RenderResult::Repeat
            }
            0xF0FD => {
                // A skipped unit and the one after it.
                self.cursor += 1;
                RenderResult::Repeat
            }
            EXT_CTRL_CODE_BEGIN => self.control_code(font, window, flags),
            0x25BC => {
                // Wait with the arrow, then restart the page.
                self.state = 2;
                self.init_down_arrow_counters(flags);
                RenderResult::Update
            }
            0x25BD => {
                // Wait with the arrow, then scroll.
                self.state = 3;
                self.init_down_arrow_counters(flags);
                RenderResult::Update
            }
            _ => {
                // TryLoadGlyph + CopyGlyphToWindow: the push the
                // raster blits from, advanced by the glyph's width.
                // (A TRNAME marker reaching here is the C's own
                // GF_ASSERT — the format expands those first.)
                let glyph = font.glyph(current_char);
                window.glyphs.push(WindowGlyph {
                    font: self.font,
                    glyph: current_char,
                    x: self.current_x,
                    // currentY is a buffer row; the glyph's row in the
                    // model is the content row (the window's unscrolled
                    // space), so the cumulative scroll at print time
                    // lands with it — ScrollWindow shifts what was
                    // printed, never what prints next.
                    y: self.current_y.wrapping_add(window.scroll),
                    color: TextColor::new(self.fg_color, self.shadow_color, self.bg_color),
                    double_rows: self.glyph_table == 0xFFFC,
                });
                self.current_x = self
                    .current_x
                    .wrapping_add(u16::from(glyph.width))
                    .wrapping_add(u16::from(self.letter_spacing));
                RenderResult::Print
            }
        }
    }

    /// The `EXT_CTRL_CODE_BEGIN` cases. The cursor has already
    /// advanced past the marker, so each case reads its block from
    /// `cursor - 1` (pret's `currentChar.raw--`).
    fn control_code(&mut self, font: &Font, window: &mut Window, flags: &mut TextFlags) -> RenderResult {
        let block = self.cursor - 1;
        // The fields copy off the string so the case bodies below can
        // borrow the printer freely.
        let (code, fields, len) = match parse_ext_ctrl(&self.units.units()[block..]) {
            Ok((ctrl, len)) => (ctrl.code, ctrl.fields.to_vec(), len),
            // A malformed block: the C would walk past its message;
            // the port refuses (the retail messages all decode).
            Err(_) => {
                self.state = STATE_FINISHED;
                return RenderResult::Finish;
            }
        };
        let field = |n: usize| fields.get(n).copied().unwrap_or(0);

        match code {
            0xFF00 => {
                // The color case, with pret's unk1B save/restore
                // dance around the 0xFF sentinel.
                let mut color_field = field(0);
                if color_field == 0xFF {
                    let saved = self.unk1b;
                    self.unk1b = (self.fg_color.wrapping_sub(1) / 2).wrapping_add(100);
                    if !(100..107).contains(&saved) {
                        // break: the saved register was not a color.
                        self.cursor = block + len;
                        return RenderResult::Repeat;
                    }
                    color_field = u16::from(saved - 100);
                } else if color_field >= 100 {
                    // A saved color to restore later.
                    self.unk1b = color_field as u8;
                    self.cursor = block + len;
                    return RenderResult::Repeat;
                }
                self.fg_color = (color_field.wrapping_mul(2).wrapping_add(1)) as u8;
                self.shadow_color = (color_field.wrapping_mul(2).wrapping_add(2)) as u8;
            }
            0x200 => {
                // RenderScreenFocusIndicatorTile: frame `field` of
                // the focus NCGR blit at ((width-3)·8, 0) — the
                // blit's scroll base is the window's current one.
                window.focus = Some(WindowFocus {
                    asset: self.focus_gfx,
                    index: field(0) as u8,
                    scroll: window.scroll,
                });
            }
            0x207 => {
                self.state = 7;
                self.init_down_arrow_counters(flags);
                self.cursor = block + len;
                if self.units.units().get(self.cursor) == Some(&CHAR_LF) {
                    self.cursor += 1;
                }
                return RenderResult::Update;
            }
            0x208 => {
                self.state = 8;
                self.init_down_arrow_counters(flags);
                self.cursor = block + len;
                if self.units.units().get(self.cursor) == Some(&CHAR_LF) {
                    self.cursor += 1;
                }
                return RenderResult::Update;
            }
            0x201 => {
                self.delay_counter = field(0) as u8;
                self.cursor = block + len;
                self.state = 6;
                return RenderResult::Update;
            }
            0x202 => {
                self.unk2e = field(0);
                self.cursor = block + len;
                return RenderResult::Update;
            }
            0x203 => self.current_x = field(0),
            0x204 => self.current_y = field(0),
            0x205 => {
                // {ALN_CENTER}: center the rest of the first line.
                let width = u32::from(window.width) * 8;
                let line = font.first_line_width(
                    &self.units.units()[block..],
                    self.letter_spacing,
                );
                self.current_x = if line < width {
                    u32::from(self.x) + (width - line) / 2
                } else {
                    u32::from(self.x)
                } as u16;
            }
            0x206 => {
                // {ALN_RIGHT}: right-align the rest of the first line.
                let width = u32::from(window.width) * 8;
                let line = font.first_line_width(
                    &self.units.units()[block..],
                    self.letter_spacing,
                );
                self.current_x = if line < width {
                    width - line
                } else {
                    u32::from(self.x)
                } as u16;
            }
            0xFF01 => {
                // {SIZE}: 100 restores the normal table, 200 the
                // double-rows table (pret's unk1A is cleared
                // alongside and never read, so it stays unmodeled).
                match field(0) {
                    100 => self.glyph_table = 0,
                    200 => self.glyph_table = 0xFFFC,
                    _ => {}
                }
            }
            0xFE06 => {
                // The auto-scroll waits: 0xFE00 like 0x25BD, 0xFE01
                // like 0x25BC.
                match field(0) {
                    0xFE00 => {
                        self.state = 3;
                        self.init_down_arrow_counters(flags);
                        self.cursor = block + len;
                        return RenderResult::Update;
                    }
                    0xFE01 => {
                        self.state = 2;
                        self.init_down_arrow_counters(flags);
                        self.cursor = block + len;
                        return RenderResult::Update;
                    }
                    _ => {}
                }
            }
            // Every other block (the strvars have already been
            // expanded by MessageFormat): skipped like the C's
            // fall-through.
            _ => {}
        }

        // MsgArray_SkipControlCode + RENDER_REPEAT.
        self.cursor = block + len;
        RenderResult::Repeat
    }

    /// `TextPrinter_ContinueInputHeld`.
    fn continue_input_held(&mut self, flags: &mut TextFlags) -> bool {
        if self.held_keys.any(key::A | key::B) && self.has_print_been_sped_up {
            flags.touch_is_speeding_up_print = false;
            return true;
        }
        if flags.can_touch_speed_up_print {
            if !self.touch_held {
                return false;
            }
            if flags.touch_hitbox_active {
                if let Some(touch) = self.touch {
                    if flags.touch_is_in(touch) {
                        flags.touch_is_speeding_up_print = true;
                        return true;
                    }
                }
                return false;
            }
            flags.touch_is_speeding_up_print = true;
            return true;
        }
        false
    }

    /// `TextPrinter_ContinueInputNew`.
    fn continue_input_new(&mut self, flags: &mut TextFlags) -> bool {
        if self.new_keys.any(key::A | key::B) {
            flags.touch_is_speeding_up_print = false;
            return true;
        }
        if flags.can_touch_speed_up_print {
            if !self.touch_new {
                return false;
            }
            if flags.touch_hitbox_active {
                if let Some(touch) = self.touch {
                    if flags.touch_is_in(touch) {
                        flags.touch_is_speeding_up_print = true;
                        return true;
                    }
                }
                return false;
            }
            flags.touch_is_speeding_up_print = true;
            return true;
        }
        false
    }

    /// `TextPrinter_Continue`: A/B pressed continues (and marks the
    /// scene's hasContinuedInput; the PlaySE is deferred audio).
    fn continue_wait(&mut self, flags: &mut TextFlags) -> bool {
        if self.continue_input_new(flags) {
            flags.has_continued_input = true;
            return true;
        }
        false
    }

    /// `TextPrinter_WaitAutoMode`.
    fn wait_auto_mode(&mut self, flags: &mut TextFlags) -> bool {
        if self.auto_scroll_delay == AUTO_SCROLL_WAIT {
            return true;
        }
        self.auto_scroll_delay += 1;
        if flags.auto_scroll_can_speed_up {
            return self.continue_wait(flags);
        }
        false
    }

    /// `TextPrinter_WaitWithDownArrow`.
    fn wait_with_down_arrow(&mut self, window: &mut Window, flags: &mut TextFlags) -> bool {
        if flags.auto_scroll {
            return self.wait_auto_mode(flags);
        }
        self.draw_down_arrow(window, flags);
        self.continue_wait(flags)
    }

    /// `TextPrinter_Wait`.
    fn wait(&mut self, flags: &mut TextFlags) -> bool {
        if flags.auto_scroll {
            return self.wait_auto_mode(flags);
        }
        self.continue_wait(flags)
    }

    /// `TextPrinter_InitDownArrowCounters`.
    fn init_down_arrow_counters(&mut self, flags: &TextFlags) {
        if flags.auto_scroll {
            self.auto_scroll_delay = 0;
        } else {
            self.down_arrow_ypos_idx = 0;
            self.down_arrow_delay = 0;
        }
        // sub_0200EB68 assembles the arrow over border tiles 10/11.
        // The scene's dialogue-frame loader prepares those three poses;
        // the model's arrow resolves through the owning layer at raster time.
    }

    /// `TextPrinter_DrawDownArrow`: the 2×2-tile animation state.
    fn draw_down_arrow(&mut self, window: &mut Window, flags: &TextFlags) {
        if flags.auto_scroll || flags.use_alternate_down_arrow {
            return;
        }
        if self.down_arrow_delay != 0 {
            self.down_arrow_delay -= 1;
            return;
        }
        window.arrow = Some(WindowArrow {
            base_tile: self.arrow_base_tile,
            index: self.down_arrow_ypos_idx % DOWN_ARROW_FRAMES,
        });
        self.down_arrow_delay = DOWN_ARROW_DELAY;
        // Pret increments unbounded (its own comment asks where the
        // wrap is); the port cycles the four-entry offset table.
        self.down_arrow_ypos_idx = self.down_arrow_ypos_idx.wrapping_add(1);
    }

    /// `TextPrinter_ClearDownArrow` — pret writes blank tiles; the
    /// model restores the dialogue border underneath the arrow.
    fn clear_down_arrow(&mut self, window: &mut Window) {
        window.arrow = None;
    }

    /// `FillWindowPixelBuffer`: the fill becomes the window's, and
    /// the glyph and focus state it erases goes with it — the blit
    /// and the glyphs all lived in that buffer.
    fn fill_window(&mut self, window: &mut Window) {
        window.fill = self.bg_color;
        window.glyphs.clear();
        window.focus = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font like `font.rs`'s fixture: 8×8 glyphs, widths 8..12
    /// cycling (glyph 1 → 8, 2 → 9, …), all-zero glyph pixels — the
    /// printer reads only the widths.
    fn test_font() -> Font {
        let n = 428usize;
        let mut data = Vec::new();
        data.extend_from_slice(&(16u32 + n as u32).to_le_bytes());
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&(n as u32).to_le_bytes());
        data.push(0); // fixedWidth (unused: the width table is on)
        data.push(8); // fixedHeight
        data.push(1); // glyphWidth (tiles)
        data.push(1); // glyphHeight (tiles)
        for i in 0..n {
            data.push(8 + (i % 5) as u8);
        }
        data.extend(std::iter::repeat(0u8).take(n * 16));
        Font::parse(&data).expect("the fixture font parses")
    }

    /// A printer over `units` (font id 0: letter/line spacing 0,
    /// linefeed advance 16), font asset 0, focus asset 1.
    fn make_printer(units: &[u16], speed: u32) -> TextPrinter {
        TextPrinter::new(
            0,
            AssetId::FIRST,
            AssetId::FIRST.next(),
            GameString::from_units(units),
            0,
            0,
            TextColor::new(1, 2, 0xF),
            speed,
            0,
        )
    }

    /// A 27×4-tile window (the dialog geometry the alignment codes
    /// measure against).
    fn make_window() -> Window {
        Window {
            width: 27,
            height: 4,
            ..Window::default()
        }
    }

    /// Input with `mask` held (a `key::*` bit set), no touch.
    fn keys(mask: u16) -> Input {
        Input {
            keys: Keys(mask),
            touch: None,
        }
    }

    #[test]
    fn instant_print_pushes_every_glyph_and_finishes() {
        // "AB" as glyph ids 1 and 2: widths 8 and 9, spacing 0, so
        // the second glyph lands at x 8 and the walk finishes.
        let mut printer = make_printer(&[1, 2], TEXT_SPEED_INSTANT);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        assert!(printer.is_finished());
        assert_eq!(window.glyphs.len(), 2);
        assert_eq!(window.glyphs[0].x, 0);
        assert_eq!(window.glyphs[1].x, 8);
        assert_eq!(window.glyphs[0].color, TextColor::new(1, 2, 0xF));
        assert!(!window.glyphs[0].double_rows);
    }

    #[test]
    fn per_frame_speed_paces_one_glyph_per_speed() {
        // Speed 4: the construction-time `textSpeedBottom--` makes
        // the first glyph print on frame 0, the rest wait 3 frames.
        let mut printer = make_printer(&[1, 2], 4);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        // Frame 0: the first glyph.
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Print
        );
        assert_eq!(window.glyphs.len(), 1);
        // Frames 1-3: the delay counts down.
        for _ in 0..3 {
            assert_eq!(
                printer.render(&test_font(), &mut window, Input::default(), &mut flags),
                RenderResult::Update
            );
        }
        // Frame 4: the second glyph.
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Print
        );
        assert_eq!(window.glyphs.len(), 2);
        // Then the delay, then EOS.
        for _ in 0..3 {
            assert_eq!(
                printer.render(&test_font(), &mut window, Input::default(), &mut flags),
                RenderResult::Update
            );
        }
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Finish
        );
        assert!(printer.is_finished());
    }

    #[test]
    fn new_a_press_skips_the_delay_and_speeds_up() {
        // canABSpeedUpPrint: a new A mid-delay ends it, latching
        // hasPrintBeenSpedUp. ContinueInputNew marks no flag of its
        // own — the held frames after carry hasSpedUpInput.
        let mut printer = make_printer(&[1, 2], 4);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        flags.set_can_ab_speed_up_print(true);
        // Frame 0: the first glyph (delay 3 armed after it).
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Print
        );
        // Frame 1: the delay ticks down and the new A ends it — an
        // Update, not a print.
        assert_eq!(
            printer.render(&test_font(), &mut window, keys(key::A), &mut flags),
            RenderResult::Update
        );
        assert!(!flags.has_sped_up_input(), "the new-key path marks no flag");
        // Frame 2: held A latches the speedup — the delay zeroes and
        // the second glyph prints, the frame marked sped up.
        assert_eq!(
            printer.render(&test_font(), &mut window, keys(key::A), &mut flags),
            RenderResult::Print
        );
        assert_eq!(window.glyphs.len(), 2);
        assert!(flags.has_sped_up_input());
        flags.reset_has_sped_up_input();
        assert!(!flags.has_sped_up_input());
        // Frame 3: still held — the walk runs straight to EOS.
        assert_eq!(
            printer.render(&test_font(), &mut window, keys(key::A), &mut flags),
            RenderResult::Finish
        );
    }

    #[test]
    fn linefeed_advances_a_full_line_and_returns_to_x() {
        let mut printer = make_printer(&[1, CHAR_LF, 2], TEXT_SPEED_INSTANT);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        // Line spacing 0 + maxLetterHeight 16 (font 0).
        assert_eq!(window.glyphs[1].y, 16);
        assert_eq!(window.glyphs[1].x, 0);
    }

    #[test]
    fn color_control_sets_the_triple_and_the_unk1b_dance_restores() {
        // {COLOR 3}: fg 7, shadow 8 — then {COLOR 255} twice: the
        // first saves (fg-1)/2+100 and breaks (the initial unk1B
        // 0xFF is out of range), the second restores field
        // (saved - 100) = 3. Both glyphs carry color 3.
        let units = [
            0xFFFE, 0xFF00, 1, 3, // {COLOR 3}
            1,
            0xFFFE, 0xFF00, 1, 0xFF, // {COLOR 255}: save & break
            0xFFFE, 0xFF00, 1, 0xFF, // {COLOR 255}: restore 3
            2,
        ];
        let mut printer = make_printer(&units, TEXT_SPEED_INSTANT);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        assert_eq!(window.glyphs[0].color, TextColor::new(7, 8, 0xF));
        assert_eq!(window.glyphs[1].color, TextColor::new(7, 8, 0xF));
        // A {COLOR 100} block saves register 100 *without* changing
        // the print; a following {COLOR 255} restores field 0 →
        // fg 1, shadow 2.
        let units = [
            0xFFFE, 0xFF00, 1, 3, // {COLOR 3}
            0xFFFE, 0xFF00, 1, 100, // {COLOR 100}: save, no change
            1,
            0xFFFE, 0xFF00, 1, 0xFF, // {COLOR 255}: restore 0
            2,
        ];
        let mut printer = make_printer(&units, TEXT_SPEED_INSTANT);
        let mut window = make_window();
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        assert_eq!(window.glyphs[0].color, TextColor::new(7, 8, 0xF));
        assert_eq!(window.glyphs[1].color, TextColor::new(1, 2, 0xF));
    }

    #[test]
    fn yesno_block_sets_the_focus_indicator() {
        // msg_0249_00000's tail: `{YESNO 0}` is a 0x200 block whose
        // field is the indicator frame.
        let units = [1, 0xFFFE, 0x0200, 1, 0];
        let mut printer = make_printer(&units, TEXT_SPEED_INSTANT);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        let focus = window.focus.expect("the block set the focus");
        assert_eq!(focus.index, 0);
        assert_eq!(focus.asset, AssetId::FIRST.next());
        assert_eq!(focus.scroll, 0, "no scroll had run");
    }

    #[test]
    fn wait_states_draw_the_arrow_and_continue_on_a() {
        // "A" then 0x25BC: the print runs, then state 2 waits with
        // the arrow until a new A/B. B is held throughout so its
        // press is never new once the wait starts.
        let mut printer = make_printer(&[1, 0x25BC], 1);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        // Frame 0: the glyph (speed 1 carries no delay).
        assert_eq!(
            printer.render(&test_font(), &mut window, keys(key::B), &mut flags),
            RenderResult::Print
        );
        // Frame 1: the wait state is entered — no arrow has drawn.
        assert_eq!(
            printer.render(&test_font(), &mut window, keys(key::B), &mut flags),
            RenderResult::Update
        );
        assert!(window.arrow.is_none());
        // Frame 2: the first wait frame draws the arrow at index 0.
        assert_eq!(
            printer.render(&test_font(), &mut window, keys(key::B), &mut flags),
            RenderResult::Update
        );
        assert_eq!(window.arrow.expect("the wait drew the arrow").index, 0);
        // The 8-frame delay holds index 0.
        for _ in 0..8 {
            assert_eq!(
                printer.render(&test_font(), &mut window, keys(key::B), &mut flags),
                RenderResult::Update
            );
            assert_eq!(window.arrow.expect("still waiting").index, 0);
        }
        // The next frame steps the animation to index 1.
        assert_eq!(
            printer.render(&test_font(), &mut window, keys(key::B), &mut flags),
            RenderResult::Update
        );
        assert_eq!(window.arrow.expect("still waiting").index, 1);
        assert!(!flags.has_continued_input(), "held B never continues");
        // A new A continues: the arrow clears, the page's fill and
        // glyphs reset, and the walk runs off the string's end.
        assert_eq!(
            printer.render(&test_font(), &mut window, keys(key::A), &mut flags),
            RenderResult::Update
        );
        assert!(window.arrow.is_none(), "cleared on continue");
        assert!(window.glyphs.is_empty(), "the page restarted");
        assert_eq!(window.fill, 0xF, "the fill is the bg color");
        assert!(flags.has_continued_input());
        flags.reset_has_continued_input();
        assert!(!flags.has_continued_input());
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Finish
        );
    }

    #[test]
    fn scroll_wait_clears_the_arrow_and_scrolls_four_pixels_per_frame() {
        // 0x25BD: the wait, then a 16-pixel scroll (maxLetterHeight +
        // lineSpacing) in 4-pixel steps, then printing continues at
        // the same buffer row — a full line lower in content space.
        let units = [1, 0x25BD, 2];
        let mut printer = make_printer(&units, TEXT_SPEED_INSTANT);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        // The synchronous print runs into the wait and stops there —
        // the game's own behavior, the 0x400 budget burning in state
        // 3's wait.
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        assert_eq!(window.glyphs.len(), 1);
        assert!(window.arrow.is_some(), "the wait drew the arrow");
        assert!(!printer.is_finished());
        assert_eq!(window.scroll, 0);
        // A new A releases the wait into the scroll state — the
        // release frame scrolls nothing, and the printed glyph stays
        // in the buffer (ScrollWindow shifts it; it does not erase).
        assert_eq!(
            printer.render(&test_font(), &mut window, keys(key::A), &mut flags),
            RenderResult::Update
        );
        assert!(window.arrow.is_none());
        assert_eq!(window.glyphs.len(), 1);
        assert_eq!(window.scroll, 0);
        // Four 4-pixel steps.
        for expected in [4u16, 8, 12, 16] {
            assert_eq!(
                printer.render(&test_font(), &mut window, Input::default(), &mut flags),
                RenderResult::Update
            );
            assert_eq!(window.scroll, expected);
        }
        // The frame the distance runs out hands the machine back to
        // print; the next prints the continuation.
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Update
        );
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Print
        );
        // The continuation printed at the same currentY (buffer
        // row 0): its content row is a full line below the
        // scrolled-off one.
        assert_eq!(window.glyphs[1].y, 16);
        assert_eq!(window.glyphs[1].x, 0);
    }

    #[test]
    fn delay_control_code_pauses_for_its_field() {
        // {CTRL 201 6}: state 6 runs one Update per frame — eight
        // between the two prints (the block's own frame, the six
        // counts, the hand-back to print).
        let units = [1, 0xFFFE, 0x0201, 1, 6, 2];
        let mut printer = make_printer(&units, 1);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Print
        );
        assert_eq!(window.glyphs.len(), 1);
        for _ in 0..8 {
            assert_eq!(
                printer.render(&test_font(), &mut window, Input::default(), &mut flags),
                RenderResult::Update
            );
        }
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Print
        );
        assert_eq!(window.glyphs.len(), 2);
    }

    #[test]
    fn ctrl_202_publishes_its_field_for_the_scene() {
        let units = [0xFFFE, 0x0202, 1, 5, 1];
        let mut printer = make_printer(&units, 1);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        printer.render(&test_font(), &mut window, Input::default(), &mut flags);
        assert_eq!(printer.unk2e(), 5);
        // The next frame clears it (RunTextPrinter's reset).
        printer.render(&test_font(), &mut window, Input::default(), &mut flags);
        assert_eq!(printer.unk2e(), 0);
    }

    #[test]
    fn position_and_alignment_codes_place_the_next_line() {
        // {CTRL 203 32}/{CTRL 204 24}: absolute x/y; then {ALN_CENTER}
        // centers the remaining first line: window 27 tiles = 216
        // px, the line is one glyph of width 8 → x = (216-8)/2 = 104.
        let units = [0xFFFE, 0x0203, 1, 32, 0xFFFE, 0x0204, 1, 24, 0xFFFE, 0x0205, 0, 1];
        let mut printer = make_printer(&units, TEXT_SPEED_INSTANT);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        assert_eq!(window.glyphs[0].x, 104);
        assert_eq!(window.glyphs[0].y, 24);
        // {ALN_RIGHT}: x = 216 - width. Two glyphs, widths 8 and 9:
        // 216 - (8 + 9) = 199.
        let units = [0xFFFE, 0x0206, 0, 1, 2];
        let mut printer = make_printer(&units, TEXT_SPEED_INSTANT);
        let mut window = make_window();
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        assert_eq!(window.glyphs[0].x, 199);
    }

    #[test]
    fn size_control_switches_the_row_doubling_table() {
        let units = [0xFFFE, 0xFF01, 1, 200, 1, 0xFFFE, 0xFF01, 1, 100, 2];
        let mut printer = make_printer(&units, TEXT_SPEED_INSTANT);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        assert!(window.glyphs[0].double_rows, "the {{SIZE 200}} block doubles");
        assert!(!window.glyphs[1].double_rows, "the {{SIZE 100}} block restores");
    }

    #[test]
    fn unknown_control_blocks_are_skipped() {
        // A strvar-class block the printer never interprets (the
        // format expands those before printing): skipped whole.
        let units = [1, 0xFFFE, 0x0101, 2, 0, 0, 2];
        let mut printer = make_printer(&units, TEXT_SPEED_INSTANT);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        assert_eq!(window.glyphs.len(), 2);
        assert_eq!(window.glyphs[1].x, 8);
    }

    #[test]
    fn auto_scroll_waits_run_themselves_off() {
        // BeginAutoScroll(FALSE): the wait advances on a 100-frame
        // timer — the release is the frame the counter reads 100 at
        // entry, the 101st wait frame — the arrow never draws, and
        // the 0x25BC restart clears the page without scrolling.
        let mut printer = make_printer(&[1, 0x25BC, 2], 1);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        flags.begin_auto_scroll(false);
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Print
        );
        // The wait state is entered, then runs itself off.
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Update
        );
        for _ in 0..101 {
            assert_eq!(
                printer.render(&test_font(), &mut window, Input::default(), &mut flags),
                RenderResult::Update
            );
            assert!(window.arrow.is_none(), "auto scroll draws no arrow");
        }
        assert!(window.glyphs.is_empty(), "the page restarted");
        assert_eq!(window.scroll, 0, "state 2 restarts, not scrolls");
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Print,
            "the restart prints the continuation"
        );
    }

    #[test]
    fn touch_speedup_skips_the_delay_when_enabled() {
        let mut printer = make_printer(&[1, 2], 8);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        flags.set_can_touch_speed_up_print(true);
        flags.set_fast_forward_touch_button_hitbox((0, 0, 64, 64));
        printer.render(&test_font(), &mut window, Input::default(), &mut flags);
        assert_eq!(window.glyphs.len(), 1);
        // A touch inside the hitbox zeroes the delay.
        let input = Input {
            keys: Keys::IDLE,
            touch: Some(Touch { x: 10, y: 10 }),
        };
        assert_eq!(
            printer.render(&test_font(), &mut window, input, &mut flags),
            RenderResult::Print
        );
        assert!(flags.is_touch_speeding_up_print());
        // Outside the hitbox: the delay holds.
        let input = Input {
            keys: Keys::IDLE,
            touch: Some(Touch { x: 200, y: 10 }),
        };
        assert_eq!(
            printer.render(&test_font(), &mut window, input, &mut flags),
            RenderResult::Update
        );
    }

    #[test]
    fn alternate_down_arrow_and_finish_restate_cleanly() {
        // useAlternateDownArrow: the wait draws nothing; a second
        // render after Finish stays Finish.
        let mut printer = make_printer(&[1, 0x25BC], TEXT_SPEED_INSTANT);
        let mut window = make_window();
        let mut flags = TextFlags::default();
        flags.set_alternate_down_arrow(true);
        printer.render_instant(&test_font(), &mut window, Input::default(), &mut flags);
        assert!(window.arrow.is_none(), "the alternate arrow suppresses");
        assert!(!printer.is_finished());
        printer.render(&test_font(), &mut window, keys(key::A), &mut flags);
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Finish
        );
        assert_eq!(
            printer.render(&test_font(), &mut window, Input::default(), &mut flags),
            RenderResult::Finish,
            "the finished printer rests"
        );
    }
}
