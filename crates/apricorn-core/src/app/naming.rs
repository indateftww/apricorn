//! Player name entry, from `src/naming_screen.c`.
//!
//! The three localized keyboard pages are read through the retail ARM9's
//! pointer tables, not transcribed. The scene owns input, page slides, entry
//! limits and fades; its caller supplies the main LCRNG only at overlay exit
//! when an empty/all-space entry needs a default name. Other naming modes
//! (Pokémon, boxes, groups) belong to their respective later scenes.

use std::sync::Mutex;

use super::fade::{BrightnessFade, FadeColor, FadeScreens, FadeType};
use super::{App, ChainNext};
use crate::assets::{AssetStore, AssetsError, font_narc, frame_narc, msg_narc};
use crate::font::Font;
use crate::frame::{
    AssetId, BgLayer, DisplaySelect, LogicalFrame, PaletteLoad, ScreenSize, Sprite, TextColor,
    TilePlacement, Window, WindowFrame, WindowGlyph,
};
use crate::input::{Input, Keys, Touch, key};
use crate::rng::Lcrng;
use crate::text::string::GameString;

const NARC: &str = "a/0/3/1";
const SKIP: u16 = 0xD004;
const UPPER: u16 = 0xE002;
const BACK: u16 = 0xE007;
const OK: u16 = 0xE008;
const SPACE: u16 = 478;
/// The player-name limit passed by `OakSpeech_Init`.
pub const PLAYER_NAME_LEN: usize = 7;

/// The observable naming overlay stages (init, main and exit combined).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingState {
    /// First graphics-init pass.
    Init,
    /// Second init pass (the player mode has no Pokémon icon load).
    InitIcon,
    /// Wait for the 16-step brightness-in.
    FadeIn,
    /// Six ticks of cursor setup.
    FinishInit,
    /// Keyboard input, or a keyboard page slide.
    Input,
    /// Fade out the sub engine first.
    FadeOutSub,
    /// Then fade out the main engine.
    FadeOutMain,
    /// Overlay exit; the caller can collect the result.
    Done,
}

/// Oak's nested player-naming overlay.
pub struct NamingScreen {
    frame: LogicalFrame,
    state: NamingState,
    fade: BrightnessFade,
    delay: u8,
    pages: [[[u16; 13]; 6]; 3],
    page: usize,
    cursor: (usize, usize),
    delta_x: i32,
    prev: Input,
    repeat_delay: u8,
    entry: Vec<u16>,
    default_names: Vec<GameString>,
    result: Option<GameString>,
    font: Font,
    font_asset: AssetId,
    screens: [AssetId; 3],
    keyboard_windows: [Window; 2],
    active_bg: usize,
    slide_state: u8,
    positions: [(i16, i16); 2],
    sprite: Sprite,
    cursor_elapsed: u32,
    cursor_duration: u32,
    cursor_press: bool,
    elapsed: u32,
    gender: u8,
    glow_angle: u16,
    glow_sin: [i16; 18],
    wiggle: Option<u8>,
    button_age: [Option<u32>; 2],
}

impl NamingScreen {
    /// Loads the ROM tables and graphics for Oak's empty player-name entry.
    ///
    /// # Errors
    /// Returns an asset error for a missing or invalid ROM member.
    /// # Panics
    /// Panics for a gender other than the game's two avatar selections.
    pub fn load(store: &Mutex<AssetStore>, gender: u8) -> Result<Self, AssetsError> {
        assert!(gender <= 1);
        let mut store = store.lock().expect("asset store lock");
        let image = store.arm9_image()?;
        // Discovered by following the unique upper A..J,space,comma,period
        // row at 0x02101E80 back to its pointer at 0x021104F8.
        let word = |address: usize| -> u32 {
            u32::from_le_bytes(
                image[address - 0x02000000..address - 0x02000000 + 4]
                    .try_into()
                    .unwrap(),
            )
        };
        let mut pages = [[[0; 13]; 6]; 3];
        for (page, rows) in pages.iter_mut().enumerate() {
            for (row, keys) in rows.iter_mut().enumerate() {
                let pointer = if row == 0 {
                    0x021104E4 + page * 4
                } else {
                    0x021104F8 + (page * 5 + row - 1) * 4
                };
                let offset = word(pointer) as usize - 0x02000000;
                for (i, key) in keys.iter_mut().enumerate() {
                    *key = u16::from_le_bytes(
                        image[offset + i * 2..offset + i * 2 + 2]
                            .try_into()
                            .unwrap(),
                    );
                }
            }
        }
        let font_asset = store.load_font(font_narc::NARC, font_narc::FONT0)?;
        let font = store.font(font_asset).expect("loaded font").clone();
        let names = store.load_msg_bank(msg_narc::NARC, 254)?;
        let default_names = (usize::from(gender) * 18..usize::from(gender) * 18 + 18)
            .map(|i| GameString::from_units(store.msg_bank(names).unwrap().message(i).unwrap()))
            .collect();
        let messages = store.load_msg_bank(msg_narc::NARC, 249)?;
        let prompt = GameString::from_units(store.msg_bank(messages).unwrap().message(0).unwrap());
        let font_pal = store.load_palette(font_narc::NARC, font_narc::PAL1)?;
        let tiles = store.load_tiles(NARC, 2)?;
        let palette = store.load_palette(NARC, 0)?;
        let screens = [
            store.load_screen(NARC, 6)?,
            store.load_screen(NARC, 7)?,
            store.load_screen(NARC, 8)?,
        ];
        let sprite = Sprite {
            tiles: store.load_tiles(NARC, 10)?,
            palette: store.load_palette(NARC, 1)?,
            cells: store.load_cells(NARC, 12)?,
            animation: store.load_animation(NARC, 14)?,
            sequence: 0,
            elapsed: 0,
            x: 0,
            y: 0,
            priority: 1,
            palette_bank: 0,
        };
        let cursor_duration = store.animation(sprite.animation).unwrap().sequences()[60]
            .frames
            .iter()
            .map(|f| u32::from(f.delay))
            .sum();
        let mut frame = LogicalFrame {
            display: DisplaySelect::SubOnTop,
            ..LogicalFrame::default()
        };
        for i in 0..2 {
            frame.main.bgs[i] = BgLayer {
                enabled: true,
                char_base: 2,
                screen: Some(screens[if i == 0 { 1 } else { 0 }]),
                size: ScreenSize::W512xH256,
                priority: (i + 1) as u8,
                scroll_x: if i == 0 { 238 } else { 501 },
                scroll_y: 432,
                hidden_rect: Some((0, 0, 255, 64)),
                ..BgLayer::default()
            };
        }
        frame.main.bgs[2] = BgLayer {
            enabled: true,
            screen: Some(store.load_screen(NARC, 4)?),
            priority: 3,
            ..BgLayer::default()
        };
        frame.main.char_blocks[0].push(TilePlacement {
            asset: tiles,
            tile: 0,
        });
        frame.main.char_blocks[2].push(TilePlacement {
            asset: tiles,
            tile: 0,
        });
        frame.main.palette_loads.push(PaletteLoad {
            asset: palette,
            offset: 0,
            colors: 48,
        });
        frame.sub.bgs[0] = BgLayer {
            enabled: true,
            char_base: 2,
            ..BgLayer::default()
        };
        frame.sub.palette_loads.push(PaletteLoad {
            asset: font_pal,
            offset: 192,
            colors: 16,
        });
        let mut prompt_window = Window {
            bg: 0,
            left: 2,
            top: 19,
            width: 27,
            height: 4,
            palette: 12,
            base_tile: 0x84,
            fill: 15,
            ..Window::default()
        };
        let dialog_font = store.load_font(font_narc::NARC, font_narc::FONT1)?;
        let focus = store.load_tiles(font_narc::NARC, font_narc::FOCUS_INDICATOR)?;
        let mut printer = super::text::TextPrinter::new(
            1,
            dialog_font,
            focus,
            prompt,
            0,
            0,
            super::text::font_color(1),
            0,
            0x100,
        );
        printer.render_instant(
            store.font(dialog_font).unwrap(),
            &mut prompt_window,
            Input::default(),
            &mut super::text::TextFlags::default(),
        );
        let border = store.load_default_dialogue_frame()?;
        frame.sub.char_blocks[2].push(TilePlacement {
            asset: border,
            tile: 0x100,
        });
        frame.sub.palette_loads.push(PaletteLoad {
            asset: store.load_palette(frame_narc::NARC, frame_narc::GFX2_FRAME0_PALETTE)?,
            offset: 160,
            colors: 16,
        });
        prompt_window.frame = Some(WindowFrame {
            base_tile: 0x100,
            palette: 10,
            dialogue: true,
        });
        frame.sub.windows.push(prompt_window);
        let keyboard_windows = std::array::from_fn(|i| Window {
            bg: i as u8,
            left: 2,
            top: 1,
            width: 26,
            height: 12,
            palette: 1,
            base_tile: if i == 0 { 0x100 } else { 0x238 },
            ..Window::default()
        });
        let mut app = Self {
            frame,
            state: NamingState::Init,
            fade: BrightnessFade::default(),
            delay: 0,
            pages,
            page: 0,
            cursor: (0, 1),
            delta_x: 0,
            prev: Input::default(),
            repeat_delay: 8,
            entry: Vec::new(),
            default_names,
            result: None,
            font,
            font_asset,
            screens,
            keyboard_windows,
            active_bg: 0,
            slide_state: 4,
            positions: [(238, -80), (-11, -80)],
            sprite,
            cursor_elapsed: 0,
            cursor_duration,
            cursor_press: false,
            elapsed: 0,
            gender,
            glow_angle: 180,
            glow_sin: std::array::from_fn(|i| {
                // FX_DEG_TO_IDX followed by FX_SinIdx's low-four-bit
                // truncation, reading the original NitroSDK sine table.
                let idx = ((i * 20 * 65536 + 180) / 360) >> 4;
                let p = 0x1094dc + idx * 4;
                i16::from_le_bytes(image[p..p + 2].try_into().unwrap())
            }),
            wiggle: None,
            button_age: [None; 2],
        };
        app.draw_keyboard(1);
        app.compose();
        app.frame.main.brightness = crate::frame::MasterBrightness {
            mode: crate::frame::BrightnessMode::Down,
            value: 16,
        };
        app.frame.sub.brightness = app.frame.main.brightness;
        Ok(app)
    }

    /// Current overlay stage, useful for deterministic drivers.
    pub fn state(&self) -> NamingState {
        self.state
    }
    /// Selected keyboard page: upper, lower, symbols (0–2).
    pub fn page(&self) -> usize {
        self.page
    }
    /// Keyboard column and row (row zero contains the page/Back/OK buttons).
    pub fn cursor(&self) -> (usize, usize) {
        self.cursor
    }
    /// Current name before the exit's default-name substitution.
    pub fn entry(&self) -> &[u16] {
        &self.entry
    }
    /// Whether input can be accepted this tick.
    pub fn accepts_input(&self) -> bool {
        self.state == NamingState::Input
            && self.slide_state == 4
            && !(self.cursor_press && self.entry.len() == PLAYER_NAME_LEN)
    }
    /// Keyboard data loaded from the ROM (including its home row).
    pub fn keyboard(&self, page: usize) -> &[[u16; 13]; 6] {
        &self.pages[page]
    }

    /// Resolves `NamingScreenApp_Exit` once. Repeated reads do not draw RNG.
    /// # Panics
    /// Panics before the overlay has finished fading out.
    pub fn result(&mut self, rng: &mut Lcrng) -> &GameString {
        assert_eq!(self.state, NamingState::Done);
        self.result.get_or_insert_with(|| {
            if self.entry.iter().all(|&c| c == SPACE) {
                self.default_names[usize::from(rng.next_u16() % 18)].clone()
            } else {
                GameString::from_units(&self.entry)
            }
        })
    }

    fn draw_keyboard(&mut self, bg: usize) {
        let window = &mut self.keyboard_windows[bg];
        window.fill = [4, 7, 13][self.page];
        window.glyphs.clear();
        window.fills.clear();
        for row in 0..5 {
            for col in 0..13 {
                if (row + col) % 2 == 1 {
                    window.fills.push((
                        col as u16 * 16,
                        row as u16 * 19,
                        16,
                        19,
                        [3, 6, 12][self.page],
                    ));
                }
                let glyph = self.pages[self.page][row + 1][col];
                let center = (16 - u16::from(self.font.char_width(glyph))) / 2;
                window.glyphs.push(WindowGlyph {
                    font: self.font_asset,
                    glyph,
                    x: col as u16 * 16 + center,
                    y: row as u16 * 19 + 4,
                    color: TextColor::new(14, 15, 0),
                    double_rows: false,
                });
            }
        }
    }

    fn switch_page(&mut self, page: usize) {
        if self.page == page {
            return;
        }
        self.page = page;
        self.slide_state = 0;
    }

    fn slide(&mut self) {
        let old = self.active_bg;
        let current = old ^ 1;
        match self.slide_state {
            0 => {
                self.frame.main.bgs[old].screen = Some(self.screens[self.page]);
                self.positions[old] = (238, -80);
                self.positions[current] = (-11, -80);
                self.draw_keyboard(old);
                self.slide_state = 1;
            }
            1 => {
                self.slide_state = 2;
            }
            2 => {
                self.positions[old].0 -= 24;
                if self.positions[old].0 < -1 {
                    self.positions[old].0 = -11;
                    self.slide_state = 3;
                    self.wiggle = Some(0);
                }
                self.positions[current].1 = (self.positions[current].1 - 10).max(-196);
            }
            3 => {
                self.positions[current].1 = (self.positions[current].1 - 10).max(-196);
                if self.positions[current].1 == -196 {
                    self.slide_state = 4;
                    self.active_bg ^= 1;
                    self.frame.main.bgs[self.active_bg].priority = 1;
                    self.frame.main.bgs[self.active_bg ^ 1].priority = 2;
                }
            }
            _ => {}
        }
        for i in 0..2 {
            self.frame.main.bgs[i].scroll_x = self.positions[i].0 as u16 & 511;
            self.frame.main.bgs[i].scroll_y = self.positions[i].1 as u16 & 511;
        }
    }

    fn move_cursor(&mut self, dx: i32, dy: i32) {
        let (x, y) = self.cursor;
        let prev_key = self.pages[self.page][y][x];
        let mut nx = (x as i32 + dx).rem_euclid(13);
        let mut ny = (y as i32 + dy).rem_euclid(6);
        loop {
            let key = self.pages[self.page][ny as usize][nx as usize];
            if key != SKIP && !(key == prev_key && key > 0xE001) {
                break;
            }
            if y == 0 && key == SKIP && dy != 0 {
                nx = (nx + self.delta_x).rem_euclid(13);
            } else {
                nx = (nx + dx).rem_euclid(13);
                ny = (ny + dy).rem_euclid(6);
            }
        }
        self.cursor = (nx as usize, ny as usize);
        if dx != 0 {
            self.delta_x = dx;
        }
        self.cursor_press = false;
        self.cursor_elapsed = 0;
    }

    // The hitboxes are inclusive and overlap at their boundaries; the
    // earlier C array entry wins. In particular adjacent letters share x.
    fn touch_cursor(t: Touch) -> Option<(usize, usize)> {
        for (x, y, w, h, col, row) in [
            (25, 60, 31, 22, 0, 0),
            (57, 60, 31, 22, 2, 0),
            (89, 60, 31, 22, 4, 0),
            (0, 192, 31, 22, 4, 0),
            (157, 60, 32, 22, 8, 0),
            (197, 60, 32, 22, 11, 0),
        ] {
            if t.x >= x && t.x <= x + w && t.y >= y && t.y <= y + h {
                return Some((col, row));
            }
        }
        for row in 1..6 {
            for col in 0..13 {
                let x = 28 + col * 16;
                let y = 88 + (row - 1) * 19;
                if usize::from(t.x) >= x
                    && usize::from(t.x) <= x + 16
                    && usize::from(t.y) >= y
                    && usize::from(t.y) <= y + 19
                {
                    return Some((col, row));
                }
            }
        }
        None
    }

    fn input(&mut self, input: Input, pressed: Keys, repeated: Keys, touch_new: bool) {
        let old_cursor = self.cursor;
        let mut direction = None;
        for (bit, dx, dy) in [
            (key::UP, 0, -1),
            (key::DOWN, 0, 1),
            (key::LEFT, -1, 0),
            (key::RIGHT, 1, 0),
        ] {
            if repeated.any(bit) {
                direction = Some((dx, dy));
            }
        }
        if pressed.any(key::START) {
            self.cursor = (12, 0);
            self.cursor_press = false;
        }
        let touched = input
            .touch
            .filter(|_| touch_new)
            .and_then(Self::touch_cursor);
        if let Some(cursor) = touched {
            self.cursor = cursor;
        } else if let Some((dx, dy)) = direction {
            self.move_cursor(dx, dy);
        }
        if pressed.any(key::SELECT) {
            self.switch_page((self.page + 1) % 3);
        } else if pressed.any(key::A) || touched.is_some() {
            self.character(self.pages[self.page][self.cursor.1][self.cursor.0]);
        } else if pressed.any(key::B) {
            self.character(BACK);
        }
        // R searches sJpCharConvTable. None of the three US keyboard
        // pages' characters occur there, so it leaves player input alone.
        if self.cursor != old_cursor {
            self.glow_angle = 180;
            self.cursor_elapsed = 0;
        }
    }

    fn character(&mut self, code: u16) {
        match code {
            UPPER..=0xE004 => self.switch_page(usize::from(code - UPPER)),
            BACK => {
                self.entry.pop();
                self.button_age[0] = Some(0);
            }
            OK => {
                self.button_age[1] = Some(0);
                self.fade.begin_with_screens(
                    FadeScreens::Sub,
                    FadeType::BrightnessOut,
                    FadeColor::Black,
                    16,
                    1,
                );
                self.state = NamingState::FadeOutSub;
            }
            _ if self.entry.len() < PLAYER_NAME_LEN => {
                self.entry.push(if code == SKIP { 1 } else { code });
                self.cursor_press = true;
                self.cursor_elapsed = 0;
            }
            _ => {}
        }
    }

    fn compose(&mut self) {
        self.frame.main.windows = self.keyboard_windows.to_vec();
        let mut entry = Window {
            bg: 2,
            left: 10,
            top: 3,
            width: 11,
            height: 2,
            fill: 1,
            base_tile: 0x39C,
            ..Window::default()
        };
        for (i, &glyph) in self.entry.iter().enumerate() {
            entry.glyphs.push(WindowGlyph {
                font: self.font_asset,
                glyph,
                x: i as u16 * 12 + (12 - u16::from(self.font.char_width(glyph))) / 2,
                y: 0,
                color: TextColor::new(14, 15, 1),
                double_rows: false,
            });
        }
        self.frame.main.windows.push(entry);
        let mut sprites = Vec::new();
        let mut push = |sequence, x, y, priority, elapsed| {
            sprites.push(Sprite {
                sequence,
                x,
                y,
                priority,
                elapsed,
                ..self.sprite.clone()
            });
        };
        let (col, row) = self.cursor;
        let (x, y, sequence) = if row == 0 {
            let button = usize::from(self.pages[self.page][0][col] - UPPER);
            (
                [25, 57, 89, 97, 122, 158, 198][button],
                68,
                if button < 4 { 40 } else { 41 },
            )
        } else {
            (col as i16 * 16 + 26, (row as i16 - 1) * 19 + 91, 39)
        };
        push(
            if self.cursor_press { 60 } else { sequence },
            x,
            y,
            0,
            self.cursor_elapsed,
        );
        let wiggle = self
            .wiggle
            .map_or(0, |i| [4, 4, -3, -3, 2, 2, 0][usize::from(i)]);
        for (i, x, sequence) in [(0, 4, 3), (1, 36, 8), (2, 68, 13)] {
            // The subsprite task adds the parent bar's X=22 to buttons.
            push(
                if self.page == i {
                    sequence - 3
                } else {
                    sequence
                },
                x + 22 + wiggle,
                68,
                1,
                self.elapsed,
            );
        }
        push(
            if self.button_age[0].is_some() { 24 } else { 23 },
            158 + wiggle,
            68,
            1,
            self.button_age[0].unwrap_or(self.elapsed),
        );
        push(
            if self.button_age[1].is_some() { 26 } else { 25 },
            198 + wiggle,
            68,
            1,
            self.button_age[1].unwrap_or(self.elapsed),
        );
        push(37, 22 + wiggle, 56, 2, self.elapsed);
        for i in 0..PLAYER_NAME_LEN {
            push(
                if i == self.entry.len() { 44 } else { 43 },
                80 + i as i16 * 12,
                39,
                1,
                self.elapsed,
            );
        }
        push(48 + usize::from(self.gender), 24, 8, 1, self.elapsed);
        self.frame.main.sprites = sprites;
    }
}

impl App for NamingScreen {
    fn tick(&mut self, _frame: crate::Frame, input: Input) {
        let pressed = input.keys.pressed(self.prev.keys);
        let touch_new = input.touch.is_some() && self.prev.touch.is_none();
        // ReadKeypad's shared repeat timer (SetKeyRepeatTimers(4,8)).
        let repeated = if input.keys != self.prev.keys {
            self.repeat_delay = 8;
            pressed
        } else if input.keys != Keys::IDLE {
            self.repeat_delay = self.repeat_delay.saturating_sub(1);
            if self.repeat_delay == 0 {
                self.repeat_delay = 4;
                input.keys
            } else {
                Keys::IDLE
            }
        } else {
            Keys::IDLE
        };
        self.prev = input;
        match self.state {
            NamingState::Init => {
                self.fade
                    .begin(FadeType::BrightnessIn, FadeColor::Black, 16, 1);
                self.state = NamingState::InitIcon;
            }
            NamingState::InitIcon => self.state = NamingState::FadeIn,
            NamingState::FadeIn if self.fade.is_finished() => {
                self.state = NamingState::FinishInit;
                self.delay = 0;
            }
            NamingState::FinishInit => {
                self.delay += 1;
                if self.delay > 5 {
                    self.state = NamingState::Input;
                }
            }
            NamingState::Input => {
                if self.accepts_input() {
                    self.input(input, pressed, repeated, touch_new);
                }
                if self.cursor_press && self.cursor_elapsed >= self.cursor_duration {
                    self.cursor_press = false;
                    if self.entry.len() == PLAYER_NAME_LEN {
                        self.cursor = (12, 0);
                    }
                }
                self.slide();
                self.glow_angle += 20;
                if self.glow_angle > 360 {
                    self.glow_angle = 0;
                }
                let val = (i32::from(self.glow_sin[usize::from(self.glow_angle % 360 / 20)]) * 10)
                    / 4096
                    + 15;
                self.frame.main.obj_palette_overrides = vec![(0x1d, 29 | ((val as u16) << 5))];
            }
            NamingState::FadeOutSub if self.fade.is_finished() => {
                self.fade.begin_with_screens(
                    FadeScreens::Main,
                    FadeType::BrightnessOut,
                    FadeColor::Black,
                    16,
                    1,
                );
                self.state = NamingState::FadeOutMain;
            }
            NamingState::FadeOutMain if self.fade.is_finished() => self.state = NamingState::Done,
            _ => {}
        }
        self.fade.update();
        match self.fade.screens() {
            FadeScreens::Both => {
                self.frame.main.brightness = self.fade.brightness();
                self.frame.sub.brightness = self.fade.brightness();
            }
            FadeScreens::Main => self.frame.main.brightness = self.fade.brightness(),
            FadeScreens::Sub => self.frame.sub.brightness = self.fade.brightness(),
        }
        self.compose();
        self.wiggle = self
            .wiggle
            .and_then(|i| if i < 6 { Some(i + 1) } else { None });
        for age in self.button_age.iter_mut().flatten() {
            *age += 1;
        }
        self.elapsed += 1;
        self.cursor_elapsed += 1;
    }
    fn frame(&self) -> &LogicalFrame {
        &self.frame
    }
    fn next(&self) -> ChainNext {
        if self.state == NamingState::Done {
            ChainNext::Advance
        } else {
            ChainNext::Stay
        }
    }
}
