//! The brightness fade — pret's palette-fade manager over the master
//! brightness registers, Phase 4, step 5.
//!
//! The manager lives in the main ARM9: `BeginNormalPaletteFade`
//! (`asm/unk_0200FA24.s:81`) builds **two** fade works — one per
//! screen — steps them once per frame from
//! `HandleFadeUpdateFrame` (`:198`, called after the vblank wait in
//! `NitroMain`'s loop, `src/main.c:95-130`), and clears its flag in
//! `HandleEndFade` (`:463`). Every begin in the boot flow but Oak's
//! passes `FADE_BOTH_SCREENS` with the same type and color for both
//! works, so one [`BrightnessFade`] with
//! [`FadeScreens::Both`] models the pair; Oak's speech is the one
//! caller of `FADE_SUB_ONLY` (`oaks_speech.c`'s states 61/63/70/100/
//! 101), where only the sub-screen work is built — the main engine's
//! register keeps whatever it last held. The owning scene applies
//! [`BrightnessFade::brightness`] to the fade's
//! [screens](Self::screens) each tick.
//!
//! The fade functions are `FadeFunc_00` (`FADE_TYPE_BRIGHTNESS_OUT`)
//! and `FadeFunc_01` (`FADE_TYPE_BRIGHTNESS_IN`)
//! (`asm/unk_0201010C.s:139`/`:156`, `include/screen_fade.h`):
//!
//! * **Init** (`sub_02010B14`, reached from the fade func's first
//!   call — `BeginNormalPaletteFade` runs that inline, so `begin`
//!   both sets up *and* writes the start brightness during the
//!   calling tick's exec): an IN fade starts at the extreme and ends
//!   at 0; an OUT fade starts at 0 and ends at the extreme (black
//!   −16, white +16). The init writes `SetMasterBrightness(start)`,
//!   and stores `current = start << 7`, `end = endVal << 7`, and
//!   `delta = ((raw_color − start) << 7) / steps` (`sub_02010A6C`,
//!   the SDK's `_s32_div_f` — truncation toward zero, exactly Rust's
//!   `i32` `/`). `raw_color` is the begin's color (0 black,
//!   `0x7FFF` white), so an OUT-black fade's delta is 0 — the game's
//!   own quirk: the screen only *snaps* dark at the last step — and
//!   an IN-white fade's delta (698688 for the standard 6-step fade)
//!   overshoots the range entirely, the register's 5-bit value
//!   wrapping through the intermediate steps (see the pinned
//!   sequences in the tests).
//! * **Step** (`sub_02010BF4`, one per `update` while the work is
//!   running): `counter++`; below `frames_per_step` the step writes
//!   nothing; otherwise `counter = 0`, `steps−= 1`, and if the
//!   count ran out `current = end` and the work reports exhausted,
//!   else `current += delta` — then one write of
//!   `SetMasterBrightness(current / 128)` (the `asr/lsr/asr` dance,
//!   signed division truncating toward zero).
//! * **The work states** (`sub_02010BB4`): 1 stepping, 2 → free +
//!   report idle, 3 inert. `HandleFadeUpdateFrame` steps the works
//!   while the flag is set; once both report idle it calls
//!   `HandleEndFade`, whose `strh 0` at manager+0x14C (aliased to
//!   the flag halfword at `_021D1034+0xc`, what
//!   `IsPaletteFadeFinished` (`:223`) reads) clears it.
//!
//! **Tick order**: the scene's exec runs *before* the frame's fade
//! step, so an `is_finished` poll during exec sees the previous
//! tick's update. For the standard 6-step, 1-frame fade this puts
//! the steps on the begin tick through `begin+5` (the begin tick
//! shows step 1: `begin` ran the init inline, the post-vblank step
//! lands first), the flag clears during `begin+6`'s update, and the
//! **first poll that reads true is `begin+7`** — the budget every
//! scene port in this step cites.
//!
//! `SetMasterBrightness` (`asm/unk_0200FA24.s:444`) picks the engine
//! register; `GXx_SetMasterBrightness_` (`lib/NitroSDK/asm/gx.s:240`)
//! writes it: 0 disables the effect, a positive value ORs in the Up
//! mode bit, a negative the Down mode bit, and the hardware's 5-bit
//! weight wraps (`& 0x1F`, the mask `GXx_GetMasterBrightness_` reads
//! back with). [`BrightnessFade::write`] models the direct
//! both-screen variant `sub_0200FBF4` (`:305`): black is a plain
//! `SetMasterBrightness(engine, -16)` on each screen.

use crate::frame::{BrightnessMode, MasterBrightness};

/// `enum FadeType` (`include/screen_fade.h`) — the two fade functions
/// the boot flow uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeType {
    /// `FADE_TYPE_BRIGHTNESS_OUT` (0): `FadeFunc_00` — start 0, end
    /// the extreme.
    BrightnessOut,
    /// `FADE_TYPE_BRIGHTNESS_IN` (1): `FadeFunc_01` — start the
    /// extreme, end 0.
    BrightnessIn,
}

/// The begin's `color` argument — the raw value the delta measures
/// from, and (for `sub_0200FBF4`) the direct-write color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeColor {
    /// `RGB_BLACK` (0).
    Black,
    /// `RGB_WHITE` (`0x7FFF`).
    White,
}

impl FadeColor {
    /// The raw color value (`sub_02010B14`'s `0x7FFF` test).
    fn raw(self) -> i32 {
        match self {
            Self::Black => 0,
            Self::White => 0x7FFF,
        }
    }
}

/// The begin's `screens` argument — which of the two fade works
/// `BeginNormalPaletteFade` builds (`FADE_BOTH_SCREENS` /
/// `FADE_MAIN_ONLY` / `FADE_SUB_ONLY`, `include/screen_fade.h`).
///
/// The step math is identical for either work; only which engine's
/// `SetMasterBrightness` the init and steps write differs. A fade
/// that skips a screen leaves that engine's register at its last
/// value — the frame model's per-engine brightness carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FadeScreens {
    /// `FADE_BOTH_SCREENS` (0): both works step together.
    #[default]
    Both,
    /// `FADE_MAIN_ONLY` (1): only the main engine fades.
    Main,
    /// `FADE_SUB_ONLY` (2): only the sub engine fades — Oak's
    /// speech is the boot flow's one user.
    Sub,
}

/// One master-brightness fade — the pair of fade works a
/// `FADE_BOTH_SCREENS` `BeginNormalPaletteFade` builds, stepped once
/// per frame by [`update`](Self::update).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrightnessFade {
    /// Which screens the fade drives (`BeginNormalPaletteFade`'s
    /// `screens` argument).
    screens: FadeScreens,
    /// The manager's flag (`_021D1034+0xc`): set by
    /// `BeginNormalPaletteFade`, cleared by `HandleEndFade`.
    flag: bool,
    /// The work's state (`sub_02010BB4`): 1 stepping, 2 free, 3 idle.
    work_state: u8,
    /// The remaining step count (`[0]` of the sub-work).
    steps: i32,
    /// The frames one step spans (`[4]`).
    frames_per_step: u8,
    /// The frame counter within the current step (`[8]`).
    counter: u8,
    /// The brightness accumulator, `<< 7` (`[0xc]`).
    current: i32,
    /// The end value, `<< 7` (`[0x10]`).
    end: i32,
    /// The per-step delta, `<< 7` (`[0x14]`).
    delta: i32,
    /// The last `SetMasterBrightness` argument written (the value the
    /// frame carries between writes).
    written: i32,
}

impl Default for BrightnessFade {
    fn default() -> Self {
        // Power-on: no fade, the effect disabled (`master_bright` 0).
        Self {
            screens: FadeScreens::Both,
            flag: false,
            work_state: 3,
            steps: 0,
            frames_per_step: 0,
            counter: 0,
            current: 0,
            end: 0,
            delta: 0,
            written: 0,
        }
    }
}

impl BrightnessFade {
    /// `BeginNormalPaletteFade(FADE_BOTH_SCREENS, type, type, color,
    /// steps, framesPerStep, heap)`: sets the flag, runs the fade
    /// func's init inline (the start brightness is written *now*),
    /// and arms `steps` × `frames_per_step` frames of stepping.
    ///
    /// Call [`update`](Self::update) once per tick after the scene's
    /// exec — the same tick's update is the begin tick's post-vblank
    /// step, the first of the sequence. Equivalent to
    /// [`begin_with_screens`](Self::begin_with_screens) with
    /// [`FadeScreens::Both`]; kept for the flow's every other caller.
    pub fn begin(&mut self, ty: FadeType, color: FadeColor, steps: i32, frames_per_step: u8) {
        self.begin_with_screens(FadeScreens::Both, ty, color, steps, frames_per_step);
    }

    /// `BeginNormalPaletteFade` with the screens picked: Oak's speech
    /// passes `FADE_SUB_ONLY` at `oaks_speech.c`'s states 61, 63, 70,
    /// 100, and 101, where only the sub engine's work exists.
    pub fn begin_with_screens(
        &mut self,
        screens: FadeScreens,
        ty: FadeType,
        color: FadeColor,
        steps: i32,
        frames_per_step: u8,
    ) {
        self.screens = screens;
        // sub_02010B14's start/end picks by direction and color.
        let (start, end) = match (ty, color) {
            (FadeType::BrightnessIn, FadeColor::Black) => (-16, 0),
            (FadeType::BrightnessIn, FadeColor::White) => (16, 0),
            (FadeType::BrightnessOut, FadeColor::Black) => (0, -16),
            (FadeType::BrightnessOut, FadeColor::White) => (0, 16),
        };
        // The flag is set before the inline init (BeginNormalPaletteFade
        // stores it, then runs FadeWork_UpdateFrame).
        self.flag = true;
        self.work_state = 1;
        self.steps = steps;
        self.frames_per_step = frames_per_step;
        self.counter = 0;
        self.current = start << 7;
        self.end = end << 7;
        // sub_02010A6C: ((raw_color − start) << 7) / steps, _s32_div_f.
        self.delta = ((color.raw() - start) << 7) / steps;
        // The init's SetMasterBrightness(engine, start).
        self.written = start;
    }

    /// A direct both-screen `SetMasterBrightness` — `sub_0200FBF4`'s
    /// black (each engine at −16). No fade is started; the written
    /// value stands until a fade or another write replaces it.
    pub fn write(&mut self, v: i32) {
        self.written = v;
    }

    /// `HandleFadeUpdateFrame` — the once-per-frame step, post-vblank
    /// (after the scene's exec). A no-op while the flag is clear.
    pub fn update(&mut self) {
        if !self.flag {
            return;
        }
        match self.work_state {
            1 => {
                // sub_02010BF4: the counter gate, then the step.
                self.counter = self.counter.wrapping_add(1);
                if self.counter < self.frames_per_step {
                    return; // blt _02010C32: no write this frame.
                }
                self.counter = 0;
                let next = self.steps - 1;
                if next <= 0 {
                    // Exhausted: snap to the end and write it.
                    self.current = self.end;
                    self.written = self.current / 128;
                    self.work_state = 2;
                } else {
                    self.steps = next;
                    self.current += self.delta;
                    self.written = self.current / 128;
                }
            }
            // sub_02010BB4's state 2 frees the work and reports idle;
            // the idle report is what lets HandleEndFade run — the
            // same frame clears the flag.
            2 => {
                self.work_state = 3;
                self.flag = false;
            }
            _ => {}
        }
    }

    /// `IsPaletteFadeFinished` — the flag, as the next exec sees it
    /// (the previous tick's update).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        !self.flag
    }

    /// Which screens this fade drives — the owning scene's tail
    /// applies [`brightness`](Self::brightness) to these engines and
    /// leaves the others' registers where they were.
    #[must_use]
    pub fn screens(&self) -> FadeScreens {
        self.screens
    }

    /// The last written brightness, through the register mapping of
    /// `GXx_SetMasterBrightness_`: 0 disables, positive fades up,
    /// negative down, the weight the hardware's 5 bits keep.
    #[must_use]
    pub fn brightness(&self) -> MasterBrightness {
        let v = self.written;
        if v == 0 {
            MasterBrightness {
                mode: BrightnessMode::Disabled,
                value: 0,
            }
        } else if v > 0 {
            MasterBrightness {
                mode: BrightnessMode::Up,
                value: (v & 0x1F) as u8,
            }
        } else {
            MasterBrightness {
                mode: BrightnessMode::Down,
                value: ((-v) & 0x1F) as u8,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down(value: u8) -> MasterBrightness {
        MasterBrightness {
            mode: BrightnessMode::Down,
            value,
        }
    }

    fn up(value: u8) -> MasterBrightness {
        MasterBrightness {
            mode: BrightnessMode::Up,
            value,
        }
    }

    fn off() -> MasterBrightness {
        MasterBrightness {
            mode: BrightnessMode::Disabled,
            value: 0,
        }
    }

    /// The manager's own poll timing, as the main loop's order makes
    /// it: the step runs after the exec, so a poll during tick `N`'s
    /// exec sees the update of tick `N−1`. For a 6×1 fade begun on
    /// tick B: B..B+5 show steps 1-6, the flag clears in B+6's
    /// update, the first true poll is B+7.
    #[test]
    fn in_black_six_by_one_matches_the_arm() {
        // FadeFunc_01's init: start −16, end 0,
        // delta = ((0 − (−16)) << 7) / 6 = 341.
        let mut fade = BrightnessFade::default();
        fade.begin(FadeType::BrightnessIn, FadeColor::Black, 6, 1);
        // The begin's inline init write (the pre-step screen).
        assert_eq!(fade.brightness(), down(16));
        // The six steps, recomputed from sub_02010BF4: current walks
        // −2048 + k·341, each written back / 128 trunc-toward-zero.
        let steps = [
            down(13), // (−2048 + 341) / 128 = −13.3 → −13
            down(10), // (−1707 + 341) / 128 = −10.6 → −10
            down(8),  // −1366 → −8.007 → −8
            down(5),  // −1025 → −5.3 → −5
            down(2),  // −684 → −2.68 → −2
            off(),    // exhausted: current = end (0)
        ];
        for (i, expected) in steps.iter().enumerate() {
            fade.update();
            assert_eq!(fade.brightness(), *expected, "step {}", i + 1);
            assert!(!fade.is_finished(), "the flag holds through step 6");
        }
        // The free tick: HandleEndFade clears the flag.
        fade.update();
        assert!(fade.is_finished());
    }

    #[test]
    fn out_black_six_by_one_matches_the_arm() {
        // FadeFunc_00's init: start 0, end −16, and the OUT-black
        // quirk — delta = ((0 − 0) << 7) / 6 = 0, so every step
        // before the last writes 0 and the screen only snaps dark at
        // the exhaustion snap.
        let mut fade = BrightnessFade::default();
        fade.begin(FadeType::BrightnessOut, FadeColor::Black, 6, 1);
        assert_eq!(fade.brightness(), off());
        for _ in 0..5 {
            fade.update();
            assert_eq!(fade.brightness(), off());
        }
        fade.update();
        assert_eq!(fade.brightness(), down(16), "the end snap");
        fade.update();
        assert!(fade.is_finished());
    }

    #[test]
    fn out_white_six_by_one_matches_the_arm() {
        // Start 0, end +16, delta = (0x7FFF << 7) / 6 = 699029: the
        // accumulator runs far past the register's range, and the
        // 5-bit weight wraps — 5461 & 0x1F = 21, 10922 & 0x1F = 10,
        // 16383 & 0x1F = 31, 21844 & 0x1F = 20, 27305 & 0x1F = 9,
        // then the end snap to exactly 16.
        let mut fade = BrightnessFade::default();
        fade.begin(FadeType::BrightnessOut, FadeColor::White, 6, 1);
        assert_eq!(fade.brightness(), off());
        let steps = [up(21), up(10), up(31), up(20), up(9), up(16)];
        for expected in steps {
            fade.update();
            assert_eq!(fade.brightness(), expected);
        }
        fade.update();
        assert!(fade.is_finished());
    }

    #[test]
    fn in_white_six_by_one_matches_the_arm() {
        // Start +16, end 0, delta = ((0x7FFF − 16) << 7) / 6 =
        // 698688 — the same register wrap from the other side, and
        // the exhaustion snap restores the exact 0.
        let mut fade = BrightnessFade::default();
        fade.begin(FadeType::BrightnessIn, FadeColor::White, 6, 1);
        assert_eq!(fade.brightness(), up(16));
        let steps = [up(2), up(21), up(7), up(26), up(12), off()];
        for expected in steps {
            fade.update();
            assert_eq!(fade.brightness(), expected);
        }
        fade.update();
        assert!(fade.is_finished());
    }

    #[test]
    fn a_faster_frame_rate_gates_the_steps() {
        // framesPerStep 3: the first two updates count the counter
        // and write nothing (sub_02010BF4's blt), the third is the
        // step. Two steps of an IN-black fade: −13 lands on the
        // third update.
        let mut fade = BrightnessFade::default();
        fade.begin(FadeType::BrightnessIn, FadeColor::Black, 6, 3);
        assert_eq!(fade.brightness(), down(16));
        fade.update();
        fade.update();
        assert_eq!(fade.brightness(), down(16), "the counter gate holds");
        fade.update();
        assert_eq!(fade.brightness(), down(13));
    }

    #[test]
    fn sub_0200fbf4_writes_both_dark_without_a_fade() {
        // The direct write the scenes use at construction: −16 on
        // each engine, the fade still finished.
        let mut fade = BrightnessFade::default();
        assert!(fade.is_finished());
        fade.write(-16);
        assert_eq!(fade.brightness(), down(16));
        assert!(fade.is_finished(), "no fade was started");
        // An inert update never touches the written value.
        fade.update();
        assert_eq!(fade.brightness(), down(16));
        // A later begin takes over the register, as the hardware
        // would: the init write wins.
        fade.begin(FadeType::BrightnessOut, FadeColor::Black, 6, 1);
        assert_eq!(fade.brightness(), off());
    }

    #[test]
    fn sub_only_fades_target_the_sub_engine() {
        // BeginNormalPaletteFade's screens argument picks the works
        // built; the default begin stays both, Oak's FADE_SUB_ONLY
        // fades step only the sub engine's register.
        let mut fade = BrightnessFade::default();
        fade.begin(FadeType::BrightnessOut, FadeColor::Black, 6, 1);
        assert_eq!(fade.screens(), FadeScreens::Both);
        fade.begin_with_screens(
            FadeScreens::Sub,
            FadeType::BrightnessIn,
            FadeColor::Black,
            6,
            1,
        );
        assert_eq!(fade.screens(), FadeScreens::Sub);
        // The step math is the shared fade func's — unchanged.
        fade.update();
        assert_eq!(fade.brightness(), down(13));
    }
}