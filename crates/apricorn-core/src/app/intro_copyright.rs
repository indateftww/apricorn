//! The intro movie's copyright beat — pret
//! `src/intro_movie_scene_1.c` through `INTRO_SCENE1_WAIT_GAMEFREAK`.
//!
//! Port of scene 1's first five steps, the Phase 3 scope agreed in the
//! plan (the full 5-scene movie defers to the phases where its
//! subsystems exist):
//!
//! 1. `APPEAR_COPYRIGHT` — MAIN BG0 (the copyright text, scrolled
//!    down 128px) and SUB BG0 (the blank cover over the Game Freak
//!    logo) on.
//! 2. `WAIT_COPYRIGHT` — a 30-frame hold.
//! 3. `WAIT_FADEOUT_COPYRIGHT` — a 60-frame alpha fade of both
//!    engines' BG0 toward the backdrop:
//!    `Task_IntroMovie_BlendFadeEffect` runs
//!    `ev = counter * 31 / duration` per tick and applies
//!    `EVA = 31 - ev`, `EBV = ev` (direction 1); the fade finishes
//!    when `ev` reaches 31.
//! 4. `WAIT_APPEAR_GAMEFREAK` — 20 frames later the Game Freak logo:
//!    SUB BG0 off, SUB BG1 on, MAIN BG0's scroll reset to 0, MAIN's
//!    blend reset to `G2_SetBlendAlpha(1, 0x20, 0x1F, 0)`, and the
//!    intro's skip becomes allowed.
//! 5. `WAIT_GAMEFREAK` — a 110-frame hold, after which the beat
//!    hands the boot chain to the title screen.
//!
//! The skip (pret `IntroMovie_Main`): A, START, or a new stylus
//! contact — checked *before* the step logic each tick, and only once
//! the Game Freak step allowed it — finishes the beat on that tick.
//!
//! The frame at construction is the cleared state pret leaves after
//! `IntroMovie_Scene1_Init`: every layer of both engines configured
//! from its `BgTemplate` with its assets loaded, all planes off, both
//! blend units neutral, black backdrops, and `screensFlipped`
//! (`DisplaySelect::SubOnTop`).
//!
//! Deferrals, each honest in the frame: the sunrise layers (MAIN
//! BG1–BG3, SUB BG2/BG3) load and configure but never show — they
//! first appear at `APPEAR_BG_IMAGE`, which is past this beat's scope;
//! the sun/bird OBJ sprites, the skip's white palette fade, and the
//! scenes 2–5 that follow are out of Phase 3 entirely.

use std::sync::Mutex;

use crate::app::App;
use crate::assets::{AssetStore, AssetsError, copyright_beat};
use crate::frame::{
    BgLayer, Blend, BlendEffect, ColorMode, DisplaySelect, LogicalFrame, PaletteLoad, ScreenSize,
    TilePlacement, plane,
};
use crate::input::{Input, Keys, key};

/// The scene's steps, pret's `IntroScene1State` through
/// `INTRO_SCENE1_WAIT_GAMEFREAK` (the rest of the enum is past the
/// Phase 3 scope).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Set MAIN BG0's scroll to 128 and show both BG0 planes.
    AppearCopyright,
    /// Hold the copyright 30 frames, then start the fade.
    WaitCopyright,
    /// Run the 60-frame blend fade to completion.
    WaitFadeout,
    /// Wait 20 frames, then show the Game Freak logo.
    WaitGamefreakAppear,
    /// Hold the logo 110 frames, then finish the beat.
    WaitGamefreak,
}

/// The copyright beat app.
pub struct IntroCopyright {
    /// The logical frame the last tick produced.
    frame: LogicalFrame,
    /// The current step.
    step: Step,
    /// The per-step timer (pret's `sceneTimer`): reset to 0 by a step
    /// advance, incremented once per tick — so it counts the ticks
    /// since entering the step, starting at 1.
    timer: u16,
    /// The blend fade's frame counter (`IntroMovieBgBlendAnim::counter`).
    fade_counter: u16,
    /// Whether the fade task is running.
    fade_active: bool,
    /// Whether the fade reached `ev` 31 (`IntroMovieBgBlendAnim::finished`).
    fade_finished: bool,
    /// Whether the skip is allowed (pret's `skipAllowed`).
    skip_allowed: bool,
    /// The keys held on the previous tick, for press edges.
    prev_keys: Keys,
    /// Whether the stylus was down on the previous tick.
    prev_touch: bool,
    /// Whether the beat has finished (skip or the 110-frame hold).
    done: bool,
}

impl IntroCopyright {
    /// Loads the scene's members and builds the cleared initial frame.
    ///
    /// The load order is pret's `IntroMovie_Scene1_LoadBgGfx` (the
    /// sunrise members included, per the asset table); the layer
    /// geometry is `IntroMovie_Scene1_InitBgs`' templates, with
    /// `GX_BG_CHARBASE_0x…` naming the char-block slots.
    ///
    /// # Errors
    /// Returns the store's [`AssetsError`] when a table member is
    /// missing or corrupt — unreachable in practice, since the store
    /// only opens the SHA-1-pinned dump the table is valid for.
    pub fn load(store: &Mutex<AssetStore>) -> Result<Self, AssetsError> {
        let mut store = store
            .lock()
            .expect("the asset store is only locked at app construction");
        let narc = copyright_beat::NARC;
        // LoadBgGfx's order: the shown layers first, then the sunrise
        // layers the later (out-of-scope) steps would reveal.
        let sub_bg1_char = store.load_tiles(narc, copyright_beat::SUB_BG1_CHAR)?;
        let main_bg0_char = store.load_tiles(narc, copyright_beat::MAIN_BG0_CHAR)?;
        let sub_bg1_screen = store.load_screen(narc, copyright_beat::SUB_BG1_SCREEN)?;
        let main_bg0_screen = store.load_screen(narc, copyright_beat::MAIN_BG0_SCREEN)?;
        let sub_bg0_screen = store.load_screen(narc, copyright_beat::SUB_BG0_SCREEN)?;
        let sub_bg3_char = store.load_tiles(narc, copyright_beat::SUB_BG3_CHAR)?;
        let main_bg3_char = store.load_tiles(narc, copyright_beat::MAIN_BG3_CHAR)?;
        let sub_bg3_screen = store.load_screen(narc, copyright_beat::SUB_BG3_SCREEN)?;
        let main_bg3_screen = store.load_screen(narc, copyright_beat::MAIN_BG3_SCREEN)?;
        let main_bg2_screen = store.load_screen(narc, copyright_beat::MAIN_BG2_SCREEN)?;
        let main_bg1_screen = store.load_screen(narc, copyright_beat::MAIN_BG1_SCREEN)?;
        let sub_palette = store.load_palette(narc, copyright_beat::SUB_PALETTE)?;
        let main_palette = store.load_palette(narc, copyright_beat::MAIN_PALETTE)?;

        // Both engines: BG mode 0, all layers 256×256 4bpp (the
        // templates). MAIN BG0 owns char slot 1, the sunrise layers
        // share slot 2; SUB BG0/BG1 share slot 1 (the logo cover reads
        // the logo's tiles), SUB BG3 owns slot 4.
        let layer = |char_base: u8, screen, priority: u8| BgLayer {
            hidden_rect: None,
            enabled: false,
            char_base,
            screen,
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH256,
            scroll_x: 0,
            scroll_y: 0,
            priority,
        };

        let mut frame = LogicalFrame {
            display: DisplaySelect::SubOnTop,
            ..LogicalFrame::default()
        };
        // GXLoadPal's 0x140-byte loads at slot offset 0: the first
        // 160 colors of each engine's palette RAM.
        frame.main.palette_loads.push(PaletteLoad {
            asset: main_palette,
            offset: 0,
            colors: 0x140 / 2,
        });
        frame.sub.palette_loads.push(PaletteLoad {
            asset: sub_palette,
            offset: 0,
            colors: 0x140 / 2,
        });
        frame.main.char_blocks[1].push(TilePlacement {
            asset: main_bg0_char,
            tile: 0,
        });
        frame.main.char_blocks[2].push(TilePlacement {
            asset: main_bg3_char,
            tile: 0,
        });
        frame.main.bgs = [
            layer(1, Some(main_bg0_screen), 0),
            layer(2, Some(main_bg1_screen), 1),
            layer(2, Some(main_bg2_screen), 2),
            layer(2, Some(main_bg3_screen), 3),
        ];
        frame.sub.char_blocks[1].push(TilePlacement {
            asset: sub_bg1_char,
            tile: 0,
        });
        frame.sub.char_blocks[4].push(TilePlacement {
            asset: sub_bg3_char,
            tile: 0,
        });
        frame.sub.bgs = [
            layer(1, Some(sub_bg0_screen), 0),
            layer(1, Some(sub_bg1_screen), 1),
            // SUB BG2's template names slot 3, but nothing ever loads
            // there and no screen arrives — the cleared tilemap.
            layer(3, None, 2),
            layer(4, Some(sub_bg3_screen), 3),
        ];

        Ok(Self {
            frame,
            step: Step::AppearCopyright,
            timer: 0,
            fade_counter: 0,
            fade_active: false,
            fade_finished: false,
            skip_allowed: false,
            prev_keys: Keys::IDLE,
            prev_touch: false,
            done: false,
        })
    }
}

impl App for IntroCopyright {
    fn tick(&mut self, _frame: crate::Frame, input: Input) {
        let new_keys = input.keys.pressed(self.prev_keys);
        self.prev_keys = input.keys;
        let touch_new = input.touch.is_some() && !self.prev_touch;
        self.prev_touch = input.touch.is_some();

        // pret checks the skip at the top of IntroMovie_Main, before
        // the scene's state machine, and leaves the frame untouched.
        if self.skip_allowed && (new_keys.any(key::A | key::START) || touch_new) {
            self.done = true;
            return;
        }

        let mut advanced = false;
        match self.step {
            Step::AppearCopyright => {
                // BgSetPosTextAndCommit(MAIN_0, SET_Y, 128) and both
                // engines' BG0 planes on.
                self.frame.main.bgs[0].scroll_y = 128;
                self.frame.main.bgs[0].enabled = true;
                self.frame.sub.bgs[0].enabled = true;
                self.step = Step::WaitCopyright;
                advanced = true;
            }
            Step::WaitCopyright => {
                if self.timer >= 30 {
                    // StartBlendFadeEffect on both engines' blend units
                    // (blend[0] → MAIN, blend[1] → SUB): plane1 BG0,
                    // plane2 BD, 60 frames, direction 1.
                    self.fade_active = true;
                    self.fade_counter = 0;
                    self.step = Step::WaitFadeout;
                    advanced = true;
                }
            }
            Step::WaitFadeout => {
                if self.fade_finished {
                    self.step = Step::WaitGamefreakAppear;
                    advanced = true;
                }
            }
            Step::WaitGamefreakAppear => {
                if self.timer >= 20 {
                    self.skip_allowed = true;
                    self.frame.sub.bgs[0].enabled = false;
                    self.frame.sub.bgs[1].enabled = true;
                    self.frame.main.bgs[0].scroll_y = 0;
                    // G2_SetBlendAlpha(1, 0x20, 0x1F, 0): MAIN's blend
                    // back to showing BG0 whole (the fade left EVA 0).
                    self.frame.main.blend = Blend {
                        plane1: plane::BG0,
                        effect: BlendEffect::Alpha,
                        plane2: plane::BD,
                        eva: 31,
                        ebv: 0,
                        evy: 0,
                    };
                    self.step = Step::WaitGamefreak;
                    advanced = true;
                }
            }
            Step::WaitGamefreak => {
                // pret's scene 1 continues into the sunrise here; the
                // Phase 3 beat ends at the same 110-frame hold.
                if self.timer >= 110 {
                    self.done = true;
                }
            }
        }

        // Task_IntroMovie_BlendFadeEffect: one counter step per tick
        // while the fade runs — ev = counter * 31 / 60, applied as
        // EVA = 31 - ev, EBV = ev on both engines.
        if self.fade_active && !self.fade_finished {
            self.fade_counter += 1;
            let ev = (self.fade_counter * 31 / 60) as u8;
            let finished = ev >= 31;
            let ev = ev.min(31);
            let fade = Blend {
                plane1: plane::BG0,
                effect: BlendEffect::Alpha,
                plane2: plane::BD,
                eva: 31 - ev,
                ebv: ev,
                evy: 0,
            };
            self.frame.main.blend = fade;
            self.frame.sub.blend = fade;
            self.fade_finished = finished;
        }

        // IntroMovie_AdvanceSceneStep resets the timer; Main's
        // ++sceneTimer then runs on every call, so the first tick of a
        // step sees 1.
        self.timer = if advanced { 1 } else { self.timer + 1 };
    }

    fn frame(&self) -> &LogicalFrame {
        &self.frame
    }

    fn next(&self) -> crate::app::ChainNext {
        if self.done {
            crate::app::ChainNext::Advance
        } else {
            crate::app::ChainNext::Stay
        }
    }
}
