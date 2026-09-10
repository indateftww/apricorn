//! The main menu — pret `src/application/main_menu/main_menu.c` (the
//! OVY_74 overlay between the save check and the new-game flow),
//! Phase 4, step 5.
//!
//! `gApp_MainMenu` owns three BG layers of engine A — MAIN_0
//! (256×512, priority 2, the button column), MAIN_1 (256×256,
//! priority 1, the new-game dialog), MAIN_2 (256×512, priority 0, the
//! wireless icons) — and walks `MainMenuApp_Main`'s switch
//! (`:1392`): the setup pass, the save probe, the button build, the
//! interactive list, then the overlay's exit fade and free pass.
//! With no loadable save the overlay never draws: the probe tick
//! picks NEW_GAME and goes straight to the exit fade (11 ticks,
//! `main_menu.c:1429-1431`); a clean save builds five buttons —
//! CONTINUE, NEW GAME, POKEWALKER, NINTENDO WFC, WII MESSAGE
//! SETTINGS (`sMainMenuButtons`'s gates, this port's reach) — and
//! fades in to the interactive list (12 ticks).
//!
//! The port follows pret's shapes one-to-one: the builder trio
//! (`asm/overlay_74_thumb.s:25503`/`25569` — window creation plus a
//! synchronous `TEXT_SPEED_NOTRANSFER` label print), the focus
//! redraws (`ov74_0222841C`/`ov74_02228548`: the focused button
//! takes the FRAME1 tiles at bank 3, the resting ones FRAME0 at
//! bank 2), the fade helpers (`ov74_0223539C`/`ov74_022353FC`:
//! the 6×1 both-screen fade, the registered-next state parked in
//! bss, the color picked by the white flag `ov74_02235390`), the
//! scroll (`HandleScreenScroll`, 12 pixels a frame at most), and
//! the nested dialog machine (`ov74_0222779C`, states 15-20).
//!
//! **The new-game dialog**: A on NEW GAME arms it (`ov74_022270C4`
//! sets `unk40` bit 7 and `unk148`, jumps to state 6 *without* a
//! fade — `:243-246`); the dialog then runs its own machine — the
//! frame graphics into MAIN_1, the frameless centered WARNING
//! window plus the two option windows (`msg_0017_00005-7`), a
//! 30-frame countdown, the option input, and the close that hands
//! the result back to state 6, which either returns to the list
//! (the B edge) or runs the exit fade into the free pass.
//!
//! Deferrals, each honest in the frame: the OBJ arrow sprites and
//! `MainMenu_UpdateArrowSprites` (sprite system), the button-border
//! animation and the three `HW_PLTT` single-color writes (`[31]`,
//! `[33]`, the deferred `[54]` — the model's palette RAM composes
//! only file loads), the wireless icons (`MainMenu_SetupWifiTiles`,
//! MAIN_2's content), the CONTINUE player-info fields
//! (`msg_0442_00013-20` — they need the structured save blocks),
//! `ov74_022276AC`'s mystery-gift discovery (and the `unk48`
//! rebuild / `unk188` re-arm and dialog rows 0-3 it feeds),
//! `DetectInsertedGBACart` (no GBA slot on this model —
//! `connectedAgbGame` stays 0, so the migrate button never draws),
//! the main list's touch selection (`MainMenu_HandleTouchInput` —
//! its pixel test walks the rendered tilemap; the *dialog's* two
//! hitboxes are honored, their pixel test always passing per
//! pret's own comment), `ov74_022358BC`, audio (`PlaySE`,
//! `Sound_Stop`, the `unk13C` scene var), the heap, and the
//! `effectiveScreenY == 384` tilemap wipe (unreachable with five
//! buttons — the deepest row sits at 240).

use std::sync::Mutex;

use crate::app::fade::{BrightnessFade, FadeColor, FadeType};
use crate::app::text::{TextFlags, TextPrinter, TEXT_SPEED_NOTRANSFER};
use crate::app::{App, ChainNext};
use crate::assets::{AssetStore, AssetsError, font_narc, frame_narc, msg_narc};
use crate::frame::{
    AssetId, BgLayer, ColorMode, DisplaySelect, LogicalFrame, PaletteLoad, ScreenSize, TextColor,
    TilePlacement, Window, WindowFrame,
};
use crate::font::Font;
use crate::input::{Input, Keys, Touch, key};
use crate::text::string::GameString;

/// `NARC_msg_msg_0442_bin` — the button-label bank.
const MSG_BANK_BUTTONS: usize = 442;
/// `NARC_msg_msg_0017_bin` — the new-game dialog's bank.
const MSG_BANK_DIALOG: usize = 17;
/// The button bank's messages this port prints, in id order —
/// CONTINUE 0, NEW GAME 1, POKEWALKER 9, WII MESSAGE SETTINGS 10,
/// NINTENDO WFC 12 (the gated buttons' labels arrive with their
/// discovery flows).
const BUTTON_MSG_IDS: [usize; 5] = [0, 1, 9, 10, 12];
/// The dialog's three messages — ids 5-7, `ov74_0223BC30`'s rows
/// 4-6: the warning, "Begin adventure", "Return to the menu".
const DIALOG_MSG_IDS: [usize; 3] = [5, 6, 7];

/// `APPOPTION_COUNT` (`= APPOPTION_WII_SETTINGS`, 9) — the button
/// count; every array and loop is indexed by *table position* 0-8,
/// so slot 0 is CONTINUE and TITLE_SCREEN has no slot.
const SLOT_COUNT: usize = 9;

/// The MAIN_0 layer — the button column (`GF_BG_LYR_MAIN_0`).
const MAIN_0: usize = 0;
/// The MAIN_1 layer — the new-game dialog (`GF_BG_LYR_MAIN_1`).
const MAIN_1: usize = 1;
/// The MAIN_2 layer — the wireless icons (`GF_BG_LYR_MAIN_2`).
const MAIN_2: usize = 2;
/// MAIN_0's char block — its template's char base `0x0`.
const MAIN0_BLOCK: usize = 0;
/// MAIN_1's char block — its template's char base `0x8000`.
const MAIN1_BLOCK: usize = 0x8000 / 0x8000;
/// MAIN_2's char block — `0x0`, MAIN_0's block again.
const MAIN2_BLOCK: usize = 0;
/// MAIN_0's priority (`G2_SetBG0Priority(2)`, `:810`).
const MAIN0_PRIORITY: u8 = 2;
/// MAIN_1's priority (`:815`).
const MAIN1_PRIORITY: u8 = 1;
/// MAIN_2's priority (`:820`).
const MAIN2_PRIORITY: u8 = 0;

/// `LoadUserFrameGfx1(…, 0x3F7, 2, 0)` — FRAME0's tiles at `0x3F7`,
/// its NCLR into bank 2 (the resting border).
const FRAME0_TILE: u16 = 0x3F7;
/// `LoadUserFrameGfx1(…, 0x3EE, 3, 1)` — FRAME1's tiles at `0x3EE`,
/// its NCLR into bank 3 (the focused border).
const FRAME1_TILE: u16 = 0x3EE;
/// The two frame banks' palette-RAM offsets, in colors (bank n = n·16).
const FRAME0_BANK: u16 = 2 * 16;
const FRAME1_BANK: u16 = 3 * 16;
/// `LoadFontPal0`'s two banks — slot 1 first, then slot 0 (`:826-827`).
const FONT_BANK1: u16 = 1 * 16;
const FONT_BANK0: u16 = 0 * 16;
/// One 16-color palette load — one bank.
const BANK_COLORS: u16 = 16;

/// `MAIN_MENU_BACKGROUND_COLOR` — `RGB(12, 12, 31)` (`:1390`), raw
/// through `GX_RGB`'s shifts (R low, B high —
/// `lib/include/nitro/gx/gxcommon.h:9-13`); case 4's `HW_PLTT[0]`
/// write, the engine's backdrop color.
const BACKGROUND_COLOR: u16 = 12 | (12 << 5) | (31 << 10);

/// The button windows' geometry — every one of them: `SetWindowX(3)`
/// / `ov74_02235568(…, 3, …)` at left 3, `ov74_02235464(…, 23, …)` 23
/// tiles wide, the builder's `paletteNum1` 1.
const BUTTON_LEFT: u8 = 3;
const BUTTON_WIDTH: u8 = 23;
/// The builder trio's print color — `0x0001020F`,
/// `MAKE_TEXT_COLOR(1, 2, 15)`.
const BUTTON_TEXT_COLOR: TextColor = TextColor::new(1, 2, 15);
/// The builder trio's fill byte (`[0x48]`, `FillWindowPixelBuffer`).
const WINDOW_FILL: u8 = 0xF;

/// The dialog's warning window — `ov74_0223BC30` row 4, frameless
/// (`unk.unk0 = 2`), its text centered over the 32-tile row
/// (`:673-679`) at `textY` 4, base tile `0x91`.
const WARNING_LEFT: u8 = 0;
const WARNING_TOP: u8 = 1;
const WARNING_WIDTH: u8 = 32;
const WARNING_HEIGHT: u8 = 11;
const WARNING_BASE_TILE: u16 = 0x91;
const WARNING_TEXT_Y: u16 = 4;
/// The dialog's two option windows — rows 5-6, `textY` 4, base tiles
/// `(i * 72) + 1`, the default `{0x3F7, 2}` frame.
const OPTION_LEFT: u8 = 4;
const OPTION_TOPS: [u8; 2] = [14, 19];
const OPTION_WIDTH: u8 = 24;
const OPTION_HEIGHT: u8 = 3;
const OPTION_BASE_TILES: [u16; 2] = [1, 1 + 72];
const OPTION_TEXT_Y: u16 = 4;
/// The dialog's open countdown (`unk14C`, `:700`).
const DIALOG_COUNTDOWN: u32 = 30;
/// The selection-change input lockout (`unk1B4 = 6`,
/// `ChangeCurrentAppOption`).
const INPUT_LOCKOUT: u32 = 6;

/// `sNewGameButtonHitboxes` (`:187-190`) — the two option rows, as
/// (top, bottom, left, right). The C's pixel test always passes
/// (pret's own comment: the hitbox sits fully inside the window), so
/// the port tests the rect alone.
const NEW_GAME_HITBOXES: [(u8, u8, u8, u8); 2] = [(112, 136, 32, 224), (152, 176, 32, 224)];

/// `FX32_ONE` — the fixed-point unit of the scroll accumulators.
const FX32_ONE: i32 = 4096;
/// `GX_LCD_SIZE_Y` — the screen the scroll windows measure against.
const SCREEN_HEIGHT: i32 = 192;
/// The scroll's per-frame clamp — `FX32_CONST(12)` (`:770-776`).
const SCROLL_MAX_STEP: i32 = 12 * FX32_ONE;
/// The scroll's snap threshold — `FX32_CONST(0.125f)` (`:779`).
const SCROLL_SNAP: i32 = FX32_ONE / 8;

/// `MainMenu_AppOption` (`main_menu.h`) — the ten selections; the
/// values are the enum's own, and `unkEC[slot]` stores the button's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppOption {
    /// `APPOPTION_TITLE_SCREEN` — the B edge's pick.
    TitleScreen,
    /// `APPOPTION_CONTINUE` — slot 0.
    Continue,
    /// `APPOPTION_NEW_GAME` — slot 1.
    NewGame,
    /// `APPOPTION_POKEWALKER` — slot 2.
    Pokewalker,
    /// `APPOPTION_MYSTERY_GIFT` — slot 3 (gated off).
    MysteryGift,
    /// `APPOPTION_RANGER` — slot 4 (gated off).
    Ranger,
    /// `APPOPTION_MIGRATE_AGB` — slot 5 (gated off).
    MigrateAgb,
    /// `APPOPTION_CONNECT_TO_WII` — slot 6 (gated off).
    ConnectToWii,
    /// `APPOPTION_WFC` — slot 7.
    Wfc,
    /// `APPOPTION_WII_SETTINGS` — slot 8.
    WiiSettings,
}

/// The menu's exit — `MainMenu_QueueSelectedApp` (`:1482`), the
/// `RegisterMainOverlay` call the finishing overlay makes.
///
/// CONTINUE and the app leaves (`:1488`, `:1494-1515`) register
/// overlays later phases port; the machine routes them through
/// [`crate::app::game::GameState::Continue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainMenuExit {
    /// OVY_36's `ov36_App_MainMenu_SelectOption_Continue` (`:1488`).
    Continue,
    /// OVY_36's `ov36_App_MainMenu_SelectOption_NewGame` (`:1491`) —
    /// the step's target flow.
    NewGame,
    /// OVY_74's mystery-gift app (`:1494`).
    MysteryGift,
    /// OVY_74's migrate app (`:1497`).
    MigrateAgb,
    /// OVY_74's Ranger app (`:1500`).
    ConnectToRanger,
    /// `sub_02027098("data/eoo.dat")` (`:1502`).
    ConnectToWii,
    /// `Sound_Stop` + the WFC setup (`:1505-1507`).
    Wfc,
    /// `Sound_Stop` + OVY_112's Pokewalker app (`:1509-1511`).
    Pokewalker,
    /// `Sound_Stop` + OVY_75's Wii message settings (`:1513-1515`).
    WiiSettings,
    /// The title screen — `intro_title`'s `gApplication_TitleScreen`
    /// (`:1517-1518`), the B edge's pick.
    BackToTitle,
}

/// One row of `sMainMenuButtons` (`:175-185`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Button {
    /// The row's `id` — what `unkEC[slot]` becomes when in use.
    option: AppOption,
    /// The row's `height` — CONTINUE's is the double 10.
    height: u8,
    /// The row's `msgId` in the button bank (MIGRATE_AGB's is
    /// dynamic, picked by the inserted cartridge — never here).
    msg: usize,
    /// The row's `printFunction` — `None` is NEW_GAME's, the one row
    /// the build loop prints itself.
    print: Option<PrintFunction>,
}

/// The button print functions (`:166-173`), by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrintFunction {
    /// `MainMenu_PrintContinueButton`.
    Continue,
    /// `MainMenu_PrintMigrateFromAgbButton`.
    MigrateFromAgb,
    /// `MainMenu_PrintMysteryGiftButton`.
    MysteryGift,
    /// `MainMenu_PrintConnectToRangerButton`.
    ConnectToRanger,
    /// `MainMenu_PrintConnectToWiiButton`.
    ConnectToWii,
    /// `MainMenu_PrintNintendoWFCSetupButton`.
    NintendoWfcSetup,
    /// `MainMenu_PrintConnectToPokewalkerButton`.
    ConnectToPokewalker,
    /// `MainMenu_PrintWiiMessageSettingsButton`.
    WiiMessageSettings,
}

/// `sMainMenuButtons` (`:175-185`) — the nine rows, table-position
/// indexed.
const BUTTONS: [Button; SLOT_COUNT] = [
    Button {
        option: AppOption::Continue,
        height: 10,
        msg: 0,
        print: Some(PrintFunction::Continue),
    },
    Button {
        option: AppOption::NewGame,
        height: 4,
        msg: 1,
        print: None,
    },
    Button {
        option: AppOption::Pokewalker,
        height: 4,
        msg: 9,
        print: Some(PrintFunction::ConnectToPokewalker),
    },
    Button {
        option: AppOption::MysteryGift,
        height: 4,
        msg: 2,
        print: Some(PrintFunction::MysteryGift),
    },
    Button {
        option: AppOption::Ranger,
        height: 4,
        msg: 3,
        print: Some(PrintFunction::ConnectToRanger),
    },
    Button {
        option: AppOption::MigrateAgb,
        height: 4,
        msg: 0,
        print: Some(PrintFunction::MigrateFromAgb),
    },
    Button {
        option: AppOption::ConnectToWii,
        height: 4,
        msg: 11,
        print: Some(PrintFunction::ConnectToWii),
    },
    Button {
        option: AppOption::Wfc,
        height: 4,
        msg: 12,
        print: Some(PrintFunction::NintendoWfcSetup),
    },
    Button {
        option: AppOption::WiiSettings,
        height: 4,
        msg: 10,
        print: Some(PrintFunction::WiiMessageSettings),
    },
];

/// The print functions' availability gates, as this port computes
/// them: CONTINUE always draws (`MainMenu_PrintContinueButton` has
/// no gate — its player-info fields are the deferral), WFC,
/// Pokéwalker, and Wii Message Settings are unconditional, and the
/// discovery/GBA-slot/Wi-connect gates never open here.
const fn button_available(print: PrintFunction) -> bool {
    matches!(
        print,
        PrintFunction::Continue
            | PrintFunction::NintendoWfcSetup
            | PrintFunction::ConnectToPokewalker
            | PrintFunction::WiiMessageSettings
    )
}

/// `MainMenuApp_Main`'s switch (`:1409-1474`) — the overlay states.
/// The C's dead cases are documented on the variants that replaced
/// them: case 2 (the unused warning screen, only reachable from
/// case 1's dead else) and case 9 (the never-assigned exit state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuState {
    /// Case 0 — `MainMenu_SetupGraphics`, then state 1.
    SetupGraphics,
    /// Case 1 — `ov74_02227580` always returns 0, so the else
    /// (fade-in to state 2) is dead; the state goes to 3.
    Prepare,
    /// Case 3 — the save probe: no save → NEW_GAME + the exit fade;
    /// a save → the button build.
    ChooseApp,
    /// Case 4 — the build, the focus redraw, the fade-in, the
    /// backdrop color.
    BuildMenu,
    /// Case 5 — the interactive list.
    Input,
    /// Case 6 — the dialog's aftermath: the B edge back to the list,
    /// anything else into the exit fade.
    DialogDone,
    /// Case 7 — `MainMenu_FreeGraphics`, `ret TRUE` (no scroll that
    /// frame).
    Free,
    /// Case 8 — the fade wait: `ov74_022353FC` polls, then takes the
    /// parked state.
    WaitFade,
}

/// `ov74_0222779C`'s switch (`:643-757`) — the dialog machine over
/// `unk144`. State 18 (the bit0/bit1 rows' dismissal) is deferred
/// with the discovery flows that arm those bits; they never set
/// here, so the machine runs 15 → 16 → 17 → 19 → 20 → 15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DialogState {
    /// Case 15 — the idle state: `unk148`'s edge arms the dialog.
    #[default]
    Idle,
    /// Case 16 — the frame graphics into MAIN_1.
    LoadFrames,
    /// Case 17 — the windows (the bit7 path: frameless warning +
    /// two options), the countdown, the plane toggles.
    Build,
    /// Case 19 — the countdown, then the option input; the close
    /// removes the three windows.
    OptionInput,
    /// Case 20 — the plane toggles back, `unk144 = 15`.
    Restore,
}

/// `MenuInputStateMgr`'s states (`include/menu_input_state.h`) — the
/// stylus-or-buttons mode both focus redraws read. The zero-init
/// default is BUTTONS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum InputMode {
    /// `MENU_INPUT_STATE_BUTTONS` (0).
    #[default]
    Buttons,
    /// `MENU_INPUT_STATE_TOUCH` (1).
    Touch,
}

/// The main menu app.
pub struct MainMenu {
    /// The logical frame the last tick produced.
    frame: LogicalFrame,
    /// The fade pair — the both-screen works the overlay's helpers
    /// drive.
    fade: BrightnessFade,
    /// The scene's printer policy — the power-on defaults (the menu
    /// never touches the flags).
    flags: TextFlags,
    /// Font 0, cloned at load so ticks need no store lock (the
    /// builder trio's `fontId`).
    font: Font,
    /// The font-0 asset the pushed glyphs reference.
    font_asset: AssetId,
    /// The focus-indicator asset every printer carries.
    focus_asset: AssetId,
    /// FRAME0's tiles (`LoadUserFrameGfx1`'s frame-0 NCGR).
    frame0_tiles: AssetId,
    /// FRAME1's tiles (the focused border).
    frame1_tiles: AssetId,
    /// The frames' NCLR (`frame_narc::PALETTE`).
    frame_pal: AssetId,
    /// The font's palette (`font_narc::PAL0`).
    font_pal: AssetId,
    /// The button bank's printed messages, ids
    /// [`BUTTON_MSG_IDS`], cloned at load.
    button_messages: Vec<GameString>,
    /// The dialog bank's three messages, ids [`DIALOG_MSG_IDS`].
    dialog_messages: Vec<GameString>,
    /// `!data->dontHaveSavedata` — `Save_FileExists`, the probe's
    /// fact.
    save_exists: bool,
    /// The overlay state (`MainMenuApp_Main`'s `*state`).
    state: MenuState,
    /// The state parked in bss for the fade wait (`ov74_0223539C`'s
    /// `bss+0xc`, taken by `ov74_022353FC`).
    fade_next: MenuState,
    /// The dialog machine's state (`unk144`).
    dialog_state: DialogState,
    /// The shared input mode (`menuInputState.state`).
    input_mode: InputMode,
    /// The picked app (`selectedApp`) — APPOPTION_TITLE_SCREEN at
    /// the memset zero.
    selected_app: AppOption,
    /// The focused button (`currentOption`), a table position.
    current_option: usize,
    /// The dialog's focused option (`currentNewGameOption`), 0 or 1.
    current_new_game_option: usize,
    /// The button windows' indices into the engine's window list —
    /// `data->unk5C`'s bookkeeping. `None` is `unkEC[slot] == 0`,
    /// the slot not in use (no button's id is TITLE_SCREEN, so the
    /// option and the in-use flag are one fact).
    slot_windows: [Option<usize>; SLOT_COUNT],
    /// The dialog's warning-window index — the three dialog windows
    /// are `base`, `base+1`, `base+2` (the options), `None` when
    /// none are in use.
    dialog_base: Option<usize>,
    /// The pending-dialog bits (`unk40`) — bit 7 the new-game one.
    unk40: u32,
    /// The consumed-dialog bits (`unk44`).
    unk44: u32,
    /// The dialog arm countdown (`unk148`) — 1 after the A press.
    unk148: u32,
    /// The dialog's open countdown (`unk14C`).
    unk14c: u32,
    /// The dialog's result (`unk150`) — bit 0 begin, bit 1 return.
    unk150: u32,
    /// The selection-change lockout (`unk1B4`).
    unk1b4: u32,
    /// The bss white-flag (`ov74_02235390`) — the B edge and the
    /// Wii connect set it, and the next fade fades *out to white*.
    /// Zero on a fresh overlay load: black.
    white_exit: bool,
    /// The scroll's current offset (`currentScreenY`), fx32.
    current_screen_y: i32,
    /// The scroll's target (`effectiveScreenY`), fx32.
    effective_screen_y: i32,
    /// The keys held on the previous tick, for press edges.
    prev_keys: Keys,
    /// Whether the stylus was down on the previous tick.
    prev_touch: bool,
    /// Whether the free pass has run (`ret TRUE`).
    done: bool,
}

impl MainMenu {
    /// Constructs the app — `MainMenuApp_Init` plus the asset loads
    /// the model resolves through the store.
    ///
    /// `save_exists` is `Save_FileExists` — the fact the probe tick
    /// branches on.
    ///
    /// The construction frame is the register state inherited from
    /// the save check: the screens still flipped
    /// (`DisplaySelect::SubOnTop` — the setup pass writes
    /// `GX_SetDispSelect(SUB_MAIN)` itself), both LCDs black
    /// (`sub_0200FBF4`, `:1364-1365`), everything else cleared. The
    /// first tick is the setup pass.
    ///
    /// # Errors
    /// Returns the store's [`AssetsError`] when a member is missing
    /// or corrupt — unreachable in practice against the pinned dump.
    pub fn load(store: &Mutex<AssetStore>, save_exists: bool) -> Result<Self, AssetsError> {
        let mut store = store
            .lock()
            .expect("the asset store is only locked at app construction");
        // The graphics the setup and dialog passes draw from.
        let frame0_tiles = store.load_tiles(frame_narc::NARC, frame_narc::FRAME0_CHAR)?;
        let frame1_tiles = store.load_tiles(frame_narc::NARC, frame_narc::FRAME1_CHAR)?;
        let frame_pal = store.load_palette(frame_narc::NARC, frame_narc::PALETTE)?;
        let font_asset = store.load_font(font_narc::NARC, font_narc::FONT0)?;
        let font_pal = store.load_palette(font_narc::NARC, font_narc::PAL0)?;
        let focus_asset = store.load_tiles(font_narc::NARC, font_narc::FOCUS_INDICATOR)?;
        let bank = store.load_msg_bank(msg_narc::NARC, MSG_BANK_BUTTONS)?;
        let text = store.msg_bank(bank).expect("the just-loaded bank");
        let mut button_messages = Vec::with_capacity(BUTTON_MSG_IDS.len());
        for id in BUTTON_MSG_IDS {
            let units = text.message(id).expect("the button bank's labels");
            button_messages.push(GameString::from_units(units));
        }
        let bank = store.load_msg_bank(msg_narc::NARC, MSG_BANK_DIALOG)?;
        let text = store.msg_bank(bank).expect("the just-loaded bank");
        let mut dialog_messages = Vec::with_capacity(DIALOG_MSG_IDS.len());
        for id in DIALOG_MSG_IDS {
            let units = text.message(id).expect("the dialog bank's rows");
            dialog_messages.push(GameString::from_units(units));
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
            // sub_0200FBF4 both LCDs black — carried from the first
            // tick's brightness.
            fade: {
                let mut fade = BrightnessFade::default();
                fade.write(-16);
                fade
            },
            flags: TextFlags::default(),
            font,
            font_asset,
            focus_asset,
            frame0_tiles,
            frame1_tiles,
            frame_pal,
            font_pal,
            button_messages,
            dialog_messages,
            save_exists,
            state: MenuState::SetupGraphics,
            fade_next: MenuState::Free,
            // unk144 = 15 (:1378); everything else the memset zero.
            dialog_state: DialogState::Idle,
            input_mode: InputMode::Buttons,
            selected_app: AppOption::TitleScreen,
            current_option: 0,
            current_new_game_option: 0,
            slot_windows: [None; SLOT_COUNT],
            dialog_base: None,
            unk40: 0,
            unk44: 0,
            unk148: 0,
            unk14c: 0,
            unk150: 0,
            unk1b4: 0,
            white_exit: false,
            // currentScreenY = effectiveScreenY = 0 (:1371-1372).
            current_screen_y: 0,
            effective_screen_y: 0,
            prev_keys: Keys::IDLE,
            prev_touch: false,
            done: false,
        })
    }

    /// The selected app's queue step — `MainMenu_QueueSelectedApp`
    /// (`:1482`), as the finishing tick leaves it. `None` until the
    /// app finishes (the free pass).
    #[must_use]
    pub fn exit(&self) -> Option<MainMenuExit> {
        if !self.done {
            return None;
        }
        Some(match self.selected_app {
            AppOption::Continue => MainMenuExit::Continue,
            AppOption::NewGame => MainMenuExit::NewGame,
            AppOption::MysteryGift => MainMenuExit::MysteryGift,
            AppOption::MigrateAgb => MainMenuExit::MigrateAgb,
            AppOption::Ranger => MainMenuExit::ConnectToRanger,
            AppOption::ConnectToWii => MainMenuExit::ConnectToWii,
            AppOption::Wfc => MainMenuExit::Wfc,
            AppOption::Pokewalker => MainMenuExit::Pokewalker,
            AppOption::WiiSettings => MainMenuExit::WiiSettings,
            AppOption::TitleScreen => MainMenuExit::BackToTitle,
        })
    }

    /// `ov74_0223539C(type, newState, state, 8)` (`overlay_74_thumb.s:25436`):
    /// the both-screen fade, 6 frames × 1, the color picked by the
    /// bss white flag; `*state` takes the wait (8) and the new state
    /// parks in bss for `ov74_022353FC`'s poll.
    fn begin_exit_fade(&mut self, ty: FadeType, next: MenuState) {
        let color = if self.white_exit {
            FadeColor::White
        } else {
            FadeColor::Black
        };
        self.fade.begin(ty, color, 6, 1);
        self.state = MenuState::WaitFade;
        self.fade_next = next;
    }

    /// `ov74_02235390(1)` — the bss white flag.
    fn set_white_exit(&mut self) {
        self.white_exit = true;
    }

    /// The setup pass — case 0: `MainMenu_SetupGraphics` (`:787`).
    /// The three `ov74_02235308` templates toggle their planes on,
    /// the font and frame palettes load, and the backdrop goes
    /// black (`HW_PLTT[0] = RGB_BLACK`; `[31]` and `[33]`'s
    /// single-color writes are deferred).
    fn setup_graphics(&mut self) {
        // GX_SetDispSelect(SUB_MAIN) — already the inherited state.
        self.frame.display = DisplaySelect::SubOnTop;
        // MAIN_0: 256x512, screen 0xF000, char 0x0, priority 2.
        self.frame.main.bgs[MAIN_0] = BgLayer {
            enabled: true,
            char_base: MAIN0_BLOCK as u8,
            screen: None,
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH512,
            scroll_x: 0,
            scroll_y: 0,
            priority: MAIN0_PRIORITY,
        };
        // MAIN_1: 256x256, screen 0xD800, char 0x8000, priority 1.
        self.frame.main.bgs[MAIN_1] = BgLayer {
            enabled: true,
            char_base: MAIN1_BLOCK as u8,
            screen: None,
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH256,
            scroll_x: 0,
            scroll_y: 0,
            priority: MAIN1_PRIORITY,
        };
        // MAIN_2: 256x512, screen 0xE000, char 0x0, priority 0 —
        // MAIN_0's char block again.
        self.frame.main.bgs[MAIN_2] = BgLayer {
            enabled: true,
            char_base: MAIN2_BLOCK as u8,
            screen: None,
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH512,
            scroll_x: 0,
            scroll_y: 0,
            priority: MAIN2_PRIORITY,
        };
        // LoadFontPal0 slot 1, then slot 0 (:826-827).
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.font_pal,
            offset: FONT_BANK1,
            colors: BANK_COLORS,
        });
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.font_pal,
            offset: FONT_BANK0,
            colors: BANK_COLORS,
        });
        // HW_PLTT[0] = RGB_BLACK — the engine's backdrop color.
        self.frame.main.backdrop = 0;
        // LoadUserFrameGfx1: FRAME0's NCGR at 0x3F7, NCLR into bank
        // 2; FRAME1's NCGR at 0x3EE, NCLR into bank 3 (:832-833) —
        // into MAIN_0's char block.
        self.frame.main.char_blocks[MAIN0_BLOCK].push(TilePlacement {
            asset: self.frame0_tiles,
            tile: FRAME0_TILE,
        });
        self.frame.main.char_blocks[MAIN0_BLOCK].push(TilePlacement {
            asset: self.frame1_tiles,
            tile: FRAME1_TILE,
        });
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.frame_pal,
            offset: FRAME0_BANK,
            colors: BANK_COLORS,
        });
        self.frame.main.palette_loads.push(PaletteLoad {
            asset: self.frame_pal,
            offset: FRAME1_BANK,
            colors: BANK_COLORS,
        });
    }

    /// The free pass — case 7: `MainMenu_FreeGraphics` (`:1282`):
    /// the per-slot `RemoveWindow`s, the three tilemap buffers, and
    /// `GX_SetDispSelect(MAIN_SUB)`. The plane-enable registers are
    /// not touched — the next overlay's own init owns them from
    /// here.
    fn free_graphics(&mut self) {
        self.frame.main.windows.clear();
        self.dialog_base = None;
        self.frame.display = DisplaySelect::MainOnTop;
        self.done = true;
    }

    /// The button build — `ov74_022282CC` (`:1086`): the slot walk,
    /// the base-tile walk (`unk20 += height * 23` every slot), and
    /// the per-slot window creation or reposition. Returns whether
    /// any print function drew (`ret`).
    fn build_buttons(&mut self, input: Input) -> bool {
        let mut ret = false;
        let mut base_tile: u16 = 1; // data->unk20
        let mut y: u8 = 1;
        for slot in 0..SLOT_COUNT {
            let button = BUTTONS[slot];
            match button.print {
                // NEW_GAME — the NULL printFunction's branch
                // (:1119-1123): the window, the label, in use
                // unconditionally — and on a rebuild the window is
                // already in use, so `ov74_02235568` takes the
                // SetWindowX/Y path and re-prints into the same
                // window rather than re-adding it.
                None => {
                    let index = match self.slot_windows[slot] {
                        Some(index) => {
                            let window = &mut self.frame.main.windows[index];
                            window.left = BUTTON_LEFT;
                            window.top = y;
                            index
                        }
                        None => self.push_button_window(y, button.height, base_tile),
                    };
                    self.print_button_label(index, button.msg, input);
                    self.frame.main.windows[index].frame =
                        Some(WindowFrame {
                            base_tile: FRAME0_TILE,
                            palette: 2,
                            dialogue: false,
                        });
                    self.slot_windows[slot] = Some(index);
                    y += button.height + 2;
                }
                Some(print) => {
                    if let Some(index) = self.slot_windows[slot] {
                        // Already in use — the rebuild branch
                        // (:1101-1112): reposition and redraw the
                        // resting frame (the wireless icon the
                        // branch also draws is deferred).
                        let window = &mut self.frame.main.windows[index];
                        window.left = BUTTON_LEFT;
                        window.top = y;
                        window.frame = Some(WindowFrame {
                            base_tile: FRAME0_TILE,
                            palette: 2,
                            dialogue: false,
                        });
                        y += button.height + 2;
                        ret = true;
                    } else if button_available(print)
                        && self.print_button(slot, print, y, base_tile, input)
                    {
                        y += button.height + 2;
                        ret = true;
                    }
                }
            }
            base_tile += u16::from(button.height) * 23;
            // data->unk1B0++ per in-use slot (:1127-1129) — the
            // count's one read (:384) is the deferred touch path's
            // bounds check, so the port keeps no copy.
        }
        // MainMenu_UpdateArrowSprites is deferred with the sprites;
        // the build's own focus redraw (:1131-1132).
        self.redraw_focus(self.current_option as i32);
        ret
    }

    /// A button window pushed at (3, `y`) — `AddWindowParameterized`
    /// through `ov74_02235568`: 23 wide, `paletteNum1` 1, base tile
    /// `unk20`'s walk value.
    fn push_button_window(&mut self, y: u8, height: u8, base_tile: u16) -> usize {
        let index = self.frame.main.windows.len();
        self.frame.main.windows.push(Window {
            bg: MAIN_0 as u8,
            left: BUTTON_LEFT,
            top: y,
            width: BUTTON_WIDTH,
            height,
            palette: 1,
            base_tile,
            fill: WINDOW_FILL,
            glyphs: Vec::new(),
            scroll: 0,
            frame: None,
            arrow: None,
            focus: None,
        });
        index
    }

    /// An available print function's draw — the shared shape of
    /// `MainMenu_PrintContinueButton` and its unconditional
    /// siblings (`:917`/`1061`/`1070`/`1078`): the window via
    /// `ov74_02235568(…, 3, y, msgId)`, the label, the resting
    /// frame, and the slot's id. CONTINUE's player-info fields and
    /// the wireless icons the originals also draw are deferred.
    fn print_button(
        &mut self,
        slot: usize,
        print: PrintFunction,
        y: u8,
        base_tile: u16,
        input: Input,
    ) -> bool {
        debug_assert!(button_available(print), "the gated fns never draw here");
        let index = self.push_button_window(y, BUTTONS[slot].height, base_tile);
        self.print_button_label(index, BUTTONS[slot].msg, input);
        self.frame.main.windows[index].frame = Some(WindowFrame {
            base_tile: FRAME0_TILE,
            palette: 2,
            dialogue: false,
        });
        self.slot_windows[slot] = Some(index);
        true
    }

    /// The builder trio's print — `ov74_0223547C` +
    /// `ov74_02235568`'s `AddTextPrinterParameterizedWithColor`
    /// (`overlay_74_thumb.s:25569`/`25685`): the fill, then the
    /// synchronous `TEXT_SPEED_NOTRANSFER` print of the button
    /// bank's `msg` at (`text_x`, `text_y`).
    fn print_button_label(&mut self, index: usize, msg: usize, input: Input) {
        let msg_index = BUTTON_MSG_IDS
            .iter()
            .position(|&id| id == msg)
            .expect("the loaded button messages");
        let units = self.button_messages[msg_index].clone();
        let mut printer = TextPrinter::new(
            0,
            self.font_asset,
            self.focus_asset,
            units,
            0,
            0,
            BUTTON_TEXT_COLOR,
            TEXT_SPEED_NOTRANSFER,
            0,
        );
        let window = &mut self.frame.main.windows[index];
        // FillWindowPixelBuffer(fill) — the glyphs and focus go with
        // the fill.
        window.glyphs.clear();
        window.focus = None;
        printer.render_instant(&self.font, window, input, &mut self.flags);
    }

    /// The dialog bank's print — the same builder trio over a
    /// dialog window, at the builder's `textX`/`textY`.
    fn print_dialog_message(&mut self, index: usize, msg: usize, text_x: u16, text_y: u16, input: Input) {
        let msg_index = msg - DIALOG_MSG_IDS[0];
        let units = self.dialog_messages[msg_index].clone();
        let mut printer = TextPrinter::new(
            0,
            self.font_asset,
            self.focus_asset,
            units,
            text_x,
            text_y,
            BUTTON_TEXT_COLOR,
            TEXT_SPEED_NOTRANSFER,
            0,
        );
        let window = &mut self.frame.main.windows[index];
        window.glyphs.clear();
        window.focus = None;
        printer.render_instant(&self.font, window, input, &mut self.flags);
    }

    /// The focus redraw — `ov74_0222841C` (`:1137`): every in-use
    /// button within five of the current takes one of the three
    /// border/bank pairs. `current` may be 0xFF (the touch path's
    /// deselect redraw).
    fn redraw_focus(&mut self, current: i32) {
        for slot in 0..SLOT_COUNT {
            let Some(index) = self.slot_windows[slot] else {
                continue;
            };
            // Slot 0's skip while scrolled to the bottom
            // (:1144) — the 384 scroll is unreachable with this
            // port's five buttons, and the model carries it anyway.
            if slot == 0 && self.effective_screen_y == 2 * SCREEN_HEIGHT * FX32_ONE {
                continue;
            }
            if (slot as i32) < (current - 5) || (slot as i32) > (current + 5) {
                continue;
            }
            let window = &mut self.frame.main.windows[index];
            if self.input_mode == InputMode::Touch {
                window.frame = Some(WindowFrame {
                    base_tile: FRAME1_TILE,
                    palette: 2,
                    dialogue: false,
                });
                window.palette = 0;
            } else if slot as i32 == current {
                window.frame = Some(WindowFrame {
                    base_tile: FRAME1_TILE,
                    palette: 3,
                    dialogue: false,
                });
                window.palette = 0;
            } else {
                window.frame = Some(WindowFrame {
                    base_tile: FRAME0_TILE,
                    palette: 2,
                    dialogue: false,
                });
                window.palette = 1;
            }
        }
    }

    /// The dialog focus redraw — `ov74_02228548` (`:1167`), over the
    /// two option windows.
    fn redraw_dialog_focus(&mut self, focus: i32) {
        let Some(base) = self.dialog_base else {
            return;
        };
        for i in 0..2 {
            let window = &mut self.frame.main.windows[base + 1 + i];
            if focus < 0 {
                window.frame = Some(WindowFrame {
                    base_tile: FRAME0_TILE,
                    palette: 2,
                    dialogue: false,
                });
                window.palette = 1;
            } else if self.input_mode == InputMode::Touch {
                window.frame = Some(WindowFrame {
                    base_tile: FRAME1_TILE,
                    palette: 2,
                    dialogue: false,
                });
                window.palette = 0;
            } else if i as i32 == focus {
                window.frame = Some(WindowFrame {
                    base_tile: FRAME1_TILE,
                    palette: 3,
                    dialogue: false,
                });
                window.palette = 0;
            } else {
                window.frame = Some(WindowFrame {
                    base_tile: FRAME0_TILE,
                    palette: 2,
                    dialogue: false,
                });
                window.palette = 1;
            }
        }
    }

    /// `ChangeCurrentAppOption` (`:1191`): the walk over the table —
    /// clamped at both ends, stopping at the first in-use slot —
    /// then the redraw and the lockout on a change.
    fn change_current_app_option(&mut self, offset: i32) -> bool {
        let mut option = self.current_option as i32;
        let old_option = option;
        loop {
            option += offset;
            if option == -1 {
                option = 0;
            }
            if option == SLOT_COUNT as i32 {
                option = SLOT_COUNT as i32 - 1;
            }
            if option == self.current_option as i32 {
                break;
            }
            if self.slot_windows[option as usize].is_some() {
                // PlaySE(SEQ_SE_DP_SELECT) — audio deferred.
                break;
            }
        }
        self.current_option = option as usize;
        if option != old_option {
            self.redraw_focus(self.current_option as i32);
            self.unk1b4 = INPUT_LOCKOUT;
            true
        } else {
            false
        }
    }

    /// `CountAvailableAppsBefore` (`:261`) — the in-use slots above
    /// `slot`.
    fn count_available_apps_before(&self, slot: usize) -> u32 {
        self.slot_windows[..slot]
            .iter()
            .filter(|window| window.is_some())
            .count() as u32
    }

    /// `ov74_022286F8` (`:1226`): keep the option's window on
    /// screen — set the scroll target to its row, and rebuild the
    /// buttons when the target is the top (the 384 target's
    /// tilemap wipe is unreachable with five buttons, and deferred).
    fn scroll_to(&mut self, slot: usize, input: Input) {
        let index = self
            .slot_windows[slot]
            .expect("the scroll target is an in-use slot");
        let option_y = (i32::from(self.frame.main.windows[index].top) - 1) * 8;
        let screen_y = self.effective_screen_y / FX32_ONE;
        if screen_y <= option_y && screen_y + SCREEN_HEIGHT > option_y {
            return;
        }
        self.effective_screen_y = option_y * FX32_ONE;
        // MainMenu_UpdateArrowSprites — deferred with the sprites.
        if self.effective_screen_y == 0 {
            self.build_buttons(input);
        }
    }

    /// `HandleScreenScroll` (`:762`): the eased walk of the current
    /// offset toward the target, at most 12 pixels a frame, snapped
    /// within an eighth — then both scrolling layers' Y.
    fn handle_screen_scroll(&mut self) {
        if self.current_screen_y == self.effective_screen_y {
            return;
        }
        let mut step = (self.effective_screen_y - self.current_screen_y) / 4;
        if step.abs() > SCROLL_MAX_STEP {
            step = if step > 0 {
                SCROLL_MAX_STEP
            } else {
                -SCROLL_MAX_STEP
            };
        }
        self.current_screen_y += step;
        if (self.effective_screen_y - self.current_screen_y).abs() < SCROLL_SNAP {
            self.current_screen_y = self.effective_screen_y;
        }
        let scroll_y = (self.current_screen_y / FX32_ONE) as u16;
        self.frame.main.bgs[MAIN_0].scroll_y = scroll_y;
        self.frame.main.bgs[MAIN_2].scroll_y = scroll_y;
    }

    /// The interactive list — case 5's `MainMenu_HandleInput` (`:395`).
    /// The list's own touch selection is deferred with the arrow
    /// sprites, so touches read as no input and the key path runs.
    fn handle_input(&mut self, input: Input, new_keys: Keys) {
        self.handle_key_input(input, new_keys);
    }

    /// `MainMenu_HandleKeyInput` (`:272`).
    fn handle_key_input(&mut self, input: Input, new_keys: Keys) {
        const TRANSITION: u16 = key::Y
            | key::X
            | key::UP
            | key::DOWN
            | key::LEFT
            | key::RIGHT
            | key::B
            | key::A;
        // The touch→buttons transition (:275-279) — the input mode
        // is only Touch when the dialog left it so.
        if new_keys.any(TRANSITION) && self.input_mode == InputMode::Touch {
            self.input_mode = InputMode::Buttons;
            self.redraw_focus(self.current_option as i32);
            return;
        }
        if new_keys.any(key::A) {
            self.confirm_selection(false);
            return;
        }
        if new_keys.any(key::B) {
            self.confirm_selection(true);
            return;
        }
        // The unk48 rebuild gate (:287-289) — ov74_022276AC's
        // discovery, never set here.
        let old_option = self.current_option;
        let mut scroll_slot = 0;
        let mut changed = false;
        if new_keys.any(key::UP) {
            if self.change_current_app_option(-1) {
                let mut unk = self.count_available_apps_before(old_option);
                if unk == 7 {
                    // Eight in-use slots above the old one — a full
                    // nine-button menu, unreachable with this
                    // port's five.
                    unk = 3;
                    changed = true;
                } else if unk == 3 {
                    unk = 0;
                    changed = true;
                }
                scroll_slot = unk as usize;
            }
        } else if new_keys.any(key::DOWN) {
            changed = self.change_current_app_option(1);
            scroll_slot = self.current_option;
        }
        if changed {
            self.scroll_to(scroll_slot, input);
        }
    }

    /// `ov74_022270C4` (`:227`) — the A/B press. `is_b` is the B
    /// edge's `a2 == TRUE`: the pick becomes TITLE_SCREEN and the
    /// white flag sets (`ov74_02235390(1)`), so the exit fade fades
    /// out to white. NEW GAME detours into the dialog (state 6, no
    /// fade); anything else runs the exit fade.
    fn confirm_selection(&mut self, is_b: bool) {
        if !is_b {
            // PlaySE and the MIGRATE_AGB cart-pullout error
            // (:229-236) — audio and the slot, both deferred.
            self.selected_app = BUTTONS[self.current_option].option;
        } else {
            self.selected_app = AppOption::TitleScreen;
            self.set_white_exit();
        }
        if self.selected_app == AppOption::NewGame {
            // unk40 |= 1 << 7 — the new-game dialog arms.
            self.unk40 |= 1 << 7;
            self.unk148 = 1;
            self.state = MenuState::DialogDone;
        } else {
            if self.selected_app == AppOption::ConnectToWii {
                self.set_white_exit();
            }
            self.begin_exit_fade(FadeType::BrightnessOut, MenuState::Free);
            // unk13C's 13 → 14 (:254-256) — audio, deferred.
        }
    }

    /// `ov74_0222779C` (`:632`) — the dialog machine, one pass per
    /// tick ahead of the state switch. `true` is the consumed frame
    /// (`MainMenuApp_Main`'s early return, `:1397-1401`): the state
    /// switch and the input lockout skip, the scroll still runs.
    fn handle_dialog(&mut self, input: Input, new_keys: Keys, touch_new: bool) -> bool {
        match self.dialog_state {
            DialogState::Idle => {
                // Case 15 — the arm countdown's edge.
                if self.unk148 == 0 {
                    return false;
                }
                self.unk148 -= 1;
                if self.unk148 == 0 {
                    self.dialog_state = DialogState::LoadFrames;
                }
                true
            }
            DialogState::LoadFrames => {
                // Case 16 — the frame graphics into MAIN_1: the
                // same NCGRs and banks as the setup pass
                // (:655-656), into MAIN_1's char block.
                // BgClearTilemapBufferAndCommit(MAIN_1) — the
                // model's cleared tilemap; HW_PLTT[33] deferred.
                self.frame.main.char_blocks[MAIN1_BLOCK].push(TilePlacement {
                    asset: self.frame0_tiles,
                    tile: FRAME0_TILE,
                });
                self.frame.main.char_blocks[MAIN1_BLOCK].push(TilePlacement {
                    asset: self.frame1_tiles,
                    tile: FRAME1_TILE,
                });
                self.frame.main.palette_loads.push(PaletteLoad {
                    asset: self.frame_pal,
                    offset: FRAME0_BANK,
                    colors: BANK_COLORS,
                });
                self.frame.main.palette_loads.push(PaletteLoad {
                    asset: self.frame_pal,
                    offset: FRAME1_BANK,
                    colors: BANK_COLORS,
                });
                self.dialog_state = DialogState::Build;
                true
            }
            DialogState::Build => {
                // Case 17 — the bit7 path (:670-705). The bit0/bit1
                // rows (ov74_0223BC30 rows 0-3, dialog state 18)
                // arm only from the deferred discovery flows; the
                // bits never set here, so the pending mask always
                // carries bit 7.
                let pending = self.unk40 & !self.unk44;
                debug_assert!(
                    pending & (1 << 7) != 0,
                    "only the new-game dialog arms in this port"
                );
                // The frameless warning window, its text centered
                // over the 32-tile row (:673-679).
                let warning_text = self.dialog_messages[0].clone();
                let text_width = self
                    .font
                    .multiline_width(warning_text.units(), 0);
                let text_x = (i32::from(WARNING_WIDTH) * 8 - text_width as i32) / 2;
                let dialog_base = self.frame.main.windows.len();
                self.frame.main.windows.push(Window {
                    bg: MAIN_1 as u8,
                    left: WARNING_LEFT,
                    top: WARNING_TOP,
                    width: WARNING_WIDTH,
                    height: WARNING_HEIGHT,
                    palette: 0,
                    base_tile: WARNING_BASE_TILE,
                    fill: WINDOW_FILL,
                    glyphs: Vec::new(),
                    scroll: 0,
                    frame: None,
                    arrow: None,
                    focus: None,
                });
                self.print_dialog_message(
                    dialog_base,
                    DIALOG_MSG_IDS[0],
                    text_x as u16,
                    WARNING_TEXT_Y,
                    input,
                );
                // BgTilemapRectChangePalette(…, 0) (:685) — the bank
                // the builder's paletteNum1 0 already set.
                // The two option windows (:688-696).
                for i in 0..2 {
                    let index = self.frame.main.windows.len();
                    self.frame.main.windows.push(Window {
                        bg: MAIN_1 as u8,
                        left: OPTION_LEFT,
                        top: OPTION_TOPS[i],
                        width: OPTION_WIDTH,
                        height: OPTION_HEIGHT,
                        palette: 0,
                        base_tile: OPTION_BASE_TILES[i],
                        fill: WINDOW_FILL,
                        glyphs: Vec::new(),
                        scroll: 0,
                        frame: None,
                        arrow: None,
                        focus: None,
                    });
                    self.print_dialog_message(
                        index,
                        DIALOG_MSG_IDS[1 + i],
                        0,
                        OPTION_TEXT_Y,
                        input,
                    );
                    self.frame.main.windows[index].frame = Some(WindowFrame {
                        base_tile: FRAME0_TILE,
                        palette: 2,
                        dialogue: false,
                    });
                }
                self.dialog_base = Some(dialog_base);
                self.current_new_game_option = 0;
                self.redraw_dialog_focus(-1);
                self.dialog_state = DialogState::OptionInput;
                self.unk14c = DIALOG_COUNTDOWN;
                // The plane toggles (:707-709): BG0 and BG2 off,
                // BG1 on.
                self.frame.main.bgs[MAIN_0].enabled = false;
                self.frame.main.bgs[MAIN_2].enabled = false;
                self.frame.main.bgs[MAIN_1].enabled = true;
                true
            }
            DialogState::OptionInput => {
                // Case 19 — the countdown, then the option input;
                // the close removes the three windows (:731-749).
                if self.unk14c != 0 {
                    self.unk14c -= 1;
                    if self.unk14c == 0 {
                        self.redraw_dialog_focus(0);
                    }
                } else {
                    let input_result = self.new_game_handle_input(input, new_keys, touch_new);
                    // AdvanceButtonBorderAnimation — deferred.
                    if input_result & 3 != 0 {
                        let base = self
                            .dialog_base
                            .take()
                            .expect("the open dialog's windows");
                        self.frame.main.windows.truncate(base);
                        self.redraw_focus(self.current_option as i32);
                        self.dialog_state = DialogState::Restore;
                        self.unk150 = input_result;
                    }
                }
                true
            }
            DialogState::Restore => {
                // Case 20 — the plane toggles back (:751-756).
                self.frame.main.bgs[MAIN_0].enabled = true;
                self.frame.main.bgs[MAIN_2].enabled = true;
                self.frame.main.bgs[MAIN_1].enabled = false;
                self.dialog_state = DialogState::Idle;
                true
            }
        }
    }

    /// `MainMenu_NewGame_HandleInput` (`:433`).
    fn new_game_handle_input(&mut self, input: Input, new_keys: Keys, touch_new: bool) -> u32 {
        let (had_touch, touch_result) = self.new_game_handle_touch_input(input, touch_new);
        if had_touch {
            self.input_mode = InputMode::Touch;
            self.redraw_dialog_focus(0xFF);
            return touch_result;
        }
        if new_keys == Keys::IDLE {
            return 0;
        }
        const TRANSITION: u16 = key::Y
            | key::X
            | key::UP
            | key::DOWN
            | key::LEFT
            | key::RIGHT
            | key::B
            | key::A;
        if new_keys.any(TRANSITION) && self.input_mode == InputMode::Touch {
            self.input_mode = InputMode::Buttons;
            self.redraw_dialog_focus(self.current_new_game_option as i32);
            return 0;
        }
        if new_keys.any(key::UP | key::DOWN) {
            self.current_new_game_option ^= 1;
            self.redraw_dialog_focus(self.current_new_game_option as i32);
            return 0; // PlaySE — audio deferred.
        }
        let mut ret = 1;
        if new_keys.any(key::A) {
            if self.current_new_game_option != 0 {
                ret = 2;
            }
        } else {
            ret = 2;
            if !new_keys.any(key::B) {
                ret = 0;
            }
        }
        ret
    }

    /// `MainMenu_NewGame_HandleTouchInput` (`:408`): the hitbox pick
    /// — ret 1 on "Begin adventure", 2 on "Return to the menu". The
    /// original's pixel test always passes (pret's own comment:
    /// the hitbox sits fully inside the window), so the port tests
    /// the rect alone.
    fn new_game_handle_touch_input(&self, input: Input, touch_new: bool) -> (bool, u32) {
        if !touch_new {
            return (false, 0);
        }
        let Some(hitbox) = input.touch.and_then(touch_hitbox) else {
            return (false, 0);
        };
        (true, if hitbox == 0 { 1 } else { 2 })
    }
}

/// `TouchscreenHitbox_FindRectAtTouchNew(sNewGameButtonHitboxes)`
/// — the first rect containing the touch, if any.
fn touch_hitbox(touch: Touch) -> Option<usize> {
    NEW_GAME_HITBOXES
        .iter()
        .position(|&(top, bottom, left, right)| {
            u32::from(touch.x) >= u32::from(left)
                && u32::from(touch.x) < u32::from(right)
                && u32::from(touch.y) >= u32::from(top)
                && u32::from(touch.y) < u32::from(bottom)
        })
}

impl App for MainMenu {
    fn tick(&mut self, _frame: crate::Frame, input: Input) {
        let new_keys = input.keys.pressed(self.prev_keys);
        self.prev_keys = input.keys;
        let touch_new = input.touch.is_some() && !self.prev_touch;
        self.prev_touch = input.touch.is_some();

        // data->frames++ (:1394) — the counter is never read; the
        // port keeps no copy.

        // The dialog machine runs ahead of the state switch; a
        // consumed frame skips the switch and the lockout but not
        // the scroll (:1397-1401).
        let consumed = self.handle_dialog(input, new_keys, touch_new);
        let mut finished = false;
        if !consumed {
            // AdvanceButtonBorderAnimation — deferred.
            if self.unk1b4 != 0 {
                self.unk1b4 -= 1;
            }
            match self.state {
                MenuState::SetupGraphics => {
                    self.setup_graphics();
                    self.state = MenuState::Prepare;
                }
                MenuState::Prepare => {
                    // ov74_02227580 always returns 0, so the else
                    // (fade-in to state 2, the backdrop write,
                    // :1417-1420) is dead code.
                    self.state = MenuState::ChooseApp;
                }
                MenuState::ChooseApp => {
                    // unk13C = 12 (:1428) — audio, deferred.
                    if !self.save_exists {
                        self.selected_app = AppOption::NewGame;
                        self.begin_exit_fade(FadeType::BrightnessOut, MenuState::Free);
                    } else {
                        // DetectInsertedGBACart (:1433) — no slot on
                        // this model; connectedAgbGame stays 0.
                        self.state = MenuState::BuildMenu;
                    }
                }
                MenuState::BuildMenu => {
                    // The sprites, wifi tiles, and the vblank CB
                    // (:1438-1440) — deferred.
                    self.build_buttons(input);
                    self.redraw_focus(self.current_option as i32);
                    self.begin_exit_fade(FadeType::BrightnessIn, MenuState::Input);
                    // HW_PLTT[0] = MAIN_MENU_BACKGROUND_COLOR (:1444)
                    // — the engine's backdrop color.
                    self.frame.main.backdrop = BACKGROUND_COLOR;
                    // unk13C = 10 — audio, deferred.
                }
                MenuState::Input => {
                    self.handle_input(input, new_keys);
                    // The unk48 rebuild / unk188 re-arm (:1449-1455)
                    // — discovery, deferred.
                }
                MenuState::DialogDone => {
                    if self.dialog_state == DialogState::Idle {
                        if self.unk150 & u32::from(key::B) != 0 {
                            // The dialog's return edge — back to the
                            // list, no fade (:1458-1460).
                            self.state = MenuState::Input;
                        } else {
                            // Begin adventure — the exit fade
                            // (:1461-1463).
                            self.begin_exit_fade(
                                FadeType::BrightnessOut,
                                MenuState::Free,
                            );
                        }
                    }
                }
                MenuState::Free => {
                    self.free_graphics();
                    finished = true;
                }
                MenuState::WaitFade => {
                    // ov74_022353FC — the poll takes the parked
                    // state.
                    if self.fade.is_finished() {
                        self.state = self.fade_next;
                    }
                }
            }
        }
        // The per-frame tail (:1476-1478) — ov74_022276AC and
        // ov74_022358BC are deferred; the finishing passes (case 7
        // and case 9's ret TRUE) return before the scroll.
        if !finished {
            self.handle_screen_scroll();
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
    fn the_button_table_restates_s_main_menu_buttons() {
        // :175-185 — nine rows, table-position indexed: slot 0 is
        // CONTINUE (APPOPTION_COUNT is APPOPTION_WII_SETTINGS, the
        // count of buttons), NEW_GAME the only NULL printFunction.
        assert_eq!(BUTTONS.len(), SLOT_COUNT);
        let options = BUTTONS.map(|button| button.option);
        assert_eq!(
            options,
            [
                AppOption::Continue,
                AppOption::NewGame,
                AppOption::Pokewalker,
                AppOption::MysteryGift,
                AppOption::Ranger,
                AppOption::MigrateAgb,
                AppOption::ConnectToWii,
                AppOption::Wfc,
                AppOption::WiiSettings,
            ]
        );
        let heights = BUTTONS.map(|button| button.height);
        assert_eq!(heights, [10, 4, 4, 4, 4, 4, 4, 4, 4]);
        // The label ids: CONTINUE 0, NEW GAME 1, POKEWALKER 9,
        // WII SETTINGS 10, WFC 12 (:176-184).
        assert_eq!(BUTTONS[0].msg, 0);
        assert_eq!(BUTTONS[1].msg, 1);
        assert_eq!(BUTTONS[2].msg, 9);
        assert_eq!(BUTTONS[8].msg, 10);
        assert_eq!(BUTTONS[7].msg, 12);
        // MIGRATE_AGB's label is dynamic — the inserted cartridge
        // picks it, so the row carries 0 and never draws here.
        assert_eq!(BUTTONS[5].msg, 0);
        assert!(BUTTONS[1].print.is_none());
    }

    #[test]
    fn the_base_tile_walk_matches_the_build() {
        // data->unk20: 1, then += height * 23 every slot — the
        // window base tiles of a full build (:1093-1125). A clean
        // save draws slots {0, 1, 2, 7, 8}, whose walk values are
        // 1, 231, 323, 783, 875.
        let mut base_tile = 1u16;
        let got = BUTTONS.map(|button| {
            let tile = base_tile;
            base_tile += u16::from(button.height) * 23;
            tile
        });
        assert_eq!(got, [1, 231, 323, 415, 507, 599, 691, 783, 875]);
        // The drawn slots' values, in draw order.
        let drawn = [0, 1, 2, 7, 8].map(|slot| got[slot]);
        assert_eq!(drawn, [1, 231, 323, 783, 875]);
    }

    #[test]
    fn the_availability_gates_match_this_ports_deferrals() {
        // CONTINUE draws unconditionally (its gate is the save
        // blocks its fields read — deferred); WFC, Pokéwalker, and
        // Wii Message Settings draw unconditionally; the discovery,
        // GBA-slot, and Wi-connect gates never open.
        assert!(button_available(PrintFunction::Continue));
        assert!(button_available(PrintFunction::NintendoWfcSetup));
        assert!(button_available(PrintFunction::ConnectToPokewalker));
        assert!(button_available(PrintFunction::WiiMessageSettings));
        assert!(!button_available(PrintFunction::MysteryGift));
        assert!(!button_available(PrintFunction::MigrateFromAgb));
        assert!(!button_available(PrintFunction::ConnectToRanger));
        assert!(!button_available(PrintFunction::ConnectToWii));
    }

    #[test]
    fn the_new_game_hitboxes_cover_the_two_option_rows() {
        // :187-190 — two 24-pixel rows, 32 pixels of margin each
        // side.
        assert_eq!(NEW_GAME_HITBOXES[0], (112, 136, 32, 224));
        assert_eq!(NEW_GAME_HITBOXES[1], (152, 176, 32, 224));
        // Inside the rows picks the option; between them, or in the
        // margin, picks nothing.
        assert_eq!(touch_hitbox(Touch { x: 128, y: 112 }), Some(0));
        assert_eq!(touch_hitbox(Touch { x: 128, y: 135 }), Some(0));
        assert_eq!(touch_hitbox(Touch { x: 128, y: 152 }), Some(1));
        assert_eq!(touch_hitbox(Touch { x: 128, y: 145 }), None);
        assert_eq!(touch_hitbox(Touch { x: 10, y: 120 }), None);
        assert_eq!(touch_hitbox(Touch { x: 250, y: 160 }), None);
    }

    #[test]
    fn the_menu_colors_restate_the_raw_values() {
        // MAIN_MENU_BACKGROUND_COLOR — RGB(12, 12, 31) (:1390).
        assert_eq!(BACKGROUND_COLOR, 0x7D8C);
        // The builder trio's print color — 0x0001020F, and the fill
        // byte the builder prints over.
        assert_eq!(BUTTON_TEXT_COLOR, TextColor::new(1, 2, 15));
        assert_eq!(WINDOW_FILL, 0xF);
    }

    #[test]
    fn the_scroll_constants_restate_their_fx_values() {
        // FX32_ONE 4096; the clamp FX32_CONST(12); the snap
        // FX32_CONST(0.125f); the screen GX_LCD_SIZE_Y.
        assert_eq!(FX32_ONE, 4096);
        assert_eq!(SCROLL_MAX_STEP, 49152);
        assert_eq!(SCROLL_SNAP, 512);
        assert_eq!(SCREEN_HEIGHT, 192);
    }
}