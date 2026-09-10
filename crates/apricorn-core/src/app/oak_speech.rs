//! Oak's intro speech — pret `src/oaks_speech.c` (+ its yes/no menu
//! `oaks_speech_yesnomenu.c` and the brightness transitions of
//! `brightness.c`), the overlay between the new-game boot and the
//! naming screen, Phase 4, step 5.
//!
//! `OakSpeech_Main` is an overlay state machine *around* a second,
//! bigger machine (`OakSpeech_DoMainTask`, 68 states): `pState` 0
//! reinitializes the whole scene (the naming screen returns here when
//! it finishes), `pState` 1 runs the touch-advance button then one
//! `DoMainTask` pass per tick, and the exits — the speech done, or
//! the naming overlay launched — run the same 6×1 fade-out and the
//! same cleanup, one branch ending the app (registering
//! `ov36_App_InitGameState_AfterOakSpeech`, `oaks_speech.c:650`) and
//! the other dropping into the naming screen's overlay run and back
//! to `pState` 0.
//!
//! The per-frame order (`src/main.c`'s loop, one [`App::tick`] per
//! frame):
//!
//! * **Prologue** — the keypad read: `PAD_Read() |
//!   gSystem.simulatedInputs` (`system.c:245`). The touch-advance
//!   button sets `simulatedInputs` during one tick's exec; the next
//!   tick's read ORs it in as an A press, feeding `DoMainTask` *and*
//!   the text printers (they read `gSystem.newKeys`).
//! * **Exec** — the `pState` machine, the touch-advance handler
//!   first (`HandleTouchToAdvanceButton`, `oaks_speech.c:2224`).
//! * **Tail** — the post-vblank services in order:
//!   `DoAllScreenBrightnessTransitionStep` (the MAIN static, then
//!   the SUB one, `brightness.c:110`), `HandleFadeUpdateFrame` (the
//!   palette fade, applied only to the fade's screens — Oak is the
//!   boot flow's one `FADE_SUB_ONLY` user), the BGM fade timer
//!   (`DoSoundUpdateFrame`, `sound.c:100`), and the frame-model
//!   rebuild of the window lists from the scene's windows.
//!
//! The gender portraits, touch button and Marill now carry their ROM
//! NCGR/NCLR/NCER/NANR resources into the OBJ renderer. Remaining
//! deferrals: sprite-completion waits (the existing immediate-poll
//! timeline) and the yes/no cursor;
//! audio —
//!   the BGM/SE/cry calls, with the *timer* of
//!   `GF_SndStartFadeOutBGM(0, 6)` modeled, since state 47 polls
//!   it; `SetKeyRepeatTimers(4, 8)` (no key-repeat model); the heap;
//!   `OakSpeech_Exit`'s save writes (the naming screen's port owns
//!   them); the rival naming screen (the C never launches it —
//!   the rival name stays the empty string the message format
//!   buffers); `lastInputWasTouch` and the multichoice `unk_0`
//!   (write-only); `DrawPicOnBgLayer`'s layer-2 branch (callers
//!   always pass `OAK_SPEECH_PIC_NONE`); both
//!   `BG_ClearCharDataRange` calls (no-ops — the range is cleared
//!   already); `sBgPicNCGR_NCLR` rows 3–5 and 7–9 (Ethan 2–4 and
//!   Lyra 2–4, never drawn); the deadstripped tables
//!   `ov53_021E84F8`/`ov53_021E84FC`.

use std::sync::Mutex;

use crate::app::fade::{BrightnessFade, FadeColor, FadeScreens, FadeType};
use crate::app::text::{AUTO_SCROLL_OFF, TEXT_SPEED_INSTANT, TextFlags, TextPrinter, font_color};
use crate::app::{App, ChainNext};
use crate::assets::{
    AssetStore, AssetsError, font_narc, frame_narc, intro_narc, msg_narc, yesno_narc,
};
use crate::font::Font;
use crate::frame::{
    AssetId, BgLayer, Blend, BlendEffect, ColorMode, DisplaySelect, EngineFrame, LogicalFrame,
    PaletteLoad, ScreenSize, TextColor, TilePlacement, TilemapEdit,
    Window, WindowFrame, Sprite, plane,
};
use crate::input::{Input, Keys, Touch, key};
use crate::rtc::RtcDateTime;
use crate::text::format::MessageFormat;
use crate::text::string::GameString;

// (GF_SinDeg(degrees) * 8) >> 12 for even degrees 0..358,
// from NitroSDK's FX_SinCosTable_ and FX_DEG_TO_IDX. Includes the
// table-index truncation and negative arithmetic shift. The u16 angle
// wraps after long holds, so all even degrees (not just tens) are needed.
const GENDER_BLINK_BRIGHTNESS: [i8; 180] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 8, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    6, 6, 6, 6, 6, 6, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 3, 3, 3, 3,
    2, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0, -1, -1, -1, -2, -2, -2, -2, -3, -3,
    -3, -3, -4, -4, -4, -4, -5, -5, -5, -5, -6, -6, -6, -6, -6, -7, -7, -7, -7, -7,
    -7, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8,
    -8, -8, -8, -8, -8, -8, -8, -8, -8, -8, -7, -7, -7, -7, -7, -7, -6, -6, -6, -6,
    -6, -5, -5, -5, -5, -5, -4, -4, -4, -4, -3, -3, -3, -2, -2, -2, -2, -1, -1, -1,
];

/// `NARC_msg_msg_0219_bin` — the speech bank (`msg_0219_00000`
/// through `msg_0219_00062`, the C's highest id).
const MSG_BANK: usize = 219;
/// The speech bank's 63 messages, cloned at load.
const MSG_COUNT: usize = 63;

/// The dialog print's frame delay —
/// `Options_GetTextFrameDelay(options)` at a fresh save's default
/// text speed 1 (`options.c:53-65`): 4 frames per character.
const TEXT_FRAME_DELAY: u32 = 4;

/// `OakSpeech_InitBgs`' per-layer char bases — MAIN_0..3 and
/// SUB_0..3 take `0x18000`, `0x14000`, `0x10000`, `0x0c000`
/// (`oaks_speech.c:687-704`, `:715-732`): slots 6, 5, 4, 3 —
/// `6 − layer` — both engines' templates.
const fn char_base(layer: u8) -> u8 {
    6 - layer
}

/// `sBgTemplate_Main`'s priority (`oaks_speech.c:361`).
const BG_PRIORITY_MAIN: u8 = 1;
/// `sBgTemplate_Sub`'s priority (`oaks_speech.c:376`).
const BG_PRIORITY_SUB: u8 = 0;
/// `SetBgPriority(GF_BG_LYR_SUB_3, 3)` (`oaks_speech.c:735`).
const BG_PRIORITY_SUB_3: u8 = 3;

/// `LoadUserFrameGfx2(…, 0x3E2, 4, 0)` — frame id 0's tiles through
/// the Gfx2 path, into MAIN_0's char block at `0x3E2`, its palette
/// (member `frame + 0x1A`) into bank 4.
const GFX2_TILE: u16 = 0x3E2;
/// `LoadUserFrameGfx1(…, 0x3D9, 3, 0)` — FRAME0's tiles at `0x3D9`,
/// its NCLR into bank 3.
const GFX1_TILE: u16 = 0x3D9;
/// The frame banks' palette-RAM offsets, in colors (bank n = n·16).
const GFX2_BANK: u16 = 4 * 16;
const GFX1_BANK: u16 = 3 * 16;
/// `LoadFontPal0(MAIN_BG, 0xA0)` — the font NCLR into slot offset
/// `0xA0` bytes = 80 colors; `LoadFontPal1(MAIN_BG, 0xC0)` = 96;
/// `LoadFontPal0(SUB_BG, 0x1C0)` = 224 (`oaks_speech.c:709-710`,
/// `:736`).
const FONT0_MAIN_OFFSET: u16 = 0xA0 / 2;
const FONT1_MAIN_OFFSET: u16 = 0xC0 / 2;
const FONT0_SUB_OFFSET: u16 = 0x1C0 / 2;
/// One 16-color palette load — one bank.
const BANK_COLORS: u16 = 16;

/// `LoadButtonTutorialGfx`'s palette loads (`oaks_speech.c:1172-1173`):
/// MAIN NCLR member 1 `@MAIN 0` for `0x60` bytes — 48 colors; SUB
/// NCLR member 30 `@SUB 0` for `0xA0` bytes — 80 colors (HeartGold
/// arm; the port pins HeartGold everywhere).
const TUTORIAL_MAIN_PAL_COLORS: u16 = 0x60 / 2;
const TUTORIAL_SUB_PAL_COLORS: u16 = 0xA0 / 2;

/// `DrawPicOnBgLayer`'s palette load (`oaks_speech.c:1201`): NCLR
/// `@MAIN 0xE0` — 112 colors — for 32 bytes: one 16-color bank.
const PIC_PALETTE_OFFSET: u16 = 0xE0 / 2;
const PIC_PALETTE_COLORS: u16 = 32 / 2;
/// `ov53_021E6824`'s palette load (`oaks_speech.c:1234`): NCLR
/// member 33 `@SUB 0xE0` — 112 colors — for `0x60` bytes: 48 colors.
const SUB2_PALETTE_OFFSET: u16 = 0xE0 / 2;
const SUB2_PALETTE_COLORS: u16 = 0x60 / 2;
/// `ov53_021E6824`'s male-arm scroll (`oaks_speech.c:1238-1246`):
/// SUB_0, SUB_2, and SUB_1 all scroll to `0x88`; the female arm
/// scrolls them to 0.
const GENDER_SCROLL_X: u16 = 0x88;

/// `OakSpeechYesNo_SetBackgroundPalette`'s load
/// (`oaks_speech_yesnomenu.c:62`): NCLR `@SUB 32·palette` bytes —
/// 16·palette colors — for `0x20` bytes.
const fn yesno_palette_offset(palette: u8) -> u16 {
    palette as u16 * 16
}

/// `ov53_021E8518` — the touch-advance message window, SUB_0
/// (`oaks_speech.c:183-191`): left 24, top 20, 7×2, palette 14,
/// baseTile `0x0A3`.
const TOUCH_TEMPLATE: (u8, u8, u8, u8, u8, u16) = (24, 20, 7, 2, 14, 0x0A3);
/// `sWindowTemplate_DialogMsg` (`oaks_speech.c:193-201`): MAIN_0,
/// left 2, top 19, 27×4, palette 6, baseTile `0x36D`.
const DIALOG_TEMPLATE: (u8, u8, u8, u8, u8, u16) = (2, 19, 27, 4, 6, 0x36D);
/// The fullscreen message window — MAIN_0, left 4, top 0, 24×24,
/// palette 5, baseTile `0x12D`. `copy1` (kind 1) and `copy2` (kinds
/// 0/2/3) are byte-identical (`oaks_speech.c:203-211`, `:234-242`);
/// kind 3 shifts the left edge +4 tiles at print
/// (`oaks_speech.c:1016-1017`).
const FULLSCREEN_TEMPLATE: (u8, u8, u8, u8, u8, u16) = (4, 0, 24, 24, 5, 0x12D);
/// `sMultichoiceMenuButtonWindowTemplates` (`oaks_speech.c:1067-1117`),
/// indexed `[numChoices − 2][choice]`: (left, top, width, height,
/// palette 14, baseTile) on SUB_0.
const MULTICHOICE_TEMPLATES: [&[(u8, u8, u8, u8, u8, u16)]; 2] = [
    &[(2, 6, 13, 3, 14, 0x001), (2, 16, 13, 3, 14, 0x036)],
    &[
        (7, 3, 18, 3, 14, 0x001),
        (7, 10, 18, 3, 14, 0x037),
        (7, 17, 18, 3, 14, 0x06D),
    ],
];
/// `OakSpeechYesNo_CreateWindows` (`oaks_speech_yesnomenu.c:108-109`):
/// SUB_0, palette 14 — YES at left 4, top 8, 24×2, baseTile `0x50`;
/// NO at left 4, top 14, 24×2, baseTile `0x80`.
const YESNO_TEMPLATES: [(u8, u8, u8, u16); 2] = [(8, 24, 2, 0x50), (14, 24, 2, 0x80)];

/// `ov53_021E8510` (`oaks_speech.c:218-232`) — the touch-advance
/// button's single held rect, pret order (top, bottom, left, right).
const TOUCH_ADVANCE_RECT: (u16, u16, u16, u16) = (144, 191, 168, 255);
/// `ov53_021E8650` (`oaks_speech.c:397-475`) — the multichoice menus'
/// touch rects, per menu id, pret order (top, bottom, left, right):
/// menu 0 the three tutorial buttons, menus 1–2 the two-choice
/// lists.
const MULTICHOICE_HITBOXES: [&[(u16, u16, u16, u16)]; 3] = [
    &[(20, 50, 50, 213), (76, 106, 50, 213), (132, 162, 50, 213)],
    &[(26, 83, 138, 253), (108, 164, 138, 253)],
    &[(26, 83, 10, 125), (108, 164, 10, 125)],
];
/// `sTouchscreenHitboxes_GenderSelect` (`oaks_speech.c:270-292`).
const GENDER_HITBOXES: [(u16, u16, u16, u16); 2] = [(25, 173, 18, 111), (25, 173, 144, 239)];
/// The yes/no menu's `sHitboxes` (`oaks_speech_yesnomenu.c:157-179`).
const YESNO_HITBOXES: [(u16, u16, u16, u16); 2] = [(50, 92, 3, 251), (99, 140, 3, 251)];

/// `sMultichoiceMenuBgCursorCoords` (`oaks_speech.c:381-395`) — the
/// SUB_1 y scroll per menu and cursor (rows 1–2's third entry is C
/// zero-fill, never read).
const MULTICHOICE_CURSOR_Y: [&[u16]; 3] = [&[0, 0x1C7, 0x18F], &[0, 0x1AF], &[0, 0x1AF]];
/// `sMultichoiceMenuParam[menuId][1]` (`oaks_speech.c:477-494`) — the
/// menus' option counts (member [0] is the screen, in
/// [`intro_narc::MULTICHOICE_SCREENS`]).
const MULTICHOICE_OPTIONS: [u8; 4] = [3, 2, 2, 2];

/// `sOakPicTranslationParam` (`oaks_speech.c:257-268`) — per slide
/// direction: (start, target, step).
const OAK_TRANSLATION_PARAM: [[i16; 3]; 2] = [[0, -52, -2], [-52, 0, 2]];

/// `OakSpeech_BlendLayer`'s constant second target
/// (`oaks_speech.c:838`): BG1 | BG2 | BG3.
const BLEND_SECOND_TARGET: u8 = plane::BG1 | plane::BG2 | plane::BG3;
/// State 47's transition mask (`oaks_speech.c:1808`): BG1 | BG3 |
/// OBJ on the MAIN engine.
const OAK_FADE_MASK: u8 = plane::BG1 | plane::BG3 | plane::OBJ;
/// State 52's flash masks (`oaks_speech.c:1837-1838`): BG0 | BG1 |
/// BG3 | OBJ on MAIN, BG0 | BG2 | BG3 | OBJ on SUB.
const BALL_FLASH_MAIN_MASK: u8 = plane::BG0 | plane::BG1 | plane::BG3 | plane::OBJ;
const BALL_FLASH_SUB_MASK: u8 = plane::BG0 | plane::BG2 | plane::BG3 | plane::OBJ;

/// `enum OakSpeechPic` (`oaks_speech.c:36-41`) — the Diamond/Pearl
/// animated-pic vestige: only the first frame of each row is ever
/// drawn (`DrawPicOnBgLayer` loads `sBgPicNCGR_NCLR[pic][0]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Pic {
    /// `OAK_SPEECH_PIC_NONE`.
    None = 0,
    /// `OAK_SPEECH_PIC_OAK`.
    Oak = 1,
    /// `OAK_SPEECH_PIC_ETHAN`.
    Ethan = 2,
    /// `OAK_SPEECH_PIC_LYRA`.
    Lyra = 6,
}

/// `enum OakSpeechMainState` (`oaks_speech.c:43-129`) — the
/// `DoMainTask` chain. The values are pret's (gaps and all — 73–92,
/// 104–109, 112–119, and 122 don't exist); 121 falls through into
/// 123 (`oaks_speech.c:2125`), so it lasts exactly one tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum MainState {
    /// 0: SUB_2 off, the tutorial BGM — the music is deferred.
    StartTutorialMusic = 0,
    /// 1: poll the 6×1 fade, then a 40-frame hold.
    WaitFadeInTutorialMenu = 1,
    /// 2: the tutorial-menu fullscreen text (kind 2).
    PrintTutorialMenuMessages = 2,
    /// 3: the tutorial multichoice.
    TutorialMenuHandleInput = 3,
    /// 4: the fullscreen text's fade back out, then the cleanup.
    FadeOutTutorialMenu = 4,
    /// 5: the 6×1 fade-out.
    FadeOutTutorialMenuBgs = 5,
    /// 6: poll the fade, then route by the chosen tutorial.
    WaitFadeOutTutorialMenuBgs = 6,
    /// 7: the shared tutorial-menu setup + the 6×1 fade-in.
    FadeInTutorialMenu = 7,
    /// 8: the control-info setup + fade-in.
    FadeInControlInfo = 8,
    /// 9: poll the fade.
    WaitFadeInControlInfo = 9,
    /// 11–21: the control-info message chain.
    ControlInfo1 = 11,
    ControlInfo2 = 12,
    ControlInfo3 = 13,
    ControlInfo4 = 14,
    ControlInfo5 = 15,
    ControlInfo6 = 16,
    ControlInfo7 = 17,
    ControlInfo8 = 18,
    ControlInfo9 = 19,
    ControlInfo10 = 20,
    ControlInfo11 = 21,
    /// 22: the last control message — kind 2, so its early TRUE
    /// leaves the print machine mid-flight (state 17 spends its
    /// first ticks finishing that fade — msg 13 never prints).
    ControlInfo23 = 22,
    /// 23: hide the touch button, clear SUB_0, start the yes/no.
    AskUnderstood = 23,
    /// 24: the yes/no menu's input.
    AskUnderstoodHandleYesNo = 24,
    /// 27: the understood-yes blend + fade-out.
    UnderstoodYes = 27,
    /// 28: poll the fade, clear both BG0s, back to the menu.
    UnderstoodYesWaitFade = 28,
    /// 29: the understood-no blend, back to the control info.
    UnderstoodNo = 29,
    /// 34: the adventure-info setup + fade-in.
    FadeInAdventureInfo = 34,
    /// 35: poll the fade.
    WaitFadeInAdventureInfo = 35,
    /// 36–41: the adventure-info message chain.
    AdventureInfo1 = 36,
    AdventureInfo2 = 37,
    AdventureInfo3 = 38,
    AdventureInfo4 = 39,
    AdventureInfo5 = 40,
    AdventureInfo6 = 41,
    /// 42: the 6×1 fade-out.
    AdventureInfoFadeOut = 42,
    /// 43: poll the fade, then back to the tutorial menu.
    WaitFadeOutAdventureInfo = 43,
    /// 44: the no-info-needed setup + fade-in.
    NoInfoNeededFadeIn = 44,
    /// 45: poll the fade, then the 40-frame hold.
    WaitFadeInNoInfoNeeded = 45,
    /// 46: the time-of-day dialog, then the BGM fade.
    PrintTimeOfDayMsg = 46,
    /// 47: Oak's pic + the 16-step brightness-in.
    ShowOak = 47,
    /// 48: poll the transition.
    WaitFadeInOak = 48,
    /// 49: "Welcome to the world of Pokémon!"
    WelcomeToWorld = 49,
    /// 50: slide Oak's pic right.
    SlideOakRight = 50,
    /// 51: "This world is inhabited…" — then the Marill's ball.
    ThisWorldIsInhabited = 51,
    /// 52: the 30-frame hold, then the two 4-step flash transitions.
    BallOpeningFlash = 52,
    /// 53: the flash done — the Marill appears at full brightness.
    AppearMarill = 53,
    /// 54: the OBJ brightness ramp down, then the cry.
    MarillCry = 54,
    /// 55: the post-cry hold.
    WaitMarillCry = 55,
    /// 56: "You and a Pokémon…"
    WeLiveAlongside = 56,
    /// 57: the OBJ blend-off that hides the Marill.
    HideMarill = 57,
    /// 58: the post-hide hold.
    WaitAfterHideMarill = 58,
    /// 59: slide Oak's pic back left.
    SlideOakLeft = 59,
    /// 60: "Tell me a little about yourself."
    TellMeAboutYourself = 60,
    /// 61: "Are you a boy? Or are you a girl?" — then the SUB fade.
    AreYouAGender = 61,
    /// 62: poll the fade.
    WaitFadeOutToAskGender = 62,
    /// 63: the gender-select setup + the SUB fade-in.
    SetupGenderSelectMenu = 63,
    /// 64: poll the fade, then take the last-chosen gender.
    WaitFadeInGenderSelectMenu = 64,
    /// 65: the gender-select input.
    GenderSelectMenuHandleInput = 65,
    /// 66: the confirm frame, SUB_0 cleared, the gendered message
    /// queued.
    PrepareAskConfirmGender = 66,
    /// 67: "Are you sure?" (the gendered grammar vestige).
    AskConfirmGender = 67,
    /// 68: the confirm multichoice.
    ConfirmGenderYesNoInitMenu = 68,
    /// 69: the confirm multichoice's input (SUB_0/SUB_2 on every
    /// tick).
    ConfirmGenderYesNoHandleInput = 69,
    /// 70: route by the confirmed choice.
    ConfirmGenderYesNoHandleResult = 70,
    /// 71: the no-branch's SUB fade-out.
    ConfirmGenderNoWaitFadeOut = 71,
    /// 72: the re-setup, the SUB fade-in, back to the question.
    ConfirmGenderNoWaitFadeIn = 72,
    /// 93: "Your name?" — the pre-naming hold.
    ConfirmGenderYes = 93,
    /// 94: the 40-frame hold before the naming screen.
    PromptNameDelayBefore = 94,
    /// 95: launch the naming overlay (the outer machine takes over).
    PromptNameLaunchNamingScreen = 95,
    /// 96: the post-naming restore.
    PromptNameRestoreGraphicsAfter = 96,
    /// 97: "Is your name…?" — the confirm dialog.
    ConfirmNameYesNoInitMenu = 97,
    /// 98: the name-confirm multichoice's input.
    ConfirmNameYesNoHandleInput = 98,
    /// 99: route by the confirmed choice.
    ConfirmNameYesNoHandleResult = 99,
    /// 100: the yes-branch's SUB fade-out.
    ConfirmNameYes = 100,
    /// 101: the yes-branch's re-setup + SUB fade-in.
    ConfirmNameYesWaitFadeOut = 101,
    /// 102: poll the fade.
    ConfirmNameYesWaitFadeIn = 102,
    /// 103: "Your very own Pokémon story is about to unfold."
    YourAdventureUnfolds = 103,
    /// 110: the 6×1 fade-out.
    FadeOutFromLastOakMessage = 110,
    /// 111: poll the fade.
    WaitFadeOutFromLastOakMessage = 111,
    /// 120: the player pic + the 6×1 fade-in.
    FadeInToShrinkAnim = 120,
    /// 121: pret's fade poll, dead — the case falls through into
    /// `NopBeforeShrinkAnim` the same tick
    /// (`oaks_speech.c:2121-2131`).
    WaitFadeInToShrinkAnim = 121,
    /// 123: one tick: `state = INIT_SHRINK_ANIM_STATE`.
    NopBeforeShrinkAnim = 123,
    /// 124: hide the touch button, arm the shrink anim.
    InitShrinkAnimState = 124,
    /// 125: the 30-frame hold.
    DelayBeforeShrinkAnim = 125,
    /// 126: the shrink anim — its TRUE finishes the speech.
    RunShrinkAnim = 126,
}

/// `data->printDialogMsgState` (`oaks_speech_internal.h`) — the
/// dialog sub-machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogState {
    /// 0: the window, the frame, the printer's construction print.
    PrintText,
    /// 1: poll `TextPrinterCheckActive`.
    WaitForPrinter,
    /// 2: mode 0 waits for A; mode 1 returns immediately.
    Exit,
}

/// `data->printAndFadeFullScreenTextState` — the fullscreen text
/// sub-machine. Every `DoMainTask` caller goes through the centered
/// wrapper (`oaks_speech.c:1063-1065`), so the y/height centering
/// (`0xFFFF`) is this machine's case 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrintFadeState {
    /// 0: MAIN_0 off, the window built and instantly printed.
    Print,
    /// 1: `CopyWindowToVram` — the window commits to the frame.
    Commit,
    /// 2: the MAIN_0 blend-in; kind 2 returns TRUE here, leaving
    /// the machine at BlendOut for the next caller.
    BlendIn,
    /// 3: wait for A or B.
    WaitInput,
    /// 4: the MAIN_0 blend-out.
    BlendOut,
    /// 5: drop the window, clear MAIN_0, return TRUE.
    Cleanup,
}

/// `OakSpeech_Main`'s `pState` (`oaks_speech.c:566-631`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OuterState {
    /// 0: the full re-initialization.
    Init,
    /// 1: the touch-advance button, then one `DoMainTask` pass.
    Run,
    /// 2: the speech-over fade-out polled, then the cleanup —
    /// `ret TRUE` ends the app.
    CleanupExit,
    /// 3: the naming-launch fade-out polled, then the cleanup.
    OverlayFade,
    /// 4: the naming overlay runs (stalled until the name lands).
    OverlayRun,
    /// 5: one tick, back to Init.
    OverlayDone,
}

/// `enum YesNoResponse` — the yes/no menu's results.
const YESNO_RESPONSE_YES: u8 = 1;
const YESNO_RESPONSE_NO: u8 = 2;

/// The touch-advance button's sprite actions
/// (`OakSpeech_TouchToAdvanceButtonAction`).
const TOUCHTOADVANCE_HIDE: u8 = 0;
const TOUCHTOADVANCE_SHOW: u8 = 1;
const TOUCHTOADVANCE_PRESS: u8 = 2;
const TOUCHTOADVANCE_RELEASE: u8 = 3;

/// One of `brightness.c`'s two static `BrightnessData`s — MAIN's
/// `sMainScreenBrightnessData` and SUB's
/// `sSubScreenBrightnessData` (`brightness.c:7-8`), the blend-unit
/// brightness fades Oak's speech uses for Oak's appearance (state
/// 47) and the ball-opening flash (state 52).
///
/// `StartBrightnessTransition` runs its `InitBrightnessTransition`
/// inline after a direct write of the start value (`brightness.c:64-76`),
/// and `DoAllScreenBrightnessTransitionStep` steps the MAIN static
/// then the SUB one once per frame (`:110-118`) — the scene's tail.
#[derive(Debug, Clone, Copy)]
struct BrightnessTransition {
    /// `transitionActive`.
    active: bool,
    /// `surfaceMask` — the blend's first-target planes.
    surface_mask: u8,
    /// `stepCount`.
    step_count: u16,
    /// `targetBrightness`.
    target: i16,
    /// `currentBrightness`.
    current: i16,
    /// `brightnessDiff`, after the sign fold (`brightness.c:52-57`).
    diff: u16,
    /// `transitionDirection`.
    dir: i8,
    /// `stepSizeInteger`.
    step_int: u16,
    /// `stepSizeFractional`.
    step_frac: u16,
    /// `fractionalCount`.
    frac_count: u16,
}

impl BrightnessTransition {
    /// `InitBrightnessTransition` (`brightness.c:41-62`). Returns the
    /// start value — the caller's inline `G2_SetBlendBrightness` of it
    /// (`brightness.c:67`, `:72`).
    fn start(&mut self, step_count: u16, target: i16, start: i16, surface_mask: u8) -> i16 {
        self.active = true;
        self.surface_mask = surface_mask;
        self.step_count = step_count;
        self.target = target;
        self.current = start;
        let diff = start - target;
        if diff > 0 {
            self.dir = -1;
            self.diff = diff as u16;
        } else {
            self.dir = 1;
            self.diff = -diff as u16;
        }
        self.step_int = self.diff / step_count;
        self.step_frac = self.diff % step_count;
        self.frac_count = 0;
        start
    }

    /// `DoBrightnessTransitionStep` (`brightness.c:10-39`), minus the
    /// trailing write — returns `currentBrightness`, the value the
    /// scene's `G2`/`G2S_SetBlendBrightness` carries.
    fn step(&mut self) -> i16 {
        let mut finished = false;
        // The ARM's precondition, verbatim: the else branch snaps to
        // the target and finishes.
        if self.target != self.current + i16::from(self.dir) * self.step_int as i16
            && self.current != self.target
        {
            self.current += i16::from(self.dir) * self.step_int as i16;
            self.frac_count += self.step_frac;
            if self.frac_count >= self.step_count {
                self.current += i16::from(self.dir);
                if self.current != self.target {
                    self.frac_count -= self.step_count;
                } else {
                    finished = true;
                }
            }
        } else {
            self.current = self.target;
            finished = true;
        }
        if finished {
            self.active = false;
        }
        self.current
    }

    /// `IsBrightnessTransitionActive` (`brightness.c:120-136`) — the
    /// ARM's own misnomer: TRUE when the transition is *not* running.
    #[must_use]
    fn is_brightness_transition_active(&self) -> bool {
        !self.active
    }
}

/// `OakSpeechMultichoice` (`oaks_speech_internal.h`) — the shared
/// `data->menuData`, both the tutorial and the confirm menus.
/// `unk_0` is dropped: every write is dead.
#[derive(Debug, Clone, Copy)]
struct MultichoiceMenu {
    /// `cursorPos`.
    cursor_pos: u8,
    /// `numOptions` — `sMultichoiceMenuParam[menuId][1]`.
    num_options: u8,
    /// `pressDelay` — the touch/A confirm's 20-frame hold.
    press_delay: u8,
    /// `flashDelay`.
    flash_delay: u8,
    /// `flashFramesPer` — 16 armed, 2 once a choice is active.
    flash_frames_per: u8,
    /// `flashState` — the SUB_1 flash toggle.
    flash_state: u8,
    /// `inPadMode`.
    in_pad_mode: bool,
}

impl MultichoiceMenu {
    /// `OakSpeech_InitMultichoiceMenu` (`oaks_speech.c:1251-1259`).
    fn init(&mut self, menu_id: usize) {
        self.cursor_pos = 0;
        self.num_options = MULTICHOICE_OPTIONS[menu_id];
        self.press_delay = 0;
        self.flash_delay = 0;
        self.flash_frames_per = 16;
        self.in_pad_mode = false;
    }
}

/// `TouchscreenHitbox_FindRectAtTouchNew`/`_AtTouchHeld` — the first
/// rect containing the touch. The rects are half-open on right and
/// bottom (the `main_menu` idiom).
fn find_rect_at_touch(touch: &Touch, rects: &[(u16, u16, u16, u16)]) -> Option<usize> {
    rects
        .iter()
        .position(|&(top, bottom, left, right)| {
            u32::from(touch.x) >= u32::from(left)
                && u32::from(touch.x) < u32::from(right)
                && u32::from(touch.y) >= u32::from(top)
                && u32::from(touch.y) < u32::from(bottom)
        })
}

/// `OakSpeech_GetTimeOfDayIntroMsg` (`oaks_speech.c:1477-1495`) —
/// the hour·100+minute ranges pick the greeting; the pinned clock's
/// noon lands on message 2.
fn time_of_day_intro_msg(rtc: &RtcDateTime) -> usize {
    let hhmm = rtc.minute + rtc.hour * 100;
    if (400..=1059).contains(&hhmm) {
        1
    } else if (1100..=1559).contains(&hhmm) {
        2
    } else if (1600..=1859).contains(&hhmm) {
        3
    } else if (1900..=2359).contains(&hhmm) {
        4
    } else if (0..=359).contains(&hhmm) {
        5
    } else {
        1
    }
}

/// `G2_SetBlendBrightness`'s register mapping (`GXx_SetBlendBrightness_`):
/// the mask becomes the blend's first target, the sign picks the
/// effect, and the weight is `|v|` in the register's 5 bits — 0 is
/// the Up effect with weight 0.
fn blend_brightness(mask: u8, v: i16) -> Blend {
    let effect = if v >= 0 {
        BlendEffect::BrightnessUp
    } else {
        BlendEffect::BrightnessDown
    };
    Blend {
        plane1: mask,
        effect,
        plane2: 0,
        eva: 0,
        ebv: 0,
        evy: (v.unsigned_abs() & 0x1F) as u8,
    }
}

/// The Oak speech scene.
pub struct OakSpeech {
    /// Intro OBJ resources from resdat 24/25/26/27/78.
    gender_sprites: [Sprite; 2],
    gender_visible: [bool; 2],
    touch_sprite: Sprite,
    marill_sprite: Sprite,
    marill_visible: bool,
    /// The logical frame the last tick produced.
    frame: LogicalFrame,
    /// The palette-fade pair — both-screen works everywhere but the
    /// FADE_SUB_ONLY gender-confirmation fades.
    fade: BrightnessFade,
    /// The scene's printer policy.
    flags: TextFlags,
    /// The pinned clock — `OakSpeech_GetTimeOfDayIntroMsg`'s reader.
    rtc: RtcDateTime,

    /// Font 0 — the fullscreen text and the multichoice width.
    font0: Font,
    /// Font 1 — the dialogs.
    font1: Font,
    /// Font 4 — the multichoice buttons, the touch message, the
    /// yes/no buttons.
    font4: Font,
    /// The font assets the pushed glyphs reference.
    font0_asset: AssetId,
    font1_asset: AssetId,
    font4_asset: AssetId,
    /// The focus-indicator asset every printer carries.
    focus_asset: AssetId,

    /// The speech bank's messages, cloned at load.
    messages: Vec<GameString>,
    /// `data->msgFormat` — `MessageFormat_New`'s count 8
    /// (`MessageFormat_New_Custom(8, 32, heap)`); fields 0 and 1 the
    /// player and rival names (`BufferString`'s plain copy,
    /// `message_format.c:95-98`).
    msg_format: MessageFormat,
    /// The yes/no menu's own `MessageFormat` — its messages expand
    /// against never-set fields.
    yesno_format: MessageFormat,

    // InitBgs' placements — re-pushed identically at each pState-0
    // re-init.
    /// `LoadUserFrameGfx2`'s NCGR.
    gfx2_tiles: AssetId,
    /// `LoadUserFrameGfx1`'s NCGR.
    gfx1_tiles: AssetId,
    /// `LoadUserFrameGfx2`'s NCLR (member 0x1A).
    gfx2_pal: AssetId,
    /// `LoadUserFrameGfx1`'s NCLR (member 0x19).
    gfx1_pal: AssetId,
    /// `LoadFontPal0`'s NCLR — loaded to MAIN and SUB both.
    font_pal0: AssetId,
    /// `LoadFontPal1`'s NCLR.
    font_pal1: AssetId,

    /// MAIN BG3's button-tutorial NCGR (member 0).
    button_main_char: AssetId,
    /// SUB BG3's (member 32).
    button_sub_char: AssetId,
    /// The tutorial palettes, HeartGold arm: MAIN member 1, SUB
    /// member 30.
    tutorial_main_pal: AssetId,
    tutorial_sub_pal: AssetId,
    /// SUB palette word 12, saved by LoadButtonTutorialGfx.
    gender_frame_color: u16,
    /// The u16 degree argument passed to GF_SinDeg, advancing by ten.
    gender_blink_angle: u16,
    /// `sButtonTutorialNSCR` — MAIN BG3's six layouts.
    button_screens: [AssetId; 6],
    /// `ov53_021E8558` — SUB BG3's five layouts.
    sub3_screens: [AssetId; 5],
    /// `ov53_021E8584`'s screen column — SUB BG2's three.
    sub2_screens: [AssetId; 3],
    /// SUB BG2's NCGR (member 37).
    sub2_char: AssetId,
    /// SUB BG2's NCLR (member 33).
    sub2_palette: AssetId,
    /// The multichoice cursor-flash NCGR (member 42).
    multichoice_flash_char: AssetId,
    /// `sMultichoiceMenuParam`'s screen column — SUB BG1, per menu.
    multichoice_screens: [AssetId; 4],
    /// `DrawPicOnBgLayer`'s NSCR (member 9) — MAIN BG1/BG2.
    pic_screen: AssetId,
    /// Oak's NCGR/NCLR (`sBgPicNCGR_NCLR` row 1).
    oak_char: AssetId,
    oak_pal: AssetId,
    /// Ethan's — row 2; the Lyra rows and Ethan 2–4 are never drawn.
    ethan_char: AssetId,
    ethan_pal: AssetId,
    /// Lyra's — row 6.
    lyra_char: AssetId,
    lyra_pal: AssetId,
    /// `sPlayerPicShrinkGfx_Male`/`_Female` (`oaks_speech.c:333-349`):
    /// frame 0 is the pic already on screen (Ethan/Lyra 1), frames
    /// 1–4 the shrink, entry 5 the `0xFF` end sentinel — `None`.
    shrink_male: [Option<AssetId>; 6],
    shrink_female: [Option<AssetId>; 6],

    /// The yes/no menu's NCLR/NCGR/NSCR (`a/2/3/7` members 0/1/10).
    yesno_pal: AssetId,
    yesno_char: AssetId,
    yesno_screen: AssetId,

    /// `OakSpeech_Main`'s `pState`.
    outer_state: OuterState,
    /// `data->state` — the `DoMainTask` chain.
    main_state: MainState,
    /// Whether the EXIT pass has run (`ret TRUE`).
    done: bool,

    /// The dialog window's content — `data->dialogWindow`, kept
    /// past its `RemoveWindow` (a no-op on content) until a MAIN_0
    /// clear.
    dialog: Option<Window>,
    /// `data->printDialogMsgState`.
    dialog_state: DialogState,
    /// The running dialog printer (`data->textPrinter`).
    printer: Option<TextPrinter>,
    /// The fullscreen window's content, and whether
    /// `CopyWindowToVram` has committed it to the frame.
    fullscreen: Option<Window>,
    fullscreen_committed: bool,
    /// `data->printAndFadeFullScreenTextState`.
    print_fade_state: PrintFadeState,
    /// The multichoice windows' contents — replaced by each
    /// `PrintMultichoiceMenu`, kept past their `RemoveWindow`s until
    /// a SUB_0 clear.
    multichoice_windows: Vec<Window>,
    /// `data->menuData`.
    menu: MultichoiceMenu,
    /// The touch-advance window's content (`data->
    /// controlTutorialTouchMsgWindow`).
    touch_window: Option<Window>,
    /// The touch-advance sprite's draw flag.
    touch_active: bool,
    /// The touch-advance sprite's animation number (0 released, 1
    /// depressed).
    touch_anim: u8,
    /// The yes/no buttons' contents — `OakSpeechYesNo::windows`,
    /// mapped to the frame once `Start` prints them; the planes
    /// toggle per the menu, the contents persist to a SUB_0 clear.
    yesno_windows: [Window; 2],
    yesno_mapped: bool,
    /// `OakSpeechYesNo::state`.
    yesno_state: u8,
    /// `OakSpeechYesNo::result`.
    yesno_result: u8,

    /// `data->playerGender` — memset 0 at Init (male).
    player_gender: u8,
    /// `data->lastChosenGender` — the gender-select memory across
    /// the confirm loops.
    last_chosen_gender: u8,
    /// The player name — `namingScreenArgs_Player->nameInputString`,
    /// empty until the naming screen delivers.
    player_name: GameString,
    /// The rival name — `namingScreenArgs_Rival`'s; the C never
    /// launches the rival screen.
    rival_name: GameString,
    /// `data->queuedMsgId`.
    queued_msg: usize,
    /// `data->frameDelayCounter` — `OakSpeech_WaitFrames`' one
    /// shared counter.
    frame_delay_counter: u32,
    /// `DoSoundUpdateFrame`'s `work->fadeTimer` (`sound.c:100-115`)
    /// — the BGM fade the music deferral keeps honest: 6, tail-
    /// decremented, state 47 polls for 0.
    bgm_fade_timer: u32,
    /// `data->layerBlendState`/`Ev1`/`Ev2` — `OakSpeech_BlendLayer`.
    layer_blend_state: u8,
    layer_blend_ev1: u8,
    layer_blend_ev2: u8,
    /// The brightness-transition statics, MAIN and SUB
    /// (`brightness.c:7-8`).
    brightness_main: BrightnessTransition,
    brightness_sub: BrightnessTransition,
    /// `data->playerPicShrinkAnimStep` — the shrink anim's frame
    /// counter, reused by the Marill's OBJ ramp (states 53–54).
    pic_anim_step: u16,
    /// `data->playerPicShrinkAnimDelay`.
    pic_anim_delay: u16,
    /// `data->oakPicHTranslateState`/`Pos`.
    oak_translate_state: u8,
    oak_translate_pos: i16,
    /// `data->overlayManager != NULL` — the naming screen is
    /// running.
    overlay_active: bool,
    /// The delivered naming result, consumed by the overlay-run
    /// state.
    naming_done: bool,
    /// `gSystem.simulatedInputs` — the touch-advance button's
    /// synthetic A, merged into the next tick's keypad read.
    simulated_a: bool,
    /// The keys held on the previous tick, for press edges.
    prev_keys: Keys,
    /// Whether the stylus was down on the previous tick.
    prev_touch: bool,
}

/// `AddWindowParameterized` — one window from its template
/// parameters. The pixel buffer starts cleared (every window here
/// fills before its first copy).
fn add_window(
    bg: u8,
    left: u8,
    top: u8,
    width: u8,
    height: u8,
    palette: u8,
    base_tile: u16,
) -> Window {
    Window {
        bg,
        left,
        top,
        width,
        height,
        palette,
        base_tile,
        fill: 0,
        glyphs: Vec::new(),
        scroll: 0,
        frame: None,
        arrow: None,
        focus: None,
    }
}

/// `FillWindowPixelRect`/`FillWindowPixelBuffer` — the pixel
/// buffer's erase: the fill color set, the glyphs and the focus
/// state dropped with the pixels they drew into.
fn fill_window_pixels(window: &mut Window, fill: u8) {
    window.fill = fill;
    window.glyphs.clear();
    window.focus = None;
}

/// `OakSpeech_BlendLayer`'s layer parameter — the `GF_BG_LYR_*`
/// value (or the 101/102 OBJ pseudo-layers) the callers pass, named
/// for the three the machine ever blends (`oaks_speech.c:793-827`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlendLayer {
    /// `GF_BG_LYR_MAIN_0` — the fullscreen text, first blend target
    /// of the `default` arm.
    Main0,
    /// `GF_BG_LYR_SUB_2` — the tutorial and gender menus' backdrop.
    Sub2,
    /// 101 (`GF_BG_LYR_MAIN_OBJ`) — MAIN's OBJ plane, the Marill.
    MainObj,
}

impl BlendLayer {
    /// The blend plane mask and the owning engine — the switch's
    /// `(plane, screen)` pair, `screen == 0` being MAIN.
    fn plane_and_engine(self) -> (u8, bool) {
        match self {
            BlendLayer::Main0 => (plane::BG0, true),
            BlendLayer::Sub2 => (plane::BG2, false),
            BlendLayer::MainObj => (plane::OBJ, true),
        }
    }

    /// The BG layer the `ToggleBgLayer` arms flip — `None` for the
    /// OBJ pseudo-layers, whose sprite planes the deferral leaves
    /// empty (`oaks_speech.c:848-854`, `:901-907`).
    fn bg(self) -> Option<usize> {
        match self {
            BlendLayer::Main0 => Some(0),
            BlendLayer::Sub2 => Some(2),
            BlendLayer::MainObj => None,
        }
    }
}

impl OakSpeech {
    /// Loads the speech's members and builds the cleared pre-tick
    /// state.
    ///
    /// The construction frame is the register state inherited from
    /// the main menu's exit (`GX_SetDispSelect(GX_DISP_SELECT_MAIN_SUB)`,
    /// `main_menu.c:1302`): the main screen on top, everything
    /// cleared. The first tick is `pState` 0 — the black write, the
    /// BG setup, and the button-tutorial art — and the scene carries
    /// the *last* frame's palette-fade brightness until that tick's
    /// `sub_0200FBF4`.
    ///
    /// `rtc` is the game machine's pinned clock — the same
    /// `RtcDateTime` every `GF_RTC_CopyDateTime` reads.
    ///
    /// # Errors
    /// Returns the store's [`AssetsError`] when a member is missing
    /// or corrupt — unreachable in practice against the pinned dump.
    pub fn load(store: &Mutex<AssetStore>, rtc: RtcDateTime) -> Result<Self, AssetsError> {
        let mut store = store
            .lock()
            .expect("the asset store is only locked at app construction");
        // The frame graphics and fonts InitBgs places.
        let gfx2_tiles = store.load_default_dialogue_frame()?;
        let gfx1_tiles = store.load_tiles(frame_narc::NARC, frame_narc::FRAME0_CHAR)?;
        let gfx2_pal = store.load_palette(frame_narc::NARC, frame_narc::GFX2_FRAME0_PALETTE)?;
        let gfx1_pal = store.load_palette(frame_narc::NARC, frame_narc::PALETTE)?;
        let font_pal0 = store.load_palette(font_narc::NARC, font_narc::PAL0)?;
        let font_pal1 = store.load_palette(font_narc::NARC, font_narc::PAL1)?;
        let font0_asset = store.load_font(font_narc::NARC, font_narc::FONT0)?;
        let font1_asset = store.load_font(font_narc::NARC, font_narc::FONT1)?;
        let font4_asset = store.load_font(font_narc::NARC, font_narc::FONT4)?;
        let focus_asset = store.load_tiles(font_narc::NARC, font_narc::FOCUS_INDICATOR)?;
        let font0 = store
            .font(font0_asset)
            .expect("the just-loaded font")
            .clone();
        let font1 = store
            .font(font1_asset)
            .expect("the just-loaded font")
            .clone();
        let font4 = store
            .font(font4_asset)
            .expect("the just-loaded font")
            .clone();

        // The speech bank.
        let bank = store.load_msg_bank(msg_narc::NARC, MSG_BANK)?;
        let text = store.msg_bank(bank).expect("the just-loaded bank");
        let mut messages = Vec::with_capacity(MSG_COUNT);
        for m in 0..MSG_COUNT {
            let units = text.message(m).expect("the speech bank's 63 messages");
            messages.push(GameString::from_units(units));
        }

        // The button-tutorial art (`LoadButtonTutorialGfx`).
        let narc = intro_narc::NARC;
        let button_main_char = store.load_tiles(narc, intro_narc::BUTTON_TUTORIAL_MAIN_CHAR)?;
        let button_sub_char = store.load_tiles(narc, intro_narc::BUTTON_TUTORIAL_SUB_CHAR)?;
        let tutorial_main_pal = store.load_palette(narc, intro_narc::MAIN_PALETTE)?;
        let tutorial_sub_pal = store.load_palette(narc, intro_narc::SUB_PALETTE)?;
        let [r, g, b, _] = store.palette(tutorial_sub_pal).expect("loaded palette").rgba()[12];
        let gender_frame_color = u16::from(r >> 3)
            | (u16::from(g >> 3) << 5) | (u16::from(b >> 3) << 10);
        // Each member loads once — the tables below index the loads.
        let button_screens = [
            store.load_screen(narc, intro_narc::BUTTON_TUTORIAL_SCREENS[0])?,
            store.load_screen(narc, intro_narc::BUTTON_TUTORIAL_SCREENS[1])?,
            store.load_screen(narc, intro_narc::BUTTON_TUTORIAL_SCREENS[2])?,
            store.load_screen(narc, intro_narc::BUTTON_TUTORIAL_SCREENS[3])?,
            store.load_screen(narc, intro_narc::BUTTON_TUTORIAL_SCREENS[4])?,
            store.load_screen(narc, intro_narc::BUTTON_TUTORIAL_SCREENS[5])?,
        ];
        // `ov53_021E8558`: layouts 1 and 2 share member 43.
        let sub3_44 = store.load_screen(narc, intro_narc::SUB3_SCREENS[0])?;
        let sub3_43 = store.load_screen(narc, intro_narc::SUB3_SCREENS[1])?;
        let sub3_45 = store.load_screen(narc, intro_narc::SUB3_SCREENS[3])?;
        let sub3_51 = store.load_screen(narc, intro_narc::SUB3_SCREENS[4])?;
        let sub3_screens = [sub3_44, sub3_43, sub3_43, sub3_45, sub3_51];
        // `ov53_021E8584`: the SUB BG2 layouts.
        let sub2_screens = [
            store.load_screen(narc, intro_narc::SUB2_SCREENS[0])?,
            store.load_screen(narc, intro_narc::SUB2_SCREENS[1])?,
            store.load_screen(narc, intro_narc::SUB2_SCREENS[2])?,
        ];
        let sub2_char = store.load_tiles(narc, intro_narc::SUB2_CHAR)?;
        let sub2_palette = store.load_palette(narc, intro_narc::SUB2_PALETTE)?;
        let multichoice_flash_char = store.load_tiles(narc, intro_narc::MULTICHOICE_FLASH_CHAR)?;
        // `sMultichoiceMenuParam`: menus 1 and 2 share member 50.
        let mc_49 = store.load_screen(narc, intro_narc::MULTICHOICE_SCREENS[0])?;
        let mc_50 = store.load_screen(narc, intro_narc::MULTICHOICE_SCREENS[1])?;
        let mc_52 = store.load_screen(narc, intro_narc::MULTICHOICE_SCREENS[3])?;
        let multichoice_screens = [mc_49, mc_50, mc_50, mc_52];
        let pic_screen = store.load_screen(narc, intro_narc::PIC_SCREEN)?;

        // The speaker pics — one char and one palette each, the
        // animated-pic vestige's first frames.
        let oak_char = store.load_tiles(narc, intro_narc::OAK_CHAR)?;
        let oak_pal = store.load_palette(narc, intro_narc::OAK_PALETTE)?;
        let ethan_char = store.load_tiles(narc, intro_narc::ETHAN_CHARS[0])?;
        let ethan_pal = store.load_palette(narc, intro_narc::ETHAN_PALETTE)?;
        let lyra_char = store.load_tiles(narc, intro_narc::LYRA_CHARS[0])?;
        let lyra_pal = store.load_palette(narc, intro_narc::LYRA_PALETTE)?;
        // The shrink anim's frames — index 0 is the pic itself,
        // entry 5 the end sentinel.
        let shrink_male = [
            Some(ethan_char),
            Some(store.load_tiles(narc, intro_narc::SHRINK_MALE_CHARS[0])?),
            Some(store.load_tiles(narc, intro_narc::SHRINK_MALE_CHARS[1])?),
            Some(store.load_tiles(narc, intro_narc::SHRINK_MALE_CHARS[2])?),
            Some(store.load_tiles(narc, intro_narc::SHRINK_MALE_CHARS[3])?),
            None,
        ];
        let shrink_female = [
            Some(lyra_char),
            Some(store.load_tiles(narc, intro_narc::SHRINK_FEMALE_CHARS[0])?),
            Some(store.load_tiles(narc, intro_narc::SHRINK_FEMALE_CHARS[1])?),
            Some(store.load_tiles(narc, intro_narc::SHRINK_FEMALE_CHARS[2])?),
            Some(store.load_tiles(narc, intro_narc::SHRINK_FEMALE_CHARS[3])?),
            None,
        ];

        // The yes/no menu (`OakSpeechYesNo_Create`'s windows are
        // built at the pState-0 init).
        let yesno_pal = store.load_palette(yesno_narc::NARC, yesno_narc::PALETTE)?;
        let yesno_char = store.load_tiles(yesno_narc::NARC, yesno_narc::CHAR)?;
        let yesno_screen = store.load_screen(yesno_narc::NARC, yesno_narc::SCREEN)?;

        // oaks_speech_obj.c / resdat 78: the two player cells share
        // NCER 55 / NANR 56, with their own character and palette art.
        let player_cells = store.load_cells(narc, 55)?;
        let player_animation = store.load_animation(narc, 56)?;
        let make_sprite = |tiles, palette, cells, animation, x, y, priority| Sprite {
            tiles, palette, cells, animation, x, y, priority,
            sequence: 0, elapsed: 0, palette_bank: 0,
        };
        let gender_sprites = [
            make_sprite(ethan_char, ethan_pal, player_cells, player_animation, 64, 104, 0),
            make_sprite(lyra_char, lyra_pal, player_cells, player_animation, 192, 104, 0),
        ];
        let touch_sprite = make_sprite(
            store.load_tiles(narc, 60)?, store.load_palette(narc, 59)?,
            store.load_cells(narc, 61)?, store.load_animation(narc, 62)?, 256, 192, 1);
        let marill_sprite = make_sprite(
            store.load_tiles(narc, 64)?, store.load_palette(narc, 63)?,
            store.load_cells(narc, 65)?, store.load_animation(narc, 66)?, 160, 80, 0);

        // `memset(data, 0, …)`: the whole work zeroed, then the
        // fields Init sets on top (`oaks_speech.c:546-563`).
        Ok(Self {
            gender_sprites,
            gender_visible: [false; 2],
            touch_sprite,
            marill_sprite,
            marill_visible: false,
            frame: LogicalFrame {
                display: DisplaySelect::MainOnTop,
                ..LogicalFrame::default()
            },
            fade: BrightnessFade::default(),
            // TextFlags_SetCanTouchSpeedUpPrint(FALSE) — Init's
            // power-on restore; the dialogs re-enable AB speedup per
            // print.
            flags: TextFlags::default(),
            rtc,
            font0,
            font1,
            font4,
            font0_asset,
            font1_asset,
            font4_asset,
            focus_asset,
            messages,
            msg_format: MessageFormat::new(8),
            yesno_format: MessageFormat::new(8),
            gfx2_tiles,
            gfx1_tiles,
            gfx2_pal,
            gfx1_pal,
            font_pal0,
            font_pal1,
            button_main_char,
            button_sub_char,
            tutorial_main_pal,
            tutorial_sub_pal,
            gender_frame_color,
            gender_blink_angle: 0,
            button_screens,
            sub3_screens,
            sub2_screens,
            sub2_char,
            sub2_palette,
            multichoice_flash_char,
            multichoice_screens,
            pic_screen,
            oak_char,
            oak_pal,
            ethan_char,
            ethan_pal,
            lyra_char,
            lyra_pal,
            shrink_male,
            shrink_female,
            yesno_pal,
            yesno_char,
            yesno_screen,
            outer_state: OuterState::Init,
            // data->state = START_TUTORIAL_MUSIC.
            main_state: MainState::StartTutorialMusic,
            done: false,
            dialog: None,
            dialog_state: DialogState::PrintText,
            printer: None,
            fullscreen: None,
            fullscreen_committed: false,
            print_fade_state: PrintFadeState::Print,
            multichoice_windows: Vec::new(),
            menu: MultichoiceMenu {
                cursor_pos: 0,
                num_options: 0,
                press_delay: 0,
                flash_delay: 0,
                flash_frames_per: 16,
                flash_state: 0,
                in_pad_mode: false,
            },
            touch_window: None,
            touch_active: false,
            touch_anim: 0,
            yesno_windows: [Window::default(), Window::default()],
            yesno_mapped: false,
            yesno_state: 0,
            yesno_result: YESNO_RESPONSE_YES,
            player_gender: 0,
            last_chosen_gender: 0,
            player_name: GameString::new(),
            rival_name: GameString::new(),
            queued_msg: 0,
            frame_delay_counter: 0,
            bgm_fade_timer: 0,
            layer_blend_state: 0,
            layer_blend_ev1: 0,
            layer_blend_ev2: 0,
            brightness_main: BrightnessTransition {
                active: false,
                surface_mask: 0,
                step_count: 0,
                target: 0,
                current: 0,
                diff: 0,
                dir: 1,
                step_int: 0,
                step_frac: 0,
                frac_count: 0,
            },
            brightness_sub: BrightnessTransition {
                active: false,
                surface_mask: 0,
                step_count: 0,
                target: 0,
                current: 0,
                diff: 0,
                dir: 1,
                step_int: 0,
                step_frac: 0,
                frac_count: 0,
            },
            pic_anim_step: 0,
            pic_anim_delay: 0,
            oak_translate_state: 0,
            oak_translate_pos: 0,
            overlay_active: false,
            naming_done: false,
            simulated_a: false,
            prev_keys: Keys::IDLE,
            prev_touch: false,
        })
    }

    /// The delivered naming result — the naming overlay's
    /// `nameInputString` when it finishes (`OverlayManager_Run`
    /// reporting TRUE). The overlay-run state consumes it and
    /// re-arms the outer machine's re-init.
    pub fn deliver_naming_result(&mut self, name: GameString) {
        self.player_name = name;
        self.naming_done = true;
    }

    /// The engine frames by name — MAIN is engine A (`G2_*`), SUB is
    /// engine B (`G2S_*`).
    fn engine_mut(&mut self, main: bool) -> &mut EngineFrame {
        if main {
            &mut self.frame.main
        } else {
            &mut self.frame.sub
        }
    }

    /// `BgClearTilemapBufferAndCommit` — the layer's tilemap to zero.
    /// The model drops the screen reference (a cleared tilemap is not
    /// an asset) and the layer's earlier edits (`frame.rs`'s lifetime
    /// rule: the clear zeroed the buffer). The scene's windows on the
    /// layer go with it — the stand-in for the pixels the clear
    /// erases; the dialog's printer drops with its window (the C would
    /// keep printing into freed pixels — no walk clears MAIN_0
    /// mid-print).
    fn clear_layer(&mut self, main: bool, layer: usize) {
        {
            let engine = self.engine_mut(main);
            engine.bgs[layer].screen = None;
            engine.tilemap_edits.retain(|edit| match *edit {
                TilemapEdit::Palette { bg, .. } | TilemapEdit::Fill { bg, .. } => {
                    usize::from(bg) != layer
                }
            });
        }
        if main && layer == 0 {
            self.dialog = None;
            self.printer = None;
            self.fullscreen = None;
            self.fullscreen_committed = false;
        } else if !main && layer == 0 {
            self.touch_window = None;
            self.multichoice_windows.clear();
            self.yesno_mapped = false;
        }
    }

    /// `GfGfxLoader_LoadScrnData` — one NSCR onto a layer. The load
    /// replaces the layer's whole tilemap, so the layer's earlier
    /// edits drop with it (`frame.rs`'s lifetime rule).
    fn load_screen(&mut self, main: bool, layer: usize, asset: AssetId) {
        {
            let engine = self.engine_mut(main);
            engine.tilemap_edits.retain(|edit| match *edit {
                TilemapEdit::Palette { bg, .. } | TilemapEdit::Fill { bg, .. } => {
                    usize::from(bg) != layer
                }
            });
            engine.bgs[layer].screen = Some(asset);
        }
    }

    /// `OakSpeech_FillBgLayerWithPalette` (`oaks_speech.c:931-934`)
    /// — `BgTilemapRectChangePalette` over the whole 32×24 map plus
    /// the commit.
    fn fill_bg_layer_with_palette(&mut self, main: bool, bg: u8, bank: u8) {
        self.engine_mut(main).tilemap_edits.push(TilemapEdit::Palette {
            bg,
            bank,
            left: 0,
            top: 0,
            width: 32,
            height: 24,
        });
    }

    /// `OakSpeech_WaitFrames` (`oaks_speech.c:921-929`) — the one
    /// shared counter: FALSE for `delay` calls, TRUE on call
    /// `delay + 1`, the counter re-arming either way.
    fn wait_frames(&mut self, delay: u32) -> bool {
        if self.frame_delay_counter < delay {
            self.frame_delay_counter += 1;
            false
        } else {
            self.frame_delay_counter = 0;
            true
        }
    }

    /// `G2_SetBlendAlpha`/`G2S_SetBlendAlpha` — the layer blend's
    /// current weights onto one engine.
    fn write_layer_blend(&mut self, main: bool, plane: u8) {
        let eva = self.layer_blend_ev1;
        let ebv = self.layer_blend_ev2;
        self.engine_mut(main).blend = Blend {
            plane1: plane,
            effect: BlendEffect::Alpha,
            plane2: BLEND_SECOND_TARGET,
            eva,
            ebv,
            evy: 0,
        };
    }

    /// `OakSpeech_BlendLayer` (`oaks_speech.c:788-919`) — one layer
    /// alpha-blended over BG1–BG3, sixteen steps either way. The
    /// fade-in writes and toggles the plane on at call 1, the last
    /// weight lands at call 17, and case 3's `G2_BlendNone` +
    /// `G2S_BlendNone` reports TRUE at call 19; the fade-out mirrors
    /// it, toggling the plane off at call 18 instead.
    fn blend_layer(&mut self, layer: BlendLayer, fade_out: bool) -> bool {
        let (plane, main) = layer.plane_and_engine();
        match self.layer_blend_state {
            0 => {
                if !fade_out {
                    // :831-854 — arm the fade-in and show the layer.
                    self.layer_blend_ev1 = 0;
                    self.layer_blend_ev2 = 16;
                    self.layer_blend_state = 1;
                    self.write_layer_blend(main, plane);
                    if let Some(bg) = layer.bg() {
                        self.engine_mut(main).bgs[bg].enabled = true;
                    }
                } else {
                    // :855-859 — the fade-out arms without writing.
                    self.layer_blend_ev1 = 16;
                    self.layer_blend_ev2 = 0;
                    self.layer_blend_state = 2;
                }
                false
            }
            1 => {
                // :861-881 — step toward full first-target weight.
                if self.layer_blend_ev2 != 0 {
                    self.layer_blend_ev1 += 1;
                    self.layer_blend_ev2 -= 1;
                    self.write_layer_blend(main, plane);
                } else {
                    self.layer_blend_state = 3;
                }
                false
            }
            2 => {
                // :882-909 — step back, hiding the layer at the end.
                if self.layer_blend_ev1 != 0 {
                    self.layer_blend_ev1 -= 1;
                    self.layer_blend_ev2 += 1;
                    self.write_layer_blend(main, plane);
                } else {
                    self.layer_blend_state = 3;
                    if let Some(bg) = layer.bg() {
                        self.engine_mut(main).bgs[bg].enabled = false;
                    }
                }
                false
            }
            _ => {
                // :910-915 — both engines' blend units neutral.
                self.frame.main.blend = Blend::default();
                self.frame.sub.blend = Blend::default();
                self.layer_blend_state = 0;
                true
            }
        }
    }

    /// `OakSpeech_InitBgs` (`oaks_speech.c:674-750`) — both engines'
    /// four text layers from the templates, every tilemap cleared,
    /// the frame and font graphics on MAIN, the font palette on SUB,
    /// all eight planes off, and the button-tutorial art.
    ///
    /// The `pState` 0 re-init re-runs the whole function, so the
    /// char blocks and palette loads start empty and the identical
    /// placements push again.
    fn init_bgs(&mut self) {
        // InitBgFromTemplate + BgClearTilemapBufferAndCommit, MAIN
        // (:685-705) then SUB (:712-733): 256×256 4bpp layers, MAIN
        // at priority 1 (`sBgTemplate_Main`, :351-364) and SUB at 0
        // (`sBgTemplate_Sub`, :366-379); each layer's char base is
        // the template's per-layer slot — 0x18000/0x14000/0x10000/
        // 0x0c000 for layers 0–3, the 0x4000-unit slot 6−layer.
        for main in [true, false] {
            for layer in 0..4 {
                self.clear_layer(main, layer);
                let engine = self.engine_mut(main);
                engine.char_blocks = Default::default();
                engine.palette_loads.clear();
                engine.palette_overrides.clear();
                engine.bgs[layer] = BgLayer {
                    enabled: false,
                    char_base: char_base(layer as u8),
                    screen: None,
                    color_mode: ColorMode::Bpp4,
                    size: ScreenSize::W256xH256,
                    scroll_x: 0,
                    scroll_y: 0,
                    priority: if main {
                        BG_PRIORITY_MAIN
                    } else {
                        BG_PRIORITY_SUB
                    },
                };
            }
        }
        // SetBgPriority(SUB_3, 3) (:735).
        self.frame.sub.bgs[3].priority = BG_PRIORITY_SUB_3;

        // LoadUserFrameGfx2(MAIN_0, 0x3E2, 4, 0) (:707): frame 0's
        // NCGR (member 2) at tile 0x3E2, its NCLR (member 0x1A) into
        // palette bank 4.
        self.frame.main.char_blocks[usize::from(char_base(0))].push(TilePlacement {
            asset: self.gfx2_tiles,
            tile: GFX2_TILE,
        });
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.gfx2_pal,
            offset: GFX2_BANK,
            colors: BANK_COLORS,
        });
        // LoadUserFrameGfx1(MAIN_0, 0x3D9, 3, 0) (:708): member 0's
        // NCGR at 0x3D9, its NCLR (member 0x19) into bank 3.
        self.frame.main.char_blocks[usize::from(char_base(0))].push(TilePlacement {
            asset: self.gfx1_tiles,
            tile: GFX1_TILE,
        });
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.gfx1_pal,
            offset: GFX1_BANK,
            colors: BANK_COLORS,
        });
        // LoadFontPal0(MAIN_BG, 0xA0) + LoadFontPal1(MAIN_BG, 0xC0)
        // (:709-710) — byte offsets into palette RAM, i.e. color
        // offsets 80 and 96.
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.font_pal0,
            offset: FONT0_MAIN_OFFSET,
            colors: BANK_COLORS,
        });
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.font_pal1,
            offset: FONT1_MAIN_OFFSET,
            colors: BANK_COLORS,
        });
        // LoadFontPal0(SUB_BG, 0x1C0) (:736) — color offset 224.
        // BG_ClearCharDataRange(SUB_0, 0x20) (:737) clears a tile
        // range nothing ever places in — a no-op on the model.
        self.frame.sub.palette_loads.push(PaletteLoad {
            asset: self.font_pal0,
            offset: FONT0_SUB_OFFSET,
            colors: BANK_COLORS,
        });

        // The eight ToggleBgLayer OFF (:740-747) — the layer reset
        // above already wrote them, as do pState 0's
        // DisableEngineA/BPlanes and the SetVisiblePlane(0) writes.
        // LoadButtonTutorialGfx (:748) + layerBlendState = 0 (:749).
        self.load_button_tutorial_gfx();
        self.layer_blend_state = 0;
    }

    /// `OakSpeech_LoadButtonTutorialGfx` (`oaks_speech.c:1156-1184`)
    /// — the tutorial button art on both BG3s, the HeartGold
    /// palettes, layout 1, and the black backdrops.
    fn load_button_tutorial_gfx(&mut self) {
        // LoadCharData member 0 to MAIN_3 and member 32 to SUB_3
        // (:1162, :1164) — each BG3's own char block, tile 0. The
        // BG_ClearCharDataRange(MAIN_0, 0x20) between them (:1163)
        // clears a range nothing places in.
        self.frame.main.char_blocks[usize::from(char_base(3))].push(TilePlacement {
            asset: self.button_main_char,
            tile: 0,
        });
        self.frame.sub.char_blocks[usize::from(char_base(3))].push(TilePlacement {
            asset: self.button_sub_char,
            tile: 0,
        });
        // The HeartGold arm (:1165-1167): NCLR member 1 to MAIN and
        // member 30 to SUB, each at slot offset 0 — 0x60 and 0xA0
        // bytes of palette (:1172-1173).
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.tutorial_main_pal,
            offset: 0,
            colors: TUTORIAL_MAIN_PAL_COLORS,
        });
        self.frame.sub.palette_loads.push(PaletteLoad {
            asset: self.tutorial_sub_pal,
            offset: 0,
            colors: TUTORIAL_SUB_PAL_COLORS,
        });
        self.frame.sub.palette_overrides.retain(|&(index, _)| index >= TUTORIAL_SUB_PAL_COLORS);
        // SetButtonTutorialScreenLayout(1) (:1179), the
        // DrawPic(NONE, NONE) nop (:1180), ov53_021E67C4(0) (:1181).
        self.set_button_tutorial_screen_layout(1);
        // DrawPicOnBgLayer's PIC_NONE guard fails — the nop call,
        // kept for the call itself (:1180).
        self.draw_pic(Pic::None);
        self.load_sub3_backdrop(0);
        // BG_SetMaskColor(MAIN_0/SUB_0, RGB_BLACK) (:1182-1183) —
        // the backdrops, already black at power-on.
        self.frame.main.backdrop = 0;
        self.frame.sub.backdrop = 0;
    }

    /// `OakSpeech_SetButtonTutorialScreenLayout`
    /// (`oaks_speech.c:1186-1193`) — MAIN BG3's NSCR from
    /// `sButtonTutorialNSCR`, guarded `a1 < 6`.
    fn set_button_tutorial_screen_layout(&mut self, layout: usize) {
        if layout < 6 {
            let asset = self.button_screens[layout];
            self.load_screen(true, 3, asset);
        }
    }

    /// `OakSpeech_DrawPicOnBgLayer(data, pic, PIC_NONE)`
    /// (`oaks_speech.c:1195-1204`) — the speaker pic onto MAIN_1.
    /// The `layer2pic` arm (:1206-1211) never runs — every caller
    /// passes `OAK_SPEECH_PIC_NONE` second.
    fn draw_pic(&mut self, pic: Pic) {
        let Some((char_asset, pal_asset)) = self.pic_assets(pic) else {
            // PIC_NONE — the guard's `layer1pic != 0` fails.
            return;
        };
        // LoadCharData to MAIN_1 (block 5, tile 0), the NCLR at color
        // offset 112 (slot 0xE0, 32 bytes), NSCR member 9, and the
        // whole-map palette bank 7.
        self.frame.main.char_blocks[usize::from(char_base(1))].push(TilePlacement {
            asset: char_asset,
            tile: 0,
        });
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: pal_asset,
            offset: PIC_PALETTE_OFFSET,
            colors: PIC_PALETTE_COLORS,
        });
        let screen = self.pic_screen;
        self.load_screen(true, 1, screen);
        self.fill_bg_layer_with_palette(true, 1, 7);
    }

    /// `sBgPicNCGR_NCLR`'s row for a pic (`oaks_speech.c:496-544`) —
    /// the char and palette members the rows 1 (Oak), 2 (Ethan), and
    /// 6 (Lyra) name; the other rows are never drawn.
    fn pic_assets(&self, pic: Pic) -> Option<(AssetId, AssetId)> {
        match pic {
            Pic::None => None,
            Pic::Oak => Some((self.oak_char, self.oak_pal)),
            Pic::Ethan => Some((self.ethan_char, self.ethan_pal)),
            Pic::Lyra => Some((self.lyra_char, self.lyra_pal)),
        }
    }

    /// `ov53_021E67C4` (`oaks_speech.c:1214-1226`) — SUB BG3's NSCR
    /// from `ov53_021E8558`, layouts 1 and 2 recoloring the whole
    /// map to palette banks 3 and 2.
    fn load_sub3_backdrop(&mut self, layout: usize) {
        if layout >= 5 {
            return;
        }
        let asset = self.sub3_screens[layout];
        self.load_screen(false, 3, asset);
        if layout == 1 {
            self.fill_bg_layer_with_palette(false, 3, 3);
        } else if layout == 2 {
            self.fill_bg_layer_with_palette(false, 3, 2);
        }
    }

    /// `ov53_021E6824` (`oaks_speech.c:1228-1249`) — the SUB BG2
    /// menu backdrop for layout `a1` (1 confirmation, 0 tutorial), its scroll
    /// offsets included.
    fn load_sub2_menu_backdrop(&mut self, layout: usize) {
        // The NSCR from `ov53_021E8584`'s screen column, the
        // whole-map palette bank 7, NCLR member 33 at color offset
        // 112 (slot 0xE0, 0x60 bytes), and NCGR member 37 (block 4,
        // tile 0). The BG_ClearCharDataRange(SUB_2, 0x20) between
        // the palette and char loads (:1235) clears the range the
        // char load then fills — a no-op on the placements.
        let asset = self.sub2_screens[layout];
        self.load_screen(false, 2, asset);
        self.fill_bg_layer_with_palette(false, 2, 7);
        self.frame.sub.palette_loads.push(PaletteLoad {
            asset: self.sub2_palette,
            offset: SUB2_PALETTE_OFFSET,
            colors: SUB2_PALETTE_COLORS,
        });
        self.frame.sub.char_blocks[usize::from(char_base(2))].push(TilePlacement {
            asset: self.sub2_char,
            tile: 0,
        });
        // ScheduleSetBgPosText(SUB_1, SET_Y, 0) (:1237).
        self.frame.sub.bgs[1].scroll_y = 0;
        // :1238-1248 — the male arm scrolls SUB_0/SUB_2/SUB_1 right
        // by 0x88; the female arm resets them.
        if layout == 1 {
            let scroll = if self.player_gender == 0 { GENDER_SCROLL_X } else { 0 };
            self.frame.sub.bgs[0].scroll_x = scroll;
            self.frame.sub.bgs[2].scroll_x = scroll;
            self.frame.sub.bgs[1].scroll_x = scroll;
        }
    }

    /// `ClearBgLayer0TopBottom` (`oaks_speech.c:2164-2167`) — the
    /// two BG0 tilemaps.
    fn clear_bg_layer0_top_bottom(&mut self) {
        self.clear_layer(true, 0);
        self.clear_layer(false, 0);
    }

    /// `OakSpeech_Main`'s `pState` 0 (`oaks_speech.c:570-589`) — the
    /// black write, the BG setup, the printer bank reset, the
    /// sprite engine, and the yes/no menu's windows.
    fn init_state(&mut self) {
        // sub_0200FBF4(PM_LCD_TOP/BOTTOM, RGB_BLACK) (:571-572) —
        // both screens' master brightness to black, immediately.
        self.fade.write(-16);
        let black = self.fade.brightness();
        self.frame.main.brightness = black;
        self.frame.sub.brightness = black;
        // The VBlank/HBlank callback clears, the plane disables, and
        // the SetVisiblePlane(0) writes (:573-578) are InitBgs'
        // cleared state. SetKeyRepeatTimers(4, 8) (:579) — pad
        // repeat, deferred with the pad itself.
        self.init_bgs();
        // InitMsgPrinter (:581) — ResetAllTextPrinters, i.e. no
        // running printer; the msg data and format live in `load`.
        self.printer = None;
        // BgClearTilemapBufferAndCommit(SUB_0) (:582) — InitBgs
        // cleared it a second time; the same no-op here.
        self.clear_layer(false, 0);
        self.gender_visible = [false; 2];
        self.marill_visible = false;
        // OakSpeechYesNo_Create(bgConfig, sprites[4], 6, 4, 14,
        // heap) (:585) — CreateWindows runs at Create: the two SUB_0
        // windows, filled but unmapped until Start prints them.
        self.yesno_create_windows();
        // Main_SetVBlankIntrCB + GfGfx_BothDispOn (:586-587) — the
        // display enable the frame's layer flags already carry.
    }

    /// `OakSpeechYesNo_CreateWindows`
    /// (`oaks_speech_yesnomenu.c:106-115`) — the two button windows
    /// from `AddWindowParameterized`'s templates (left 4, palette
    /// 14, base tiles 0x50/0x80), each `FillWindowPixelBuffer`'d to
    /// 0 and unmapped.
    fn yesno_create_windows(&mut self) {
        for (window, template) in self.yesno_windows.iter_mut().zip(YESNO_TEMPLATES.iter()) {
            let (top, width, height, base_tile) = *template;
            *window = add_window(0, 4, top, width, height, 14, base_tile);
            fill_window_pixels(window, 0);
        }
        self.yesno_mapped = false;
        self.yesno_state = 0;
        self.yesno_result = YESNO_RESPONSE_YES;
    }

    /// `OakSpeech_PrintDialogMsg` (`oaks_speech.c:936-984`) — the
    /// framed dialog on MAIN_0: print at the options' frame delay,
    /// then `waitButtonMode` 0's A-release or the immediate
    /// anything-else return. `RemoveWindow` frees the window struct,
    /// not its pixels — the dialog stays in the frame until a MAIN_0
    /// clear.
    fn print_dialog_msg(
        &mut self,
        input: Input,
        new_keys: Keys,
        msg_num: usize,
        wait_button_mode: u8,
    ) -> bool {
        match self.dialog_state {
            DialogState::PrintText => {
                // AddWindow + FillWindowPixelRect(0xF, 0, 0, 216, 32)
                // + DrawFrameAndWindow2(FALSE, 0x3E2, 4) (:941-943).
                let (left, top, width, height, palette, base_tile) = DIALOG_TEMPLATE;
                let mut window = add_window(0, left, top, width, height, palette, base_tile);
                fill_window_pixels(&mut window, 0xF);
                window.frame = Some(WindowFrame {
                    base_tile: GFX2_TILE,
                    palette: 4,
                    dialogue: true,
                });
                self.dialog = Some(window);
                // TextFlags_SetCanABSpeedUpPrint(TRUE) +
                // SetAutoScrollParam(AUTO_SCROLL_OFF) (:945-946).
                self.flags.set_can_ab_speed_up_print(true);
                self.flags.set_auto_scroll_param(AUTO_SCROLL_OFF);
                // ReadMsgData, the two BufferStrings — plain copies,
                // `message_format.c:95-98` — and
                // StringExpandPlaceholders (:948-953): the player
                // name into field 0, the rival name into field 1.
                self.msg_format.set_string(0, &self.player_name);
                self.msg_format.set_string(1, &self.rival_name);
                let expanded = self
                    .msg_format
                    .expand_placeholders(self.messages[msg_num].units())
                    .expect("the speech bank's placeholders resolve");
                // AddTextPrinterParameterized(window, 1, string, 0,
                // 0, Options_GetTextFrameDelay(options), NULL)
                // (:956) — the new game's default options carry text
                // speed 1, frame delay 4.
                self.printer = Some(TextPrinter::new(
                    1,
                    self.font1_asset,
                    self.focus_asset,
                    expanded,
                    0,
                    0,
                    font_color(1),
                    TEXT_FRAME_DELAY,
                    GFX2_TILE,
                ));
                // The construction-frame render — the print task runs
                // after this exec.
                self.render_dialog(input);
                self.dialog_state = DialogState::WaitForPrinter;
                false
            }
            DialogState::WaitForPrinter => {
                // TextPrinterCheckActive (:960-964): finished → the
                // EXIT wait; otherwise this tick's print step.
                if self.printer.as_ref().is_some_and(TextPrinter::is_finished) {
                    self.printer = None;
                    self.dialog_state = DialogState::Exit;
                } else {
                    self.render_dialog(input);
                }
                false
            }
            DialogState::Exit => {
                // :966-980 — mode 0 waits for a new A (the SE and the
                // write-only lastInputWasTouch deferred); anything
                // else returns immediately.
                let ret = if wait_button_mode == 0 {
                    new_keys.any(key::A)
                } else {
                    true
                };
                if ret {
                    // RemoveWindow + the state's re-arm (:977-978).
                    self.dialog_state = DialogState::PrintText;
                }
                ret
            }
        }
    }

    /// The dialog's print step — `RunTextPrinter` into the dialog
    /// window.
    fn render_dialog(&mut self, input: Input) {
        let Some(printer) = self.printer.as_mut() else {
            return;
        };
        let Some(window) = self.dialog.as_mut() else {
            return;
        };
        printer.render(&self.font1, window, input, &mut self.flags);
    }

    /// `OakSpeech_PrintAndFadeFullScreenText` through the centered
    /// wrapper (`oaks_speech.c:986-1061`, `:1063-1065`) — the
    /// borderless fullscreen message on MAIN_0: print, commit,
    /// blend in, wait for A or B, blend out, clear. The sub-machine
    /// persists across the callers: kind 2's early return leaves it
    /// at the blend-out, so the next caller's message never prints —
    /// state 17's msg 13, the genuine C skip.
    fn print_and_fade(&mut self, input: Input, new_keys: Keys, msg_num: usize, kind: u8) -> bool {
        match self.print_fade_state {
            PrintFadeState::Print => {
                // ToggleBgLayer(MAIN_0, OFF) (:992) — the plane off
                // while the window prints.
                self.frame.main.bgs[0].enabled = false;
                // The raw message — no placeholders (:994) —
                // vertically centered (:995-1000).
                let lines = self.messages[msg_num].count_lines() as i32;
                let y = ((24 - 2 * lines) / 2) as u8;
                let height = (2 * lines) as u8;
                // copy1 and copy2 are the same template
                // (`sFullScreenMsgWindowTemplate_copy1/2`,
                // :203-242); kind 3 shifts the left edge 4 tiles
                // (:1016-1017).
                let (left, _, width, _, palette, base_tile) = FULLSCREEN_TEMPLATE;
                let mut window = add_window(
                    0,
                    left + if kind == 3 { 4 } else { 0 },
                    y,
                    width,
                    height,
                    palette,
                    base_tile,
                );
                // FillWindowPixelRect(0, 0, 0, 0xC0, 0xC0) (:1007,
                // :1020).
                fill_window_pixels(&mut window, 0);
                // Font 0, instant — color (1,2,0) for kind 1, (15,2,0)
                // otherwise (:1008, :1021).
                let color = if kind == 1 {
                    TextColor::new(1, 2, 0)
                } else {
                    TextColor::new(15, 2, 0)
                };
                let mut printer = TextPrinter::new(
                    0,
                    self.font0_asset,
                    self.focus_asset,
                    self.messages[msg_num].clone(),
                    0,
                    0,
                    color,
                    TEXT_SPEED_INSTANT,
                    0,
                );
                // The instant print runs to completion within the
                // frame; CopyWindowToVram lands next call.
                printer.render_instant(&self.font0, &mut window, input, &mut self.flags);
                self.fullscreen = Some(window);
                self.fullscreen_committed = false;
                self.print_fade_state = PrintFadeState::Commit;
                false
            }
            PrintFadeState::Commit => {
                // CopyWindowToVram (:1028) — the content in the
                // tilemap from here.
                self.fullscreen_committed = true;
                self.print_fade_state = PrintFadeState::BlendIn;
                false
            }
            PrintFadeState::BlendIn => {
                // :1031-1039 — the fade-in, TRUE at the 19th call;
                // kind 2 returns the same call and leaves the machine
                // at the blend-out.
                if self.blend_layer(BlendLayer::Main0, false) {
                    if kind == 2 {
                        self.print_fade_state = PrintFadeState::BlendOut;
                        return true;
                    }
                    self.print_fade_state = PrintFadeState::WaitInput;
                }
                false
            }
            PrintFadeState::WaitInput => {
                // :1040-1046 — a new A or B releases; the touch
                // branch's lastInputWasTouch is write-only and the SE
                // deferred.
                if new_keys.any(key::A | key::B) {
                    self.print_fade_state = PrintFadeState::BlendOut;
                }
                false
            }
            PrintFadeState::BlendOut => {
                // :1047-1051 — the fade-out, TRUE at the 19th call.
                if self.blend_layer(BlendLayer::Main0, true) {
                    self.print_fade_state = PrintFadeState::Cleanup;
                }
                false
            }
            PrintFadeState::Cleanup => {
                // :1052-1057 — RemoveWindow +
                // BgClearTilemapBufferAndCommit(MAIN_0): the
                // fullscreen and the dialog both drop.
                self.clear_layer(true, 0);
                self.print_fade_state = PrintFadeState::Print;
                true
            }
        }
    }

    /// `OakSpeech_PrintMultichoiceMenu` (`oaks_speech.c:1119-1147`)
    /// — the choice buttons, each printed centered and committed the
    /// same tick. `data->numMultichoiceOptions` is the printed vec's
    /// length — the count FreeWindows walks.
    fn print_multichoice_menu(&mut self, input: Input, msg_ids: &[usize; 3], num_choices: usize) {
        // y = 4 for two choices, 8 for three (:1131-1135) — the
        // only counts the callers pass.
        let y = if num_choices == 2 { 4 } else { 8 };
        let mut windows = Vec::with_capacity(num_choices);
        for i in 0..num_choices {
            // ReadMsgDataIntoString (:1139) — raw, no placeholders.
            let message = self.messages[msg_ids[i]].clone();
            // FontID_String_GetWidth(0, string, 0) (:1140).
            let x = self.font0.string_width(message.units(), 0);
            let (left, top, width, height, palette, base_tile) =
                MULTICHOICE_TEMPLATES[num_choices - 2][i];
            let mut window = add_window(0, left, top, width, height, palette, base_tile);
            // FillWindowPixelRect(0, 0, 0, 0xC0, 0xC0) (:1142).
            fill_window_pixels(&mut window, 0);
            // Font 4, instant, color (15,1,0), centered on the button
            // (:1143).
            let mut printer = TextPrinter::new(
                4,
                self.font4_asset,
                self.focus_asset,
                message,
                ((u32::from(width) * 8 - x) / 2) as u16,
                y,
                TextColor::new(15, 1, 0),
                TEXT_SPEED_INSTANT,
                0,
            );
            printer.render_instant(&self.font4, &mut window, input, &mut self.flags);
            // CopyWindowToVram (:1144) — committed; the vec is the
            // frame's window list from the next tail.
            windows.push(window);
        }
        self.multichoice_windows = windows;
    }

    /// `OakSpeech_FreeWindows` (`oaks_speech.c:1149-1154`) — the
    /// multichoice buttons' RemoveWindows (no-ops on the pixels —
    /// the windows stay until a SUB_0 clear) and the MAIN_0
    /// tilemap's clear.
    fn free_windows(&mut self) {
        self.clear_layer(true, 0);
    }

    /// `OakSpeech_InitMultichoiceMenuWithFrameFlash`
    /// (`oaks_speech.c:1261-1268`) — the cursor flash's frame on SUB
    /// BG1: the menu's NSCR from `sMultichoiceMenuParam`, NCGR
    /// member 42, and the flash timers re-armed. `unk_0` is
    /// write-only; `cursorPos`, `inPadMode`, and `pressDelay` keep
    /// their values.
    fn init_multichoice_menu_with_frame_flash(&mut self, menu_id: usize) {
        self.menu.num_options = MULTICHOICE_OPTIONS[menu_id];
        self.menu.flash_delay = 0;
        self.menu.flash_frames_per = 16;
        let screen = self.multichoice_screens[menu_id];
        self.load_screen(false, 1, screen);
        self.frame.sub.char_blocks[usize::from(char_base(1))].push(TilePlacement {
            asset: self.multichoice_flash_char,
            tile: 0,
        });
    }

    /// `OakSpeech_MultichoiceMenuHandleInputVertical`
    /// (`oaks_speech.c:1270-1338`) — the tutorial and the
    /// gender-confirm menus. Returns the chosen index once the
    /// 20-frame arm lands, `None` otherwise; the SEs are deferred.
    fn multichoice_input(
        &mut self,
        input: Input,
        new_keys: Keys,
        touch_new: bool,
        menu_id: usize,
    ) -> Option<usize> {
        let mut ret = None;
        if self.menu.press_delay != 0 {
            // :1277-1283 — the arm counts past 20, then fires the
            // cursor's choice.
            self.menu.press_delay += 1;
            if self.menu.press_delay > 20 {
                self.menu.press_delay = 0;
                ret = Some(usize::from(self.menu.cursor_pos));
            }
        } else if touch_new {
            // :1283-1293 — a touch on a choice arms it directly.
            if let Some(hitbox) = input
                .touch
                .as_ref()
                .and_then(|touch| find_rect_at_touch(touch, MULTICHOICE_HITBOXES[menu_id]))
            {
                self.init_multichoice_menu_with_frame_flash(menu_id);
                self.frame.sub.bgs[1].enabled = true;
                self.menu.cursor_pos = hitbox as u8;
                self.frame.sub.bgs[1].scroll_y =
                    MULTICHOICE_CURSOR_Y[menu_id][hitbox];
                self.menu.press_delay = 1;
                self.menu.flash_frames_per = 2;
            }
        } else if !self.menu.in_pad_mode {
            // :1294-1300 — the first pad press starts pad mode.
            if new_keys.any(key::A | key::B | key::UP | key::DOWN) {
                self.init_multichoice_menu_with_frame_flash(menu_id);
                self.frame.sub.bgs[1].enabled = true;
                self.menu.in_pad_mode = true;
            }
        } else if new_keys.any(key::UP) {
            // :1301-1306.
            if self.menu.cursor_pos != 0 {
                self.menu.cursor_pos -= 1;
                self.frame.sub.bgs[1].scroll_y =
                    MULTICHOICE_CURSOR_Y[menu_id][usize::from(self.menu.cursor_pos)];
            }
        } else if new_keys.any(key::DOWN) {
            // :1307-1313.
            if self.menu.cursor_pos + 1 != self.menu.num_options {
                self.menu.cursor_pos += 1;
                self.frame.sub.bgs[1].scroll_y =
                    MULTICHOICE_CURSOR_Y[menu_id][usize::from(self.menu.cursor_pos)];
            }
        } else if new_keys.any(key::A) {
            // :1313-1316 — arm the choice.
            self.menu.press_delay = 1;
            self.menu.flash_frames_per = 2;
        } else if new_keys.any(key::B) {
            // :1317-1323 — B arms the last choice.
            self.menu.cursor_pos = self.menu.num_options - 1;
            self.menu.press_delay = 1;
            self.menu.flash_frames_per = 2;
            self.frame.sub.bgs[1].scroll_y =
                MULTICHOICE_CURSOR_Y[menu_id][usize::from(self.menu.cursor_pos)];
        }
        // The flash tail (:1324-1335) — every armed call flips SUB_1
        // every flashFramesPer frames.
        if self.menu.press_delay != 0 {
            self.menu.flash_delay += 1;
            if self.menu.flash_delay > self.menu.flash_frames_per {
                self.menu.flash_state ^= 1;
                self.menu.flash_delay = 0;
                self.frame.sub.bgs[1].enabled = self.menu.flash_state != 0;
            }
        }
        ret
    }

    /// `OakSpeech_BlinkHighlightedGenderFrame` / Stop (1367-1392).
    /// Each panel uses two BG colors: its pulsing fill and outline.
    fn gender_frame_highlight(&mut self, reset: bool, selected: bool) {
        let brightness = if reset {
            self.gender_blink_angle = 0;
            0
        } else {
            let value = GENDER_BLINK_BRIGHTNESS[usize::from(self.gender_blink_angle % 360 / 2)];
            self.gender_blink_angle = self.gender_blink_angle.wrapping_add(10);
            i16::from(value)
        };
        self.frame.sub.palette_overrides.retain(|&(index, _)| !(12..16).contains(&index));
        for gender in 0..2 {
            let active = selected && gender == self.menu.cursor_pos;
            let mut fill = 0;
            for shift in [0, 5, 10] {
                let channel = ((self.gender_frame_color >> shift) & 31) as i16;
                fill |= ((channel + if active { brightness } else { 0 }).clamp(0, 31) as u16) << shift;
            }
            let outline = if active { 31 | (7 << 5) | (7 << 10) } else { 27 | (28 << 5) | (28 << 10) };
            let offset = 12 + u16::from(gender) * 2;
            self.frame.sub.palette_overrides.extend([(offset, fill), (offset + 1, outline)]);
        }
    }

    /// `OakSpeech_GenderSelectHandleInput` (`oaks_speech.c:1394-1442`).
    fn gender_input(&mut self, input: Input, new_keys: Keys, touch_new: bool) -> bool {
        if touch_new {
            // :1401-1411 — a touch picks its gender outright.
            if let Some(hitbox) = input
                .touch
                .as_ref()
                .and_then(|touch| find_rect_at_touch(touch, &GENDER_HITBOXES))
            {
                self.menu.cursor_pos = hitbox as u8;
                self.menu.press_delay = 1;
                self.menu.flash_frames_per = 2;
                self.gender_frame_highlight(true, true);
                self.last_chosen_gender = hitbox as u8;
                return true;
            }
            false
        } else if !self.menu.in_pad_mode {
            // :1412-1417 — the first A/LEFT/RIGHT starts pad mode.
            if new_keys.any(key::A | key::LEFT | key::RIGHT) {
                self.menu.in_pad_mode = true;
                self.gender_frame_highlight(true, true);
            }
            false
        } else {
            // The original updates the palette before moving the cursor.
            self.gender_frame_highlight(false, true);
            if new_keys.any(key::LEFT) {
                if self.menu.cursor_pos != 0 {
                    self.menu.cursor_pos -= 1;
                }
            } else if new_keys.any(key::RIGHT) {
                if self.menu.cursor_pos + 1 != self.menu.num_options {
                    self.menu.cursor_pos += 1;
                }
            } else if new_keys.any(key::A) {
                self.menu.in_pad_mode = false;
                self.menu.press_delay = 1;
                self.menu.flash_frames_per = 2;
                self.gender_frame_highlight(true, true);
                self.last_chosen_gender = self.menu.cursor_pos;
                return true;
            }
            false
        }
    }

    /// `OakSpeech_PlayerPicShrinkAnim` (`oaks_speech.c:1449-1475`)
    /// — one frame per nine calls (the delay re-arms at 8), the
    /// table's `0xFF` sentinel reporting TRUE at call 37.
    /// `InitPlayerPicShrinkAnim` (:1444-1447) is the field resets at
    /// state 124.
    fn player_pic_shrink_anim(&mut self) -> bool {
        if self.pic_anim_delay != 0 {
            self.pic_anim_delay -= 1;
        } else {
            self.pic_anim_step += 1;
            self.pic_anim_delay = 8;
        }
        let table = if self.player_gender == 0 {
            &self.shrink_male
        } else {
            &self.shrink_female
        };
        match table[usize::from(self.pic_anim_step)] {
            // gfxId == 0xFF (:1468).
            None => true,
            Some(asset) => {
                // LoadCharData(MAIN_1, gfxId) (:1471) — the pic's own
                // char block, tile 0.
                self.frame.main.char_blocks[usize::from(char_base(1))].push(TilePlacement {
                    asset,
                    tile: 0,
                });
                false
            }
        }
    }

    /// `OakSpeech_TranslateOakPicHorizontally`
    /// (`oaks_speech.c:1497-1524`) — the Oak pic's ±52px slide on
    /// MAIN_1, 2px per call, TRUE at call 28. `direction` 0 slides
    /// right (0 → −52), 1 back (−52 → 0).
    fn translate_oak_pic(&mut self, direction: usize) -> bool {
        let [start, target, step] = OAK_TRANSLATION_PARAM[direction];
        match self.oak_translate_state {
            0 => {
                // :1499-1502 — the start position, no commit.
                self.oak_translate_state = 1;
                self.oak_translate_pos = start;
                false
            }
            1 => {
                // :1503-1517 — step, clamp at the target, and commit
                // the 9-bit scroll register.
                self.oak_translate_pos += step;
                if step > 0 {
                    if self.oak_translate_pos >= target {
                        self.oak_translate_pos = target;
                        self.oak_translate_state = 2;
                    }
                } else if self.oak_translate_pos <= target {
                    self.oak_translate_pos = target;
                    self.oak_translate_state = 2;
                }
                self.frame.main.bgs[1].scroll_x =
                    (self.oak_translate_pos as u16) & 0x1FF;
                false
            }
            _ => {
                // :1518-1520.
                self.oak_translate_state = 0;
                true
            }
        }
    }

    /// `OakSpeech_CreateMultichoiceYesNoMenu`
    /// (`oaks_speech.c:2156-2162`) — the post-naming restore's
    /// menu: the male arm's SUB_2 backdrop, the gender-select SUB_3
    /// layout, the two choice buttons, and the unchosen frame's
    /// highlight.
    fn create_multichoice_yesno_menu(&mut self, input: Input) {
        self.load_sub2_menu_backdrop(1);
        self.load_sub3_backdrop(4);
        self.print_multichoice_menu(input, &[47, 48, 0], 2);
        // FillBgTilemapRect(SUB_3, 1, 16*(gender^1), 0, 16, 23, 0) +
        // the commit (:2160-2161).
        self.frame.sub.tilemap_edits.push(TilemapEdit::Fill {
            bg: 3,
            tile: 1,
            left: 16 * (self.player_gender ^ 1),
            top: 0,
            width: 16,
            height: 23,
            palette: 0,
        });
    }

    /// `OakSpeech_ShowTutorialTouchMsg` (`oaks_speech.c:2169-2180`)
    /// — msg 60, raw, printed into the touch window and committed.
    fn show_tutorial_touch_msg(&mut self, input: Input) {
        let (left, top, width, height, palette, base_tile) = TOUCH_TEMPLATE;
        let mut window = add_window(0, left, top, width, height, palette, base_tile);
        // FillWindowPixelBuffer(window, 0) (:2175).
        fill_window_pixels(&mut window, 0);
        let mut printer = TextPrinter::new(
            4,
            self.font4_asset,
            self.focus_asset,
            self.messages[60].clone(),
            0,
            0,
            TextColor::new(15, 1, 0),
            TEXT_SPEED_INSTANT,
            0,
        );
        printer.render_instant(&self.font4, &mut window, input, &mut self.flags);
        // CopyWindowToVram + RemoveWindow (:2177-2178) — committed,
        // the content outliving the window struct.
        self.touch_window = Some(window);
    }

    /// `OakSpeech_HideTutorialTouchMsg`
    /// (`oaks_speech.c:2182-2188`) — the same window re-printed
    /// empty: the blank pixels commit over the message.
    fn hide_tutorial_touch_msg(&mut self) {
        if let Some(window) = self.touch_window.as_mut() {
            fill_window_pixels(window, 0);
        }
    }

    /// `OakSpeech_TouchToAdvanceButtonAction`
    /// (`oaks_speech.c:2190-2213`) — the sprite's draw flag and the
    /// touch window's show/hide; PRESS/RELEASE set the anim number
    /// the depressed check reads. The GF_ASSERTs enforce the
    /// callers' alternation, which the state machine keeps.
    fn touch_button_action(&mut self, input: Input, action: u8) {
        match action {
            TOUCHTOADVANCE_HIDE => {
                self.hide_tutorial_touch_msg();
                self.touch_active = false;
            }
            TOUCHTOADVANCE_SHOW => {
                self.show_tutorial_touch_msg(input);
                self.touch_active = true;
            }
            TOUCHTOADVANCE_PRESS => self.touch_anim = 1,
            _ => self.touch_anim = 0,
        }
    }

    /// `OakSpeech_HandleTouchToAdvanceButton`
    /// (`oaks_speech.c:2224-2237`) — pState 1's pre-task pass: a held
    /// touch inside the button's rect synthesizes the A the next
    /// keypad read merges (`system.c:245-270`).
    fn handle_touch_to_advance_button(&mut self, input: Input, touch_new: bool) {
        if !self.touch_active {
            return;
        }
        // FindRectAtTouchHeld(ov53_021E8510) — the current sample,
        // hitbox 0 the button's rect.
        let hitbox = input
            .touch
            .as_ref()
            .and_then(|touch| find_rect_at_touch(touch, &[TOUCH_ADVANCE_RECT]));
        if hitbox == Some(0) && touch_new {
            // gSystem.simulatedInputs = TRUE — bit 0, PAD_BUTTON_A.
            self.simulated_a = true;
            self.touch_button_action(input, TOUCHTOADVANCE_PRESS);
        } else if hitbox == Some(0) && self.touch_anim == 1 {
            // IsTouchToAdvanceButtonDepressed — held through.
            self.simulated_a = true;
        } else {
            self.touch_button_action(input, TOUCHTOADVANCE_RELEASE);
        }
    }

    /// `OakSpeechYesNo_SetBackgroundPalette`
    /// (`oaks_speech_yesnomenu.c:54-69`) — the menu backdrop's
    /// palette, char, and screen onto the background layer (SUB_2),
    /// the whole-map bank switch, and the plane off.
    fn yesno_set_background_palette(&mut self, palette: u8) {
        // GXLoadPal member 0 to SUB_BG at byte offset 32·palette —
        // color offset 16·palette, 0x20 bytes (16 colors) (:62).
        self.frame.sub.palette_loads.push(PaletteLoad {
            asset: self.yesno_pal,
            offset: yesno_palette_offset(palette),
            colors: 0x20 / 2,
        });
        // LoadCharData member 1 (:63) — SUB_2's char block, tile 0.
        self.frame.sub.char_blocks[usize::from(char_base(2))].push(TilePlacement {
            asset: self.yesno_char,
            tile: 0,
        });
        // LoadScrnData member 10 (:64).
        let screen = self.yesno_screen;
        self.load_screen(false, 2, screen);
        // BgTilemapRectChangePalette(0, 0, 32, 24, palette) + commit
        // (:65-66).
        self.fill_bg_layer_with_palette(false, 2, palette);
        // ToggleBgLayer(SUB_2, OFF) (:67).
        self.frame.sub.bgs[2].enabled = false;
    }

    /// `OakSpeechYesNo_Start` (`oaks_speech_yesnomenu.c:71-80`) —
    /// both buttons printed through the menu's own MessageFormat
    /// (whose fields nothing ever fills), the state and result
    /// armed, and both planes on.
    fn yesno_start(&mut self, input: Input, msg_yes: usize, msg_no: usize) {
        for (i, msg) in [msg_yes, msg_no].into_iter().enumerate() {
            // PrintMessageOnWindow (:119-127): ReadMsgData_
            // ExpandPlaceholders against the menu's own format.
            let expanded = self
                .yesno_format
                .expand_placeholders(self.messages[msg].units())
                .expect("the yes/no messages' placeholders resolve");
            let window = &mut self.yesno_windows[i];
            // FillWindowPixelBuffer(window, 0) (:122).
            fill_window_pixels(window, 0);
            let mut printer = TextPrinter::new(
                4,
                self.font4_asset,
                self.focus_asset,
                expanded,
                0,
                0,
                TextColor::new(15, 1, 0),
                TEXT_SPEED_INSTANT,
                0,
            );
            printer.render_instant(&self.font4, window, input, &mut self.flags);
            // CopyWindowToVram (:124).
        }
        self.yesno_state = 0;
        self.yesno_result = YESNO_RESPONSE_YES;
        // SetCursorSpritePos (:76) and the draw flag (:79) — the
        // cursor sprite, deferred.
        // ToggleBgLayer(SUB_2/SUB_0, ON) (:77-78) — the windows are
        // mapped from here.
        self.frame.sub.bgs[2].enabled = true;
        self.frame.sub.bgs[0].enabled = true;
        self.yesno_mapped = true;
    }

    /// `OakSpeechYesNo_Main` (`oaks_speech_yesnomenu.c:82-104`) —
    /// 0 while the menu runs, else the result
    /// (`YESNORESPONSE_YES` 1, `YESNORESPONSE_NO` 2). The cursor
    /// sprite's anim wait (:229-232) is immediate — no sprite model —
    /// so the result leaves the tick after the confirming input.
    fn yesno_main(&mut self, input: Input, new_keys: Keys, touch_new: bool) -> u8 {
        match self.yesno_state {
            0 => {
                if self.yesno_handle_input(input, new_keys, touch_new) {
                    self.yesno_state = 1;
                }
                0
            }
            _ => {
                // WaitCursorSpriteAnim — immediate TRUE; both planes
                // off (:92-93), the window contents staying until a
                // SUB_0 clear.
                self.frame.sub.bgs[2].enabled = false;
                self.frame.sub.bgs[0].enabled = false;
                self.yesno_result
            }
        }
    }

    /// `OakSpeechYesNo_HandleInput`
    /// (`oaks_speech_yesnomenu.c:181-227`) — the touch rects first,
    /// then the pad. The cursor sprite's moves and every SE are
    /// deferred.
    fn yesno_handle_input(&mut self, input: Input, new_keys: Keys, touch_new: bool) -> bool {
        let hitbox = if touch_new {
            input
                .touch
                .as_ref()
                .and_then(|touch| find_rect_at_touch(touch, &YESNO_HITBOXES))
        } else {
            None
        };
        let mut ret = false;
        if let Some(hitbox) = hitbox {
            // :184-198 — the touch picks its button.
            if hitbox == 0 {
                self.yesno_result = YESNO_RESPONSE_YES;
            } else {
                self.yesno_result = YESNO_RESPONSE_NO;
            }
            ret = true;
        } else if new_keys.any(key::UP) {
            // :199-204.
            if self.yesno_result != YESNO_RESPONSE_YES {
                self.yesno_result = YESNO_RESPONSE_YES;
            }
        } else if new_keys.any(key::DOWN) {
            // :205-210.
            if self.yesno_result != YESNO_RESPONSE_NO {
                self.yesno_result = YESNO_RESPONSE_NO;
            }
        } else if new_keys.any(key::A) {
            // :211-216 — A confirms the standing result.
            ret = true;
        } else if new_keys.any(key::B) {
            // :217-220 — B is a NO.
            ret = true;
            self.yesno_result = YESNO_RESPONSE_NO;
        }
        // On ret: SetCursorSpritePos + Sprite_SetAnimCtrlSeq(3) +
        // PlaySE (:221-225) — deferred.
        ret
    }

    /// `OakSpeech_DoMainTask` (`oaks_speech.c:1526-2154`) — the
    /// speech's whole chain, one case exec per tick, the arms in the
    /// switch's own order. Pret's enum skips 10, 25–26, 30–33, 73–92,
    /// 104–109, 112–119, and 122; TRUE leaves only at the shrink
    /// anim's end (:2146-2150).
    fn do_main_task(&mut self, input: Input, new_keys: Keys, touch_new: bool) -> bool {
        match self.main_state {
            MainState::StartTutorialMusic => {
                // :1530-1536 — SUB_2 off, then the BGM change
                // (Sound_SetSceneAndPlayBGM + StopBGM + PlayBGM,
                // :1532-1534) — deferred with audio.
                self.frame.sub.bgs[2].enabled = false;
                self.main_state = MainState::FadeInTutorialMenu;
            }
            MainState::FadeInTutorialMenu => {
                // :1537-1547 — the menu backdrop, the tutorial art,
                // four planes, and the 6×1 fade-in.
                self.load_sub2_menu_backdrop(0);
                self.set_button_tutorial_screen_layout(1);
                self.load_sub3_backdrop(0);
                self.frame.main.bgs[3].enabled = true;
                self.frame.main.bgs[0].enabled = true;
                self.frame.sub.bgs[0].enabled = true;
                self.frame.sub.bgs[3].enabled = true;
                self.fade
                    .begin_with_screens(FadeScreens::Both, FadeType::BrightnessIn, FadeColor::Black, 6, 1);
                self.main_state = MainState::WaitFadeInTutorialMenu;
            }
            MainState::WaitFadeInTutorialMenu => {
                // :1548-1552 — the fade gate before WaitFrames' 40.
                if self.fade.is_finished() && self.wait_frames(40) {
                    self.main_state = MainState::PrintTutorialMenuMessages;
                }
            }
            MainState::PrintTutorialMenuMessages => {
                // :1553-1561 — the fullscreen text (kind 2, the early
                // TRUE), then the three-choice menu printed the same
                // tick.
                if self.print_and_fade(input, new_keys, 7, 2) {
                    self.frame.sub.bgs[2].enabled = true;
                    self.main_state = MainState::TutorialMenuHandleInput;
                    self.print_multichoice_menu(input, &[44, 45, 46], 3);
                    self.menu.in_pad_mode = false;
                    self.menu.cursor_pos = 0;
                }
            }
            MainState::TutorialMenuHandleInput => {
                // :1562-1566.
                if self
                    .multichoice_input(input, new_keys, touch_new, 0)
                    .is_some()
                {
                    self.main_state = MainState::FadeOutTutorialMenu;
                }
            }
            MainState::FadeOutTutorialMenu => {
                // :1567-1575 — the same kind-2 call finishing its
                // blend-out, then the windows freed and three planes
                // off.
                if self.print_and_fade(input, new_keys, 7, 2) {
                    self.free_windows();
                    self.frame.sub.bgs[0].enabled = false;
                    self.frame.sub.bgs[2].enabled = false;
                    self.frame.sub.bgs[1].enabled = false;
                    self.main_state = MainState::FadeOutTutorialMenuBgs;
                }
            }
            MainState::FadeOutTutorialMenuBgs => {
                // :1576-1579.
                self.fade.begin_with_screens(
                    FadeScreens::Both,
                    FadeType::BrightnessOut,
                    FadeColor::Black,
                    6,
                    1,
                );
                self.main_state = MainState::WaitFadeOutTutorialMenuBgs;
            }
            MainState::WaitFadeOutTutorialMenuBgs => {
                // :1580-1594 — the choice routes the three paths.
                if self.fade.is_finished() {
                    self.main_state = match self.menu.cursor_pos {
                        // CONTROL INFO.
                        0 => MainState::FadeInControlInfo,
                        // ADVENTURE INFO.
                        1 => MainState::FadeInAdventureInfo,
                        // NO INFO NEEDED.
                        _ => MainState::NoInfoNeededFadeIn,
                    };
                }
            }
            MainState::FadeInControlInfo => {
                // :1597-1606 — SUB_0 cleared, the control-info art,
                // the touch button live, and the fade-in.
                self.clear_layer(false, 0);
                self.set_button_tutorial_screen_layout(1);
                self.load_sub3_backdrop(1);
                self.touch_button_action(input, TOUCHTOADVANCE_SHOW);
                self.frame.sub.bgs[0].enabled = true;
                self.frame.sub.bgs[3].enabled = true;
                self.fade
                    .begin_with_screens(FadeScreens::Both, FadeType::BrightnessIn, FadeColor::Black, 6, 1);
                self.main_state = MainState::WaitFadeInControlInfo;
            }
            MainState::WaitFadeInControlInfo => {
                // :1607-1611.
                if self.fade.is_finished() {
                    self.main_state = MainState::ControlInfo1;
                }
            }
            MainState::ControlInfo1 => {
                // :1612-1616 — the first four card messages, kind 0.
                if self.print_and_fade(input, new_keys, 9, 0) {
                    self.main_state = MainState::ControlInfo2;
                }
            }
            MainState::ControlInfo2 => {
                // :1617-1621.
                if self.print_and_fade(input, new_keys, 10, 0) {
                    self.main_state = MainState::ControlInfo3;
                }
            }
            MainState::ControlInfo3 => {
                // :1622-1626.
                if self.print_and_fade(input, new_keys, 11, 0) {
                    self.main_state = MainState::ControlInfo4;
                }
            }
            MainState::ControlInfo4 => {
                // :1627-1631.
                if self.print_and_fade(input, new_keys, 12, 0) {
                    self.main_state = MainState::ControlInfo5;
                }
            }
            MainState::ControlInfo5 => {
                // :1632-1637 — msg 23, kind 2: the early TRUE turns
                // MAIN_0 on for the dialog messages that follow.
                if self.print_and_fade(input, new_keys, 23, 2) {
                    self.frame.main.bgs[0].enabled = true;
                    self.main_state = MainState::ControlInfo6;
                }
            }
            MainState::ControlInfo6 => {
                // :1638-1642 — the first framed dialog, mode 0 (the
                // A-release wait).
                if self.print_dialog_msg(input, new_keys, 25, 0) {
                    self.main_state = MainState::ControlInfo7;
                }
            }
            MainState::ControlInfo7 => {
                // :1643-1648 — msg 13 never prints: the machine still
                // sits at the blend-out, which this call's first ticks
                // finish (the genuine C skip).
                if self.print_and_fade(input, new_keys, 13, 0) {
                    self.set_button_tutorial_screen_layout(2);
                    self.main_state = MainState::ControlInfo8;
                }
            }
            MainState::ControlInfo8 => {
                // :1649-1654 — kind 3: the left edge shifted 4 tiles.
                if self.print_and_fade(input, new_keys, 14, 3) {
                    self.set_button_tutorial_screen_layout(1);
                    self.main_state = MainState::ControlInfo9;
                }
            }
            MainState::ControlInfo9 => {
                // :1655-1660.
                if self.print_and_fade(input, new_keys, 15, 0) {
                    self.set_button_tutorial_screen_layout(3);
                    self.main_state = MainState::ControlInfo10;
                }
            }
            MainState::ControlInfo10 => {
                // :1661-1666.
                if self.print_and_fade(input, new_keys, 16, 3) {
                    self.set_button_tutorial_screen_layout(4);
                    self.main_state = MainState::ControlInfo11;
                }
            }
            MainState::ControlInfo11 => {
                // :1667-1673 — the state, the plane, and the layout
                // all in the TRUE arm, the same tick.
                if self.print_and_fade(input, new_keys, 17, 3) {
                    self.main_state = MainState::ControlInfo23;
                    self.frame.main.bgs[0].enabled = true;
                    self.set_button_tutorial_screen_layout(1);
                }
            }
            MainState::ControlInfo23 => {
                // :1674-1678 — the last control message, mode 1.
                if self.print_dialog_msg(input, new_keys, 26, 1) {
                    self.main_state = MainState::AskUnderstood;
                }
            }
            MainState::AskUnderstood => {
                // :1679-1685 — the touch button hidden, SUB_0
                // cleared, and the yes/no menu started.
                self.touch_button_action(input, TOUCHTOADVANCE_HIDE);
                self.clear_layer(false, 0);
                self.yesno_set_background_palette(7);
                self.yesno_start(input, 61, 62);
                self.main_state = MainState::AskUnderstoodHandleYesNo;
            }
            MainState::AskUnderstoodHandleYesNo => {
                // :1686-1695.
                match self.yesno_main(input, new_keys, touch_new) {
                    YESNO_RESPONSE_YES => self.main_state = MainState::UnderstoodYes,
                    YESNO_RESPONSE_NO => self.main_state = MainState::UnderstoodNo,
                    _ => {}
                }
            }
            MainState::UnderstoodYes => {
                // :1696-1701 — the menu's blend-out, then the fade.
                if self.blend_layer(BlendLayer::Sub2, true) {
                    self.fade.begin_with_screens(
                        FadeScreens::Both,
                        FadeType::BrightnessOut,
                        FadeColor::Black,
                        6,
                        1,
                    );
                    self.main_state = MainState::UnderstoodYesWaitFade;
                }
            }
            MainState::UnderstoodYesWaitFade => {
                // :1702-1707 — back to the tutorial menu.
                if self.fade.is_finished() {
                    self.clear_bg_layer0_top_bottom();
                    self.main_state = MainState::FadeInTutorialMenu;
                }
            }
            MainState::UnderstoodNo => {
                // :1708-1715 — the blend-out, the button released,
                // and back into the control info.
                if self.blend_layer(BlendLayer::Sub2, true) {
                    self.touch_button_action(input, TOUCHTOADVANCE_RELEASE);
                    self.frame.sub.bgs[2].enabled = false;
                    self.clear_bg_layer0_top_bottom();
                    self.main_state = MainState::FadeInControlInfo;
                }
            }
            MainState::FadeInAdventureInfo => {
                // :1718-1727 — the adventure art, the touch button,
                // and the fade-in.
                self.set_button_tutorial_screen_layout(5);
                self.load_sub3_backdrop(2);
                self.clear_bg_layer0_top_bottom();
                self.touch_button_action(input, TOUCHTOADVANCE_SHOW);
                self.frame.sub.bgs[0].enabled = true;
                self.frame.sub.bgs[3].enabled = true;
                self.fade
                    .begin_with_screens(FadeScreens::Both, FadeType::BrightnessIn, FadeColor::Black, 6, 1);
                self.main_state = MainState::WaitFadeInAdventureInfo;
            }
            MainState::WaitFadeInAdventureInfo => {
                // :1728-1732.
                if self.fade.is_finished() {
                    self.main_state = MainState::AdventureInfo1;
                }
            }
            MainState::AdventureInfo1 => {
                // :1733-1737 — the six adventure messages, kind 1.
                if self.print_and_fade(input, new_keys, 28, 1) {
                    self.main_state = MainState::AdventureInfo2;
                }
            }
            MainState::AdventureInfo2 => {
                // :1738-1742.
                if self.print_and_fade(input, new_keys, 29, 1) {
                    self.main_state = MainState::AdventureInfo3;
                }
            }
            MainState::AdventureInfo3 => {
                // :1743-1747.
                if self.print_and_fade(input, new_keys, 30, 1) {
                    self.main_state = MainState::AdventureInfo4;
                }
            }
            MainState::AdventureInfo4 => {
                // :1748-1752.
                if self.print_and_fade(input, new_keys, 31, 1) {
                    self.main_state = MainState::AdventureInfo5;
                }
            }
            MainState::AdventureInfo5 => {
                // :1753-1757.
                if self.print_and_fade(input, new_keys, 32, 1) {
                    self.main_state = MainState::AdventureInfo6;
                }
            }
            MainState::AdventureInfo6 => {
                // :1758-1762.
                if self.print_and_fade(input, new_keys, 33, 1) {
                    self.main_state = MainState::AdventureInfoFadeOut;
                }
            }
            MainState::AdventureInfoFadeOut => {
                // :1763-1766.
                self.fade.begin_with_screens(
                    FadeScreens::Both,
                    FadeType::BrightnessOut,
                    FadeColor::Black,
                    6,
                    1,
                );
                self.main_state = MainState::WaitFadeOutAdventureInfo;
            }
            MainState::WaitFadeOutAdventureInfo => {
                // :1767-1774 — back to the tutorial menu, MAIN_0 on
                // for the fullscreen text to come.
                if self.fade.is_finished() {
                    self.clear_bg_layer0_top_bottom();
                    self.touch_button_action(input, TOUCHTOADVANCE_HIDE);
                    self.frame.main.bgs[0].enabled = true;
                    self.main_state = MainState::FadeInTutorialMenu;
                }
            }
            MainState::NoInfoNeededFadeIn => {
                // :1777-1788 — the main path: Oak himself. The state
                // lands before the fade begins, both the same tick.
                self.clear_bg_layer0_top_bottom();
                self.touch_button_action(input, TOUCHTOADVANCE_SHOW);
                self.set_button_tutorial_screen_layout(0);
                self.clear_layer(false, 1);
                self.frame.main.bgs[0].enabled = true;
                self.frame.main.bgs[3].enabled = false;
                self.frame.sub.bgs[0].enabled = true;
                self.frame.sub.bgs[3].enabled = true;
                self.main_state = MainState::WaitFadeInNoInfoNeeded;
                self.fade
                    .begin_with_screens(FadeScreens::Both, FadeType::BrightnessIn, FadeColor::Black, 6, 1);
            }
            MainState::WaitFadeInNoInfoNeeded => {
                // :1789-1794 — the fade gate, the 40-frame hold, and
                // the time-of-day message picked.
                if self.fade.is_finished() && self.wait_frames(40) {
                    self.main_state = MainState::PrintTimeOfDayMsg;
                    self.queued_msg = time_of_day_intro_msg(&self.rtc);
                }
            }
            MainState::PrintTimeOfDayMsg => {
                // :1795-1800 — the greeting, then the BGM fade the
                // timer carries.
                if self.print_dialog_msg(input, new_keys, self.queued_msg, 1) {
                    // GF_SndStartFadeOutBGM(0, 6) (:1797) —
                    // DoSoundUpdateFrame's fadeTimer, the tail's
                    // decrement.
                    self.bgm_fade_timer = 6;
                    self.main_state = MainState::ShowOak;
                }
            }
            MainState::ShowOak => {
                // :1801-1811 — the BGM change waits out the fade
                // timer, then the pic, two planes, and the 16-step
                // brightness-in.
                if self.bgm_fade_timer == 0 {
                    // StopBGM(SEQ_GS_STARTING) + PlayBGM(SEQ_GS_STARTING2)
                    // (:1803-1804) — deferred.
                    self.draw_pic(Pic::Oak);
                    self.frame.main.bgs[3].enabled = true;
                    self.frame.main.bgs[1].enabled = true;
                    let v = self.brightness_main.start(16, 0, -16, OAK_FADE_MASK);
                    self.frame.main.blend = blend_brightness(OAK_FADE_MASK, v);
                    self.main_state = MainState::WaitFadeInOak;
                }
            }
            MainState::WaitFadeInOak => {
                // :1812-1816 — the poll reads the transition *not*
                // running (brightness.c:120-136's own inversion).
                if self.brightness_main.is_brightness_transition_active() {
                    self.main_state = MainState::WelcomeToWorld;
                }
            }
            MainState::WelcomeToWorld => {
                // :1817-1821.
                if self.print_dialog_msg(input, new_keys, 6, 1) {
                    self.main_state = MainState::SlideOakRight;
                }
            }
            MainState::SlideOakRight => {
                // :1822-1826.
                if self.translate_oak_pic(0) {
                    self.main_state = MainState::ThisWorldIsInhabited;
                }
            }
            MainState::ThisWorldIsInhabited => {
                // :1827-1833 — the dialog, then the Marill's ball
                // drawn (the sprite, deferred).
                if self.print_dialog_msg(input, new_keys, 34, 1) {
                    self.marill_sprite.sequence = 3;
                    self.marill_sprite.elapsed = 0;
                    self.marill_sprite.palette_bank = 1;
                    self.marill_visible = true;
                    self.main_state = MainState::BallOpeningFlash;
                }
            }
            MainState::BallOpeningFlash => {
                // :1835-1843 — the 30-frame hold, then the two 4-step
                // flash transitions and the ball's SE.
                if self.wait_frames(30) {
                    let v = self.brightness_main.start(4, 0, 16, BALL_FLASH_MAIN_MASK);
                    self.frame.main.blend = blend_brightness(BALL_FLASH_MAIN_MASK, v);
                    let v = self.brightness_sub.start(4, 0, 16, BALL_FLASH_SUB_MASK);
                    self.frame.sub.blend = blend_brightness(BALL_FLASH_SUB_MASK, v);
                    // PlaySE(SEQ_SE_DP_BOWA2) (:1839) — deferred.
                    self.pic_anim_step = 0;
                    self.main_state = MainState::AppearMarill;
                }
            }
            MainState::AppearMarill => {
                // :1844-1852 — both transitions done, the Marill at
                // full OBJ brightness.
                if self.brightness_main.is_brightness_transition_active()
                    && self.brightness_sub.is_brightness_transition_active()
                {
                    self.marill_sprite.sequence = 1;
                    self.marill_sprite.elapsed = 0;
                    self.marill_sprite.palette_bank = 0;
                    self.pic_anim_step = 16;
                    self.frame.main.blend = blend_brightness(plane::OBJ, 16);
                    self.main_state = MainState::MarillCry;
                }
            }
            MainState::MarillCry => {
                // :1853-1863 — Sprite_IsAnimated is the model's
                // immediate FALSE, so the ramp runs a call: 16 execs
                // writing 15 down to 0.
                self.pic_anim_step -= 1;
                self.frame.main.blend =
                    blend_brightness(plane::OBJ, self.pic_anim_step as i16);
                if self.pic_anim_step == 0 {
                    self.marill_sprite.sequence = 2;
                    self.marill_sprite.elapsed = 0;
                    self.main_state = MainState::WaitMarillCry;
                }
            }
            MainState::WaitMarillCry => {
                // :1864-1868.
                if self.wait_frames(40) {
                    self.main_state = MainState::WeLiveAlongside;
                }
            }
            MainState::WeLiveAlongside => {
                // :1869-1873.
                if self.print_dialog_msg(input, new_keys, 35, 1) {
                    self.main_state = MainState::HideMarill;
                }
            }
            MainState::HideMarill => {
                // :1874-1878 — the OBJ pseudo-layer (101) blend-out.
                if self.blend_layer(BlendLayer::MainObj, true) {
                    self.marill_visible = false;
                    self.main_state = MainState::WaitAfterHideMarill;
                }
            }
            MainState::WaitAfterHideMarill => {
                // :1879-1883.
                if self.wait_frames(30) {
                    self.main_state = MainState::SlideOakLeft;
                }
            }
            MainState::SlideOakLeft => {
                // :1884-1888.
                if self.translate_oak_pic(1) {
                    self.main_state = MainState::TellMeAboutYourself;
                }
            }
            MainState::TellMeAboutYourself => {
                // :1889-1895 — the menu fields armed for the gender
                // select.
                if self.print_dialog_msg(input, new_keys, 36, 1) {
                    self.main_state = MainState::AreYouAGender;
                    self.menu.cursor_pos = 0;
                    self.menu.num_options = 2;
                }
            }
            MainState::AreYouAGender => {
                // :1896-1901 — the question, then the SUB-only fade.
                if self.print_dialog_msg(input, new_keys, 37, 1) {
                    self.main_state = MainState::WaitFadeOutToAskGender;
                    self.fade.begin_with_screens(
                        FadeScreens::Sub,
                        FadeType::BrightnessOut,
                        FadeColor::Black,
                        6,
                        1,
                    );
                }
            }
            MainState::WaitFadeOutToAskGender => {
                // :1902-1906.
                if self.fade.is_finished() {
                    self.main_state = MainState::SetupGenderSelectMenu;
                }
            }
            MainState::SetupGenderSelectMenu => {
                // :1907-1915 — the touch button hidden, SUB_0 off,
                // the gender layout, and the SUB fade-in.
                self.touch_button_action(input, TOUCHTOADVANCE_HIDE);
                self.frame.sub.bgs[0].enabled = false;
                self.load_sub3_backdrop(4);
                self.gender_visible = [true; 2];
                self.clear_layer(false, 2);
                self.fade.begin_with_screens(
                    FadeScreens::Sub,
                    FadeType::BrightnessIn,
                    FadeColor::Black,
                    6,
                    1,
                );
                self.main_state = MainState::WaitFadeInGenderSelectMenu;
            }
            MainState::WaitFadeInGenderSelectMenu => {
                // :1916-1921 — the menu opens on the remembered
                // gender.
                if self.fade.is_finished() {
                    self.menu.cursor_pos = self.last_chosen_gender;
                    self.main_state = MainState::GenderSelectMenuHandleInput;
                }
            }
            MainState::GenderSelectMenuHandleInput => {
                // :1922-1927.
                if self.gender_input(input, new_keys, touch_new) {
                    self.main_state = MainState::PrepareAskConfirmGender;
                    self.player_gender = self.menu.cursor_pos;
                }
            }
            MainState::PrepareAskConfirmGender => {
                // :1928-1940 — the chosen frame's highlight, SUB_0
                // cleared, and the gendered message queued.
                self.frame.sub.tilemap_edits.push(TilemapEdit::Fill {
                    bg: 3,
                    tile: 1,
                    left: 16 * (self.menu.cursor_pos ^ 1),
                    top: 0,
                    width: 16,
                    height: 23,
                    palette: 0,
                });
                // BgCommitTilemapBufferToVram(SUB_2/SUB_3) (:1930-1931)
                // — the edits already in the frame's list.
                self.clear_layer(false, 0);
                self.gender_visible = [self.player_gender == 0, self.player_gender == 1];
                self.queued_msg = if self.player_gender == 0 { 38 } else { 39 };
                self.main_state = MainState::AskConfirmGender;
            }
            MainState::AskConfirmGender => {
                // :1941-1945 — the gendered grammar vestige; both
                // messages read the same in English.
                if self.print_dialog_msg(input, new_keys, self.queued_msg, 1) {
                    self.main_state = MainState::ConfirmGenderYesNoInitMenu;
                }
            }
            MainState::ConfirmGenderYesNoInitMenu => {
                // :1946-1952 — the confirm menu printed, the cursor
                // back at YES.
                self.menu.init(1);
                self.load_sub2_menu_backdrop(1);
                self.print_multichoice_menu(input, &[47, 48, 0], 2);
                self.main_state = MainState::ConfirmGenderYesNoHandleInput;
                self.menu.cursor_pos = 0;
            }
            MainState::ConfirmGenderYesNoHandleInput => {
                // :1954-1961 — both planes re-asserted every tick.
                self.frame.sub.bgs[0].enabled = true;
                self.frame.sub.bgs[2].enabled = true;
                if self
                    .multichoice_input(
                        input,
                        new_keys,
                        touch_new,
                        usize::from(self.player_gender) + 1,
                    )
                    .is_some()
                {
                    self.free_windows();
                    self.main_state = MainState::ConfirmGenderYesNoHandleResult;
                }
            }
            MainState::ConfirmGenderYesNoHandleResult => {
                // :1962-1972 — YES walks on; NO fades the SUB screen
                // back to the select.
                match self.menu.cursor_pos {
                    0 => self.main_state = MainState::ConfirmGenderYes,
                    1 => {
                        self.fade.begin_with_screens(
                            FadeScreens::Sub,
                            FadeType::BrightnessOut,
                            FadeColor::Black,
                            6,
                            1,
                        );
                        self.main_state = MainState::ConfirmGenderNoWaitFadeOut;
                    }
                    _ => {}
                }
            }
            MainState::ConfirmGenderNoWaitFadeOut => {
                // :1974-1987 — the select's teardown and rebuild.
                if self.fade.is_finished() {
                    self.frame.sub.bgs[1].enabled = false;
                    self.frame.sub.bgs[2].enabled = false;
                    self.gender_visible = [false; 2];
                    self.load_sub3_backdrop(1);
                    self.clear_layer(false, 0);
                    // ScheduleSetBgPosText(SUB_0, SET_X, 0) (:1982).
                    self.frame.sub.bgs[0].scroll_x = 0;
                    self.gender_frame_highlight(true, false);
                    self.fade.begin_with_screens(
                        FadeScreens::Sub,
                        FadeType::BrightnessIn,
                        FadeColor::Black,
                        6,
                        1,
                    );
                    self.main_state = MainState::ConfirmGenderNoWaitFadeIn;
                }
            }
            MainState::ConfirmGenderNoWaitFadeIn => {
                // :1988-1993 — the button back, then the question
                // again.
                if self.fade.is_finished() {
                    self.touch_button_action(input, TOUCHTOADVANCE_SHOW);
                    self.main_state = MainState::AreYouAGender;
                }
            }
            MainState::ConfirmGenderYes => {
                // :1997-2001 — "So that's your name?"'s lead-in.
                if self.print_dialog_msg(input, new_keys, 40, 1) {
                    self.main_state = MainState::PromptNameDelayBefore;
                }
            }
            MainState::PromptNameDelayBefore => {
                // :2003-2007 — the 40-frame hold before the overlay.
                if self.wait_frames(40) {
                    self.main_state = MainState::PromptNameLaunchNamingScreen;
                }
            }
            MainState::PromptNameLaunchNamingScreen => {
                // :2009-2014 — the naming screen's args set and the
                // overlay launched; the screen itself is Phase 4's
                // next step, so the model's seam is the outer
                // machine's stall-and-deliver.
                self.player_name = GameString::new();
                // namingScreenArgs_Player->playerGenderOrMonSpecies
                // (:2011) — the gender rides the args.
                self.overlay_active = true;
                self.main_state = MainState::PromptNameRestoreGraphicsAfter;
            }
            MainState::PromptNameRestoreGraphicsAfter => {
                // :2016-2036 — the post-naming restore: the planes,
                // the slide reset, the confirm menu, the fade-in, and
                // Oak back on screen — the same tick the outer
                // machine hands control back.
                self.frame.main.bgs[0].enabled = true;
                self.frame.main.bgs[1].enabled = true;
                self.frame.main.bgs[3].enabled = true;
                self.frame.sub.bgs[0].enabled = true;
                self.frame.sub.bgs[3].enabled = true;
                self.frame.sub.bgs[2].enabled = true;
                // BgSetPosTextAndCommit(MAIN_1, SET_X, 0) (:2023).
                self.frame.main.bgs[1].scroll_x = 0;
                self.create_multichoice_yesno_menu(input);
                self.fade
                    .begin_with_screens(FadeScreens::Both, FadeType::BrightnessIn, FadeColor::Black, 6, 1);
                self.main_state = MainState::ConfirmNameYesNoInitMenu;
                self.draw_pic(Pic::Oak);
                self.gender_visible = [self.player_gender == 0, self.player_gender == 1];
                // The gendered grammar vestige (:2029-2030); both
                // messages read the same in English.
                self.queued_msg = if self.player_gender == 0 { 41 } else { 42 };
            }
            MainState::ConfirmNameYesNoInitMenu => {
                // :2038-2043 — "Is your name…?", the menu re-armed in
                // the TRUE arm.
                if self.print_dialog_msg(input, new_keys, self.queued_msg, 1) {
                    self.main_state = MainState::ConfirmNameYesNoHandleInput;
                    self.menu.init(1);
                }
            }
            MainState::ConfirmNameYesNoHandleInput => {
                // :2045-2050 — no per-tick plane writes here, unlike
                // the gender confirm's state 69.
                if self
                    .multichoice_input(
                        input,
                        new_keys,
                        touch_new,
                        usize::from(self.player_gender) + 1,
                    )
                    .is_some()
                {
                    self.free_windows();
                    self.main_state = MainState::ConfirmNameYesNoHandleResult;
                }
            }
            MainState::ConfirmNameYesNoHandleResult => {
                // :2052-2062 — YES walks on; NO falls back into the
                // gender-confirm loop's fade-out state.
                match self.menu.cursor_pos {
                    0 => self.main_state = MainState::ConfirmNameYes,
                    1 => {
                        self.fade.begin_with_screens(
                            FadeScreens::Sub,
                            FadeType::BrightnessOut,
                            FadeColor::Black,
                            6,
                            1,
                        );
                        self.main_state = MainState::ConfirmGenderNoWaitFadeOut;
                    }
                    _ => {}
                }
            }
            MainState::ConfirmNameYes => {
                // :2064-2067 — the SUB-only fade to the send-off.
                self.fade.begin_with_screens(
                    FadeScreens::Sub,
                    FadeType::BrightnessOut,
                    FadeColor::Black,
                    6,
                    1,
                );
                self.main_state = MainState::ConfirmNameYesWaitFadeOut;
            }
            MainState::ConfirmNameYesWaitFadeOut => {
                // :2069-2081 — the menu's teardown and the button
                // back.
                if self.fade.is_finished() {
                    self.frame.sub.bgs[2].enabled = false;
                    self.frame.sub.bgs[1].enabled = false;
                    self.load_sub3_backdrop(1);
                    self.gender_visible = [false; 2];
                    self.clear_layer(false, 0);
                    // ScheduleSetBgPosText(SUB_0, SET_X, 0) (:2076).
                    self.frame.sub.bgs[0].scroll_x = 0;
                    self.touch_button_action(input, TOUCHTOADVANCE_SHOW);
                    self.fade.begin_with_screens(
                        FadeScreens::Sub,
                        FadeType::BrightnessIn,
                        FadeColor::Black,
                        6,
                        1,
                    );
                    self.main_state = MainState::ConfirmNameYesWaitFadeIn;
                }
            }
            MainState::ConfirmNameYesWaitFadeIn => {
                // :2083-2087.
                if self.fade.is_finished() {
                    self.main_state = MainState::YourAdventureUnfolds;
                }
            }
            MainState::YourAdventureUnfolds => {
                // :2089-2093 — the send-off.
                if self.print_dialog_msg(input, new_keys, 43, 1) {
                    self.main_state = MainState::FadeOutFromLastOakMessage;
                }
            }
            MainState::FadeOutFromLastOakMessage => {
                // :2097-2100.
                self.fade.begin_with_screens(
                    FadeScreens::Both,
                    FadeType::BrightnessOut,
                    FadeColor::Black,
                    6,
                    1,
                );
                self.main_state = MainState::WaitFadeOutFromLastOakMessage;
            }
            MainState::WaitFadeOutFromLastOakMessage => {
                // :2102-2106.
                if self.fade.is_finished() {
                    self.main_state = MainState::FadeInToShrinkAnim;
                }
            }
            MainState::FadeInToShrinkAnim => {
                // :2110-2119 — the player pic (Ethan male, Lyra
                // female), the fade-in, MAIN_0 off for the anim.
                if self.player_gender == 0 {
                    self.draw_pic(Pic::Ethan);
                } else {
                    self.draw_pic(Pic::Lyra);
                }
                self.fade
                    .begin_with_screens(FadeScreens::Both, FadeType::BrightnessIn, FadeColor::Black, 6, 1);
                self.main_state = MainState::WaitFadeInToShrinkAnim;
                self.frame.main.bgs[0].enabled = false;
            }
            MainState::WaitFadeInToShrinkAnim => {
                // :2121-2131 — the fade poll's write is dead: no
                // break separates the case from
                // NOP_BEFORE_SHRINK_ANIM, whose body overwrites the
                // state this same tick. One tick here, straight to
                // the anim's init — the fade-in from state 120 still
                // stepping underneath.
                if self.fade.is_finished() {
                    self.main_state = MainState::NopBeforeShrinkAnim;
                }
                self.main_state = MainState::InitShrinkAnimState;
            }
            MainState::NopBeforeShrinkAnim => {
                // :2129-2131 — unreachable through the switch: only
                // the dead write at :2123 names it.
                self.main_state = MainState::InitShrinkAnimState;
            }
            MainState::InitShrinkAnimState => {
                // :2133-2137 — the button hidden and
                // OakSpeech_InitPlayerPicShrinkAnim's resets
                // (:1444-1447).
                self.touch_button_action(input, TOUCHTOADVANCE_HIDE);
                self.pic_anim_step = 0;
                self.pic_anim_delay = 0;
                self.main_state = MainState::DelayBeforeShrinkAnim;
            }
            MainState::DelayBeforeShrinkAnim => {
                // :2139-2144 — the 30-frame hold, the shrink's SE at
                // its end.
                if self.wait_frames(30) {
                    // PlaySE(SEQ_SE_GS_HERO_SHUKUSHOU) (:2141) —
                    // deferred.
                    self.main_state = MainState::RunShrinkAnim;
                }
            }
            MainState::RunShrinkAnim => {
                // :2146-2150 — the only TRUE in the whole task.
                if self.player_pic_shrink_anim() {
                    return true;
                }
            }
        }
        false
    }

    /// `OakSpeech_Main`'s pState 2/3 cleanup (:603-607, :613-617) —
    /// YesNo_Delete, CleanupSpriteEngine (deferred with the sprites),
    /// CleanupMsgPrinter, and `OakSpeech_CleanupBgs`
    /// (`oaks_speech.c:752-770`): the printer dropped and all eight
    /// planes off.
    fn cleanup(&mut self) {
        self.gender_visible = [false; 2];
        self.marill_visible = false;
        self.touch_active = false;
        self.printer = None;
        for bg in &mut self.frame.main.bgs {
            bg.enabled = false;
        }
        for bg in &mut self.frame.sub.bgs {
            bg.enabled = false;
        }
    }
}

impl App for OakSpeech {
    /// One `OakSpeech_Main` call — the pState machine's exec between
    /// the keypad read and the frame's once-per-tick updates.
    fn tick(&mut self, _frame: crate::Frame, input: Input) {
        // The touch-advance button's synthetic A lands in
        // gSystem.simulatedInputs, which the next keypad read ORs in
        // (system.c:245-270) — the merge happens before the edge
        // detect, so the poll below sees it as a press.
        let keys = if self.simulated_a {
            Keys(input.keys.0 | key::A)
        } else {
            input.keys
        };
        self.simulated_a = false;
        let input = Input {
            keys,
            touch: input.touch,
        };
        let new_keys = keys.pressed(self.prev_keys);
        self.prev_keys = keys;
        let touch_new = input.touch.is_some() && !self.prev_touch;
        self.prev_touch = input.touch.is_some();

        // The pState machine (:566-637).
        match self.outer_state {
            OuterState::Init => {
                // pState 0 (:570-589) — the black write, the BG
                // setup, the printer bank reset, the yes/no windows.
                self.init_state();
                self.outer_state = OuterState::Run;
            }
            OuterState::Run => {
                // pState 1 (:590-600) — the touch button's pass
                // before the task, then the task; the overlay check
                // rides the same exec, its fade winning over the
                // exit's when state 95 armed it this tick.
                self.handle_touch_to_advance_button(input, touch_new);
                if self.do_main_task(input, new_keys, touch_new) {
                    self.fade.begin_with_screens(
                        FadeScreens::Both,
                        FadeType::BrightnessOut,
                        FadeColor::Black,
                        6,
                        1,
                    );
                    self.outer_state = OuterState::CleanupExit;
                }
                if self.overlay_active {
                    self.fade.begin_with_screens(
                        FadeScreens::Both,
                        FadeType::BrightnessOut,
                        FadeColor::Black,
                        6,
                        1,
                    );
                    self.outer_state = OuterState::OverlayFade;
                }
            }
            OuterState::CleanupExit => {
                // pState 2 (:601-610) — the cleanup, then ret TRUE.
                if self.fade.is_finished() {
                    self.cleanup();
                    self.done = true;
                }
            }
            OuterState::OverlayFade => {
                // pState 3 (:611-619) — the same cleanup, into the
                // overlay's run.
                if self.fade.is_finished() {
                    self.cleanup();
                    self.outer_state = OuterState::OverlayRun;
                }
            }
            OuterState::OverlayRun => {
                // pState 4 (:621-627) — OverlayManager_Run: the
                // naming screen is Phase 4's next step, so the seam
                // is the delivered result.
                if self.naming_done {
                    self.naming_done = false;
                    self.overlay_active = false;
                    self.outer_state = OuterState::OverlayDone;
                }
            }
            OuterState::OverlayDone => {
                // pState 5 (:628-630) — back to the re-init.
                self.outer_state = OuterState::Init;
            }
        }

        // The frame's once-per-tick updates, the VBlank order the
        // game's main loop keeps.
        // DoAllScreenBrightnessTransitionStep (brightness.c:110-118)
        // — MAIN's static, then SUB's, each writing its engine's
        // blend unit.
        if self.brightness_main.active {
            let v = self.brightness_main.step();
            self.frame.main.blend = blend_brightness(self.brightness_main.surface_mask, v);
        }
        if self.brightness_sub.active {
            let v = self.brightness_sub.step();
            self.frame.sub.blend = blend_brightness(self.brightness_sub.surface_mask, v);
        }
        // HandleFadeUpdateFrame — the master brightness onto the
        // fade's own screens only, the other engine's register left
        // where it was.
        self.fade.update();
        let brightness = self.fade.brightness();
        match self.fade.screens() {
            FadeScreens::Both => {
                self.frame.main.brightness = brightness;
                self.frame.sub.brightness = brightness;
            }
            FadeScreens::Main => self.frame.main.brightness = brightness,
            FadeScreens::Sub => self.frame.sub.brightness = brightness,
        }
        // DoSoundUpdateFrame's fadeTimer (sound.c:100-115) — the BGM
        // fade state 47 polls for 0.
        if self.bgm_fade_timer > 0 {
            self.bgm_fade_timer -= 1;
        }
        // The window lists — the frame's scene composition: the
        // fullscreen text and the dialog on MAIN, the touch message,
        // the multichoice buttons, and the mapped yes/no on SUB.
        let mut main_windows = Vec::new();
        if self.fullscreen_committed {
            if let Some(window) = self.fullscreen.as_ref() {
                main_windows.push(window.clone());
            }
        }
        if let Some(window) = self.dialog.as_ref() {
            main_windows.push(window.clone());
        }
        self.frame.main.windows = main_windows;
        let mut sub_windows = Vec::new();
        if let Some(window) = self.touch_window.as_ref() {
            sub_windows.push(window.clone());
        }
        sub_windows.extend(self.multichoice_windows.iter().cloned());
        if self.yesno_mapped {
            sub_windows.push(self.yesno_windows[0].clone());
            sub_windows.push(self.yesno_windows[1].clone());
        }
        self.frame.sub.windows = sub_windows;
        self.frame.main.sprites.clear();
        self.frame.sub.sprites.clear();
        if self.marill_visible {
            self.frame.main.sprites.push(self.marill_sprite.clone());
            self.marill_sprite.elapsed = self.marill_sprite.elapsed.saturating_add(1);
        }
        for (sprite, visible) in self.gender_sprites.iter_mut().zip(self.gender_visible) {
            if visible { self.frame.sub.sprites.push(sprite.clone()); }
            sprite.elapsed = sprite.elapsed.saturating_add(1);
        }
        if self.touch_active {
            self.touch_sprite.sequence = usize::from(self.touch_anim);
            self.frame.sub.sprites.push(self.touch_sprite.clone());
        }
    }

    fn frame(&self) -> &LogicalFrame {
        &self.frame
    }

    fn next(&self) -> crate::app::ChainNext {
        if self.done {
            ChainNext::Advance
        } else {
            ChainNext::Stay
        }
    }
}

impl OakSpeech {
    /// The state the game machine routes on once
    /// [`App::next`](crate::app::App::next) says `Advance` — the
    /// confirmed gender, for `AfterOakSpeech`'s profile writes.
    #[must_use]
    pub fn player_gender(&self) -> u8 {
        self.player_gender
    }

    /// The naming screen's delivered name — the player profile's
    /// trainer name, `OakSpeech_Exit`'s
    /// `PlayerName_StringToFlat` (:643).
    #[must_use]
    pub fn player_name(&self) -> &GameString {
        &self.player_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_state_discriminants_match_pret() {
        // oaks_speech.c:43-133 — the enum's values, the gaps pret
        // left (10, 25–26, 30–33, 73–92, 104–109, 112–119, 122)
        // staying gaps.
        assert_eq!(MainState::StartTutorialMusic as u8, 0);
        assert_eq!(MainState::WaitFadeInTutorialMenu as u8, 1);
        assert_eq!(MainState::PrintTutorialMenuMessages as u8, 2);
        assert_eq!(MainState::TutorialMenuHandleInput as u8, 3);
        assert_eq!(MainState::FadeOutTutorialMenu as u8, 4);
        assert_eq!(MainState::FadeOutTutorialMenuBgs as u8, 5);
        assert_eq!(MainState::WaitFadeOutTutorialMenuBgs as u8, 6);
        assert_eq!(MainState::FadeInTutorialMenu as u8, 7);
        assert_eq!(MainState::FadeInControlInfo as u8, 8);
        assert_eq!(MainState::WaitFadeInControlInfo as u8, 9);
        assert_eq!(MainState::ControlInfo1 as u8, 11);
        assert_eq!(MainState::ControlInfo2 as u8, 12);
        assert_eq!(MainState::ControlInfo3 as u8, 13);
        assert_eq!(MainState::ControlInfo4 as u8, 14);
        assert_eq!(MainState::ControlInfo5 as u8, 15);
        assert_eq!(MainState::ControlInfo6 as u8, 16);
        assert_eq!(MainState::ControlInfo7 as u8, 17);
        assert_eq!(MainState::ControlInfo8 as u8, 18);
        assert_eq!(MainState::ControlInfo9 as u8, 19);
        assert_eq!(MainState::ControlInfo10 as u8, 20);
        assert_eq!(MainState::ControlInfo11 as u8, 21);
        assert_eq!(MainState::ControlInfo23 as u8, 22);
        assert_eq!(MainState::AskUnderstood as u8, 23);
        assert_eq!(MainState::AskUnderstoodHandleYesNo as u8, 24);
        assert_eq!(MainState::UnderstoodYes as u8, 27);
        assert_eq!(MainState::UnderstoodYesWaitFade as u8, 28);
        assert_eq!(MainState::UnderstoodNo as u8, 29);
        assert_eq!(MainState::FadeInAdventureInfo as u8, 34);
        assert_eq!(MainState::WaitFadeInAdventureInfo as u8, 35);
        assert_eq!(MainState::AdventureInfo1 as u8, 36);
        assert_eq!(MainState::AdventureInfo2 as u8, 37);
        assert_eq!(MainState::AdventureInfo3 as u8, 38);
        assert_eq!(MainState::AdventureInfo4 as u8, 39);
        assert_eq!(MainState::AdventureInfo5 as u8, 40);
        assert_eq!(MainState::AdventureInfo6 as u8, 41);
        assert_eq!(MainState::AdventureInfoFadeOut as u8, 42);
        assert_eq!(MainState::WaitFadeOutAdventureInfo as u8, 43);
        assert_eq!(MainState::NoInfoNeededFadeIn as u8, 44);
        assert_eq!(MainState::WaitFadeInNoInfoNeeded as u8, 45);
        assert_eq!(MainState::PrintTimeOfDayMsg as u8, 46);
        assert_eq!(MainState::ShowOak as u8, 47);
        assert_eq!(MainState::WaitFadeInOak as u8, 48);
        assert_eq!(MainState::WelcomeToWorld as u8, 49);
        assert_eq!(MainState::SlideOakRight as u8, 50);
        assert_eq!(MainState::ThisWorldIsInhabited as u8, 51);
        assert_eq!(MainState::BallOpeningFlash as u8, 52);
        assert_eq!(MainState::AppearMarill as u8, 53);
        assert_eq!(MainState::MarillCry as u8, 54);
        assert_eq!(MainState::WaitMarillCry as u8, 55);
        assert_eq!(MainState::WeLiveAlongside as u8, 56);
        assert_eq!(MainState::HideMarill as u8, 57);
        assert_eq!(MainState::WaitAfterHideMarill as u8, 58);
        assert_eq!(MainState::SlideOakLeft as u8, 59);
        assert_eq!(MainState::TellMeAboutYourself as u8, 60);
        assert_eq!(MainState::AreYouAGender as u8, 61);
        assert_eq!(MainState::WaitFadeOutToAskGender as u8, 62);
        assert_eq!(MainState::SetupGenderSelectMenu as u8, 63);
        assert_eq!(MainState::WaitFadeInGenderSelectMenu as u8, 64);
        assert_eq!(MainState::GenderSelectMenuHandleInput as u8, 65);
        assert_eq!(MainState::PrepareAskConfirmGender as u8, 66);
        assert_eq!(MainState::AskConfirmGender as u8, 67);
        assert_eq!(MainState::ConfirmGenderYesNoInitMenu as u8, 68);
        assert_eq!(MainState::ConfirmGenderYesNoHandleInput as u8, 69);
        assert_eq!(MainState::ConfirmGenderYesNoHandleResult as u8, 70);
        assert_eq!(MainState::ConfirmGenderNoWaitFadeOut as u8, 71);
        assert_eq!(MainState::ConfirmGenderNoWaitFadeIn as u8, 72);
        assert_eq!(MainState::ConfirmGenderYes as u8, 93);
        assert_eq!(MainState::PromptNameDelayBefore as u8, 94);
        assert_eq!(MainState::PromptNameLaunchNamingScreen as u8, 95);
        assert_eq!(MainState::PromptNameRestoreGraphicsAfter as u8, 96);
        assert_eq!(MainState::ConfirmNameYesNoInitMenu as u8, 97);
        assert_eq!(MainState::ConfirmNameYesNoHandleInput as u8, 98);
        assert_eq!(MainState::ConfirmNameYesNoHandleResult as u8, 99);
        assert_eq!(MainState::ConfirmNameYes as u8, 100);
        assert_eq!(MainState::ConfirmNameYesWaitFadeOut as u8, 101);
        assert_eq!(MainState::ConfirmNameYesWaitFadeIn as u8, 102);
        assert_eq!(MainState::YourAdventureUnfolds as u8, 103);
        assert_eq!(MainState::FadeOutFromLastOakMessage as u8, 110);
        assert_eq!(MainState::WaitFadeOutFromLastOakMessage as u8, 111);
        assert_eq!(MainState::FadeInToShrinkAnim as u8, 120);
        assert_eq!(MainState::WaitFadeInToShrinkAnim as u8, 121);
        assert_eq!(MainState::NopBeforeShrinkAnim as u8, 123);
        assert_eq!(MainState::InitShrinkAnimState as u8, 124);
        assert_eq!(MainState::DelayBeforeShrinkAnim as u8, 125);
        assert_eq!(MainState::RunShrinkAnim as u8, 126);
    }

    #[test]
    fn pic_discriminants_match_pret() {
        // oaks_speech.c:36-41 — OAK_SPEECH_PIC_NONE/OAK/ETHAN/LYRA,
        // the Lyra row sitting at 6 between the never-drawn rows.
        assert_eq!(Pic::None as u8, 0);
        assert_eq!(Pic::Oak as u8, 1);
        assert_eq!(Pic::Ethan as u8, 2);
        assert_eq!(Pic::Lyra as u8, 6);
    }

    #[test]
    fn time_of_day_greeting_ranges() {
        // oaks_speech.c:1477-1495 — the hour·100+minute ranges; the
        // boundaries are the range ends, each side inclusive.
        let at = |hour, minute| {
            time_of_day_intro_msg(&RtcDateTime::new(2010, 3, 14, 0, hour, minute, 0))
        };
        assert_eq!(at(4, 0), 1, "the morning's first minute");
        assert_eq!(at(10, 59), 1);
        assert_eq!(at(11, 0), 2);
        assert_eq!(at(12, 0), 2, "the pinned clock's noon");
        assert_eq!(at(15, 59), 2);
        assert_eq!(at(16, 0), 3);
        assert_eq!(at(18, 59), 3);
        assert_eq!(at(19, 0), 4);
        assert_eq!(at(23, 59), 4);
        assert_eq!(at(0, 0), 5);
        assert_eq!(at(3, 59), 5);
    }

    #[test]
    fn touch_rects_are_half_open() {
        // The advance button's rect (144, 191, 168, 255): top/left
        // in, the bottom and right edges out — the main_menu idiom.
        let touch = |x, y| Touch { x, y };
        assert_eq!(
            find_rect_at_touch(&touch(168, 144), &[TOUCH_ADVANCE_RECT]),
            Some(0),
            "the top-left corner is in"
        );
        assert_eq!(
            find_rect_at_touch(&touch(167, 144), &[TOUCH_ADVANCE_RECT]),
            None,
            "left of the rect is out"
        );
        assert_eq!(
            find_rect_at_touch(&touch(168, 143), &[TOUCH_ADVANCE_RECT]),
            None,
            "above the rect is out"
        );
        assert_eq!(
            find_rect_at_touch(&touch(168, 191), &[TOUCH_ADVANCE_RECT]),
            None,
            "the bottom edge is out"
        );
    }

    #[test]
    fn blend_brightness_maps_the_register() {
        // GXx_SetBlendBrightness_: the mask is the first target,
        // the sign picks the effect, and the weight is 5 bits.
        let down = blend_brightness(plane::OBJ, -16);
        assert_eq!(down.effect, BlendEffect::BrightnessDown);
        assert_eq!(down.plane1, plane::OBJ);
        assert_eq!(down.evy, 16);
        let neutral = blend_brightness(plane::BG1 | plane::BG3 | plane::OBJ, 0);
        assert_eq!(neutral.effect, BlendEffect::BrightnessUp);
        assert_eq!(neutral.evy, 0);
    }

    /// One of brightness.c's two statics, cleared as
    /// `ScreenBrightnessData_InitAll` leaves them.
    fn cleared_transition() -> BrightnessTransition {
        BrightnessTransition {
            active: false,
            surface_mask: 0,
            step_count: 0,
            target: 0,
            current: 0,
            diff: 0,
            dir: 0,
            step_int: 0,
            step_frac: 0,
            frac_count: 0,
        }
    }

    #[test]
    fn brightness_transition_oaks_sixteen_steps() {
        // State 47's transition (oaks_speech.c:1808): -16 → 0 in
        // sixteen steps, the start value returned for the inline
        // write, the poll's TRUE only once the target lands.
        let mut t = cleared_transition();
        assert_eq!(t.start(16, 0, -16, OAK_FADE_MASK), -16);
        assert!(t.active);
        assert!(!t.is_brightness_transition_active());
        for i in 1..16 {
            assert_eq!(t.step(), -16 + i);
            assert!(!t.is_brightness_transition_active());
        }
        assert_eq!(t.step(), 0);
        assert!(t.is_brightness_transition_active());
        assert!(!t.active);
    }

    #[test]
    fn brightness_transition_ball_flash_four_steps() {
        // State 52's pair (oaks_speech.c:1837-1838): 16 → 0 in four
        // steps of four.
        let mut t = cleared_transition();
        assert_eq!(t.start(4, 0, 16, BALL_FLASH_MAIN_MASK), 16);
        assert_eq!(t.step(), 12);
        assert_eq!(t.step(), 8);
        assert_eq!(t.step(), 4);
        assert!(!t.is_brightness_transition_active());
        assert_eq!(t.step(), 0);
        assert!(t.is_brightness_transition_active());
    }
}
