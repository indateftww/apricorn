//! The field start menu — pret `src/start_menu.c` (the field task:
//! the action lists, the input, the top-screen bar and cursor) and
//! the sub-screen icon grid it drives, overlay 27's field UI app
//! (`asm/overlay_27.s`, asm — the ROM is the spec there), Phase 5's
//! first menu slice.
//!
//! **What HeartGold's menu is.** It is not a top-screen list: pressing
//! X turns the *touch* screen into a two-column grid of icon sprites
//! (POKéDEX, POKéMON, BAG, POKéGEAR in the left column, TRAINER CARD,
//! SAVE, OPTIONS in the right) with a centered label under each, a
//! `(X) MENU` header at the top-left, the selected icon on its
//! highlight palette; the top screen keeps the field and gains a
//! 56-pixel bar along its bottom with a bobbing arrow pointing at the
//! touch screen. The C task owns the *what*: `StartMenu_Init`
//! (`:216`) picks the inhibit mask, `StartMenu_BuildActionLists`
//! (`:483`) turns it into the insertion/display lists,
//! `Task_StartMenu_DrawCursor` (`:455`) loads the bar into MAIN BG3
//! and builds the cursor sprite, `StartMenu_HandleKeyInput` (`:591`)
//! selects `selectionToAction[fieldSystem->unkD3]` on A, and the
//! B/X edge closes (`:576-579`). Overlay 27 owns the *where*: the
//! sub-screen BG (`ov27_0225AC00`, `:1561`), the icon sprites at
//! `ov27_0225D038`'s positions (`ov27_0225B010`, `:2054`), the label
//! windows at `ov27_0225D074` (`ov27_0225BCE8`, `:3641`), the d-pad
//! walk over the `ov27_0225D0B4` graph (`ov27_0225B404`, `:2529`),
//! and the compact selection index it writes back into
//! `fieldSystem->unkD3` (`ov27_0225C170`, `:4247`) — the seam the
//! two halves share.
//!
//! **The host seam.** The field system is not ported yet, so the
//! save flags, the player's name and gender, and the map facts come
//! through [`StartMenuHost`]; the scene draws over a caller-supplied
//! base frame and restores it on close. The orchestrator wires the
//! component into the field scene once it exists.
//!
//! Deferrals, each honest in the frame: audio (`PlaySE`), the
//! launched apps (the event names the action; Save's touch save app,
//! the union-room CHAT/LOG, the RETIRE script), the closed-state
//! sub-screen UI (registered items, the running shoes toggle, the
//! A-button hint — overlay 27's `ov27_0225A690(1)` look), the
//! pop-in animation of newly unlocked icons (`ov27_0225AAD4`, a
//! closed-state pass), the safari/bug-contest/pal-park ball counters
//! (`ov27_0225C0E0`), the per-sprite OAM-mode override
//! `Sprite_SetOamMode` (`ov27_0225A8E8`, `:1170`, dims every
//! unselected icon through the sub blend unit at EVA 6 / EVB 9 — the
//! frame model's [`Sprite`] carries no OAM-mode override, so the
//! register state is carried and the icons render opaque), the
//! `MenuInputStateMgr` touch/buttons memory beyond this open, and
//! `fieldSystem->unk90`'s write-only `lastButtonSelected`.

use std::sync::Mutex;

use crate::app::fade::{BrightnessFade, FadeColor, FadeType};
use crate::app::text::{TEXT_SPEED_INSTANT, TEXT_SPEED_NOTRANSFER, TextFlags, TextPrinter};
use crate::assets::{AssetStore, AssetsError, font_narc, msg_narc, start_menu as narc};
use crate::font::Font;
use crate::frame::{
    AssetId, BgLayer, Blend, BlendEffect, ColorMode, EngineFrame, LogicalFrame, PaletteLoad,
    ScreenSize, Sprite, TextColor, TilePlacement, Window, plane,
};
use crate::input::{Input, Keys, Touch, key};
use crate::text::format::MessageFormat;
use crate::text::string::GameString;

/// `enum StartMenuAction` (`start_menu.c:48-62`) — the thirteen
/// entries of `sStartMenuActions`, in table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartMenuAction {
    /// `START_MENU_ACTION_POKEDEX` — label `msg_0196_00000`.
    Pokedex,
    /// `START_MENU_ACTION_POKEMON` — `msg_0196_00001`.
    Pokemon,
    /// `START_MENU_ACTION_BAG` — `msg_0196_00002`.
    Bag,
    /// `START_MENU_ACTION_TRAINER_CARD` — `msg_0196_00003`, the
    /// player's name.
    TrainerCard,
    /// `START_MENU_ACTION_SAVE` — `msg_0196_00004`.
    Save,
    /// `START_MENU_ACTION_OPTIONS` — `msg_0196_00005`.
    Options,
    /// `START_MENU_ACTION_RUNNING_SHOES` — `msg_0196_00006` (EXIT);
    /// its func is `STARTMENUTASKFUNC_CANCEL`, the close.
    RunningShoes,
    /// `START_MENU_ACTION_7` — `msg_0196_00007`, the union-room
    /// easy-chat entry (`Task_StartMenu_HandleSelection_RemovedEasyChatThing`).
    Action7,
    /// `START_MENU_ACTION_RETIRE` — `msg_0196_00008`.
    Retire,
    /// `START_MENU_ACTION_9` — `msg_0196_00014`, forced into display
    /// slot 7 by `StartMenu_BuildActionLists` (`:518`).
    Action9,
    /// `START_MENU_ACTION_10` — `msg_0196_00014`, forced into display
    /// slot 8 (`:519`).
    Action10,
    /// `START_MENU_ACTION_POKEGEAR` — `msg_0196_00014`.
    Pokegear,
    /// `START_MENU_ACTION_12` — `msg_0196_00014`, the union-room LOG
    /// (`sub_0203D2CC`, `:1158`).
    Action12,
}

impl StartMenuAction {
    /// The table, in enum order.
    pub const ALL: [Self; 13] = [
        Self::Pokedex,
        Self::Pokemon,
        Self::Bag,
        Self::TrainerCard,
        Self::Save,
        Self::Options,
        Self::RunningShoes,
        Self::Action7,
        Self::Retire,
        Self::Action9,
        Self::Action10,
        Self::Pokegear,
        Self::Action12,
    ];

    /// The enum value (the `u8` the action lists store).
    #[must_use]
    pub fn index(self) -> u8 {
        Self::ALL
            .iter()
            .position(|&a| a == self)
            .expect("every action is in ALL") as u8
    }

    /// The action at enum value `index`.
    #[must_use]
    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }

    /// `sActionToIconIndex[action]` (`:161-174`) — the icon whose
    /// unlock gates the action, or `None` for the table's `100`
    /// entries (always available).
    #[must_use]
    pub fn icon(self) -> Option<StartMenuIcon> {
        match self {
            Self::Pokedex => Some(StartMenuIcon::Pokedex),
            Self::Pokemon => Some(StartMenuIcon::Pokemon),
            Self::Bag => Some(StartMenuIcon::Bag),
            Self::TrainerCard => Some(StartMenuIcon::TrainerCard),
            Self::Save => Some(StartMenuIcon::Save),
            Self::Options => Some(StartMenuIcon::Options),
            Self::Pokegear => Some(StartMenuIcon::Pokegear),
            Self::RunningShoes
            | Self::Action7
            | Self::Retire
            | Self::Action9
            | Self::Action10
            | Self::Action12 => None,
        }
    }

    /// `sStartMenuActions[action].ident` (`:176-190`) — the entry's
    /// label in message bank 196.
    #[must_use]
    pub fn label_msg(self) -> usize {
        match self {
            Self::Pokedex => 0,
            Self::Pokemon => 1,
            Self::Bag => 2,
            Self::TrainerCard => 3,
            Self::Save => 4,
            Self::Options => 5,
            Self::RunningShoes => 6,
            Self::Action7 => 7,
            Self::Retire => 8,
            Self::Action9 | Self::Action10 | Self::Pokegear | Self::Action12 => 14,
        }
    }
}

/// `enum StartMenuIcon` (`include/start_menu.h`) — the eight unlock
/// gates `FieldSystem_ShouldDrawStartMenuIcon` (`:535`) knows; 0–6
/// are also the sub-screen grid's slots in the normal layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartMenuIcon {
    /// `CheckGotPokedex`.
    Pokedex,
    /// `CheckGotStarter`.
    Pokemon,
    /// `CheckGotMenuIconI(START_MENU_ICON_UNLOCK_BAG)`.
    Bag,
    /// `CheckGotPokegear`.
    Pokegear,
    /// `CheckGotMenuIconI(START_MENU_ICON_UNLOCK_TRAINER_CARD)`.
    TrainerCard,
    /// `CheckGotMenuIconI(START_MENU_ICON_UNLOCK_SAVE_BUTTON)`.
    Save,
    /// `CheckGotMenuIconI(START_MENU_ICON_UNLOCK_OPTIONS_BUTTON)`.
    Options,
    /// `PlayerSaveData_CheckRunningShoes`.
    RunningShoes,
}

impl StartMenuIcon {
    /// The eight icons, in enum order.
    pub const ALL: [Self; 8] = [
        Self::Pokedex,
        Self::Pokemon,
        Self::Bag,
        Self::Pokegear,
        Self::TrainerCard,
        Self::Save,
        Self::Options,
        Self::RunningShoes,
    ];
}

/// `enum StartMenuActionDisable` (`start_menu.c:64-75`) — the bits of
/// `StartMenuTaskData.inhibitIconFlags`.
pub mod disable {
    /// `START_MENU_ACTION_DISABLE_POKEDEX`.
    pub const POKEDEX: u32 = 1 << 0;
    /// `START_MENU_ACTION_DISABLE_POKEMON`.
    pub const POKEMON: u32 = 1 << 1;
    /// `START_MENU_ACTION_DISABLE_BAG`.
    pub const BAG: u32 = 1 << 2;
    /// `START_MENU_ACTION_DISABLE_TRAINER_CARD`.
    pub const TRAINER_CARD: u32 = 1 << 3;
    /// `START_MENU_ACTION_DISABLE_SAVE`.
    pub const SAVE: u32 = 1 << 4;
    /// `START_MENU_ACTION_DISABLE_OPTIONS`.
    pub const OPTIONS: u32 = 1 << 5;
    /// `START_MENU_ACTION_DISABLE_RUNNING_SHOES`.
    pub const RUNNING_SHOES: u32 = 1 << 6;
    /// `START_MENU_ACTION_DISABLE_7`.
    pub const ACTION_7: u32 = 1 << 7;
    /// `START_MENU_ACTION_DISABLE_RETIRE`.
    pub const RETIRE: u32 = 1 << 8;
    /// `START_MENU_ACTION_DISABLE_POKEGEAR`.
    pub const POKEGEAR: u32 = 1 << 9;
}

/// The save flags the menu's gates read (`include/constants/flags.h`).
pub mod flag {
    /// `FLAG_GOT_STARTER` — `CheckGotStarter` (`sys_flags.c:273`).
    pub const GOT_STARTER: u16 = 0x6A;
    /// `FLAG_GOT_POKEDEX` — `CheckGotPokedex` (`:281`).
    pub const GOT_POKEDEX: u16 = 0x6B;
    /// `FLAG_GOT_POKEGEAR` — `CheckGotPokegear` (`:277`).
    pub const GOT_POKEGEAR: u16 = 0x9C;
    /// `FLAG_GOT_BAG` — `CheckGotMenuIconI(0)` (`:285`, `FLAG_GOT_BAG +
    /// icon_idx`); the trainer card, save button, and options button
    /// are the next three.
    pub const GOT_BAG: u16 = 0x11B;
    /// `FLAG_GOT_TRAINER_CARD` — `CheckGotMenuIconI(1)`.
    pub const GOT_TRAINER_CARD: u16 = 0x11C;
    /// `FLAG_GOT_SAVE_BUTTON` — `CheckGotMenuIconI(2)`.
    pub const GOT_SAVE_BUTTON: u16 = 0x11D;
    /// `FLAG_GOT_OPTIONS_BUTTON` — `CheckGotMenuIconI(3)`.
    pub const GOT_OPTIONS_BUTTON: u16 = 0x11E;
    /// `FLAG_SYS_SAFARI` — `Save_VarsFlags_CheckSafariSysFlag` (`:212`).
    pub const SYS_SAFARI: u16 = 0x967;
    /// `FLAG_SYS_PAL_PARK` — `Save_VarsFlags_CheckPalParkSysFlag` (`:228`).
    pub const SYS_PAL_PARK: u16 = 0x971;
    /// `FLAG_UNK_996` — `Save_VarsFlags_CheckBugContestFlag` (`:216`).
    pub const BUG_CONTEST: u16 = 0x996;
}

/// `fieldSystem->mapLoadType` (`include/constants/field/map_load.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapLoadType {
    /// `MAP_LOAD_TYPE_OVERWORLD` (0).
    #[default]
    Overworld,
    /// `MAP_LOAD_TYPE_SAFARI` (1).
    Safari,
    /// `MAP_LOAD_TYPE_UNION` (2).
    Union,
    /// `MAP_LOAD_TYPE_COLOSSEUM` (3).
    Colosseum,
    /// `MAP_LOAD_TYPE_BATTLE_TOWER` (4).
    BattleTower,
    /// `MAP_LOAD_TYPE_5`.
    Type5,
}

/// The field facts the menu reads — the save's flags and vars, the
/// player profile, and the map — supplied by whoever owns the field
/// (the orchestrator's field scene, or a test mock).
///
/// The `flag`/`var` primitives are the `CheckScriptFlag` /
/// `GetScriptVar` seam; the named accessors are pret's helpers over
/// them and take their defaults from the [`flag`] ids. `var` and
/// `party_count` are not read by `start_menu.c`'s gates (the POKéMON
/// entry gates on `FLAG_GOT_STARTER`, not the party) — they are the
/// seam later menu slices (the party and bag screens) read.
pub trait StartMenuHost {
    /// `CheckScriptFlag(state, id)`.
    fn flag(&self, id: u16) -> bool;
    /// `GetScriptVar(state, id)`.
    fn var(&self, id: u16) -> u16;
    /// `Party_GetCount(SaveArray_Party_Get(saveData))`.
    fn party_count(&self) -> u8;
    /// `PlayerProfile_GetNamePtr` — the TRAINER CARD label's
    /// `{STRVAR_1 3, 0, 0}` (`ov27_0225BB6C`'s `BufferPlayersName`).
    fn player_name(&self) -> GameString;
    /// `PlayerProfile_GetTrainerGender` — 0 male, 1 female (the BAG
    /// icon's char, `ov27_0225AEA8:1918-1922`).
    fn player_gender(&self) -> u8;
    /// `PlayerSaveData_CheckRunningShoes` — icon 7's gate.
    fn has_running_shoes(&self) -> bool;
    /// `fieldSystem->mapLoadType`.
    fn map_load_type(&self) -> MapLoadType {
        MapLoadType::Overworld
    }
    /// `fieldSystem->bottomScreenType` — 3 picks overlay 27's
    /// union-room layout row (`ov27_0225BD50:3794-3798`).
    fn bottom_screen_type(&self) -> i32 {
        0
    }
    /// `MapHeader_MapIsAmitySquare(location->mapId)` — no HeartGold
    /// map is; the normal inhibit mask reads it (`:302-304`).
    fn map_is_amity_square(&self) -> bool {
        false
    }
    /// `FieldSystem_MapIsBattleTowerMultiPartnerSelectRoom`.
    fn map_is_battle_tower_multi_partner_select_room(&self) -> bool {
        false
    }
    /// `fieldSystem->unkD3` as the field left it — the compact
    /// selection index the grid restores on open
    /// (`ov27_02259F80:122-126`); 0 on a fresh field system.
    fn last_menu_selection(&self) -> u8 {
        0
    }
    /// `CheckGotPokedex`.
    fn has_pokedex(&self) -> bool {
        self.flag(flag::GOT_POKEDEX)
    }
    /// `CheckGotStarter`.
    fn has_starter(&self) -> bool {
        self.flag(flag::GOT_STARTER)
    }
    /// `CheckGotPokegear`.
    fn has_pokegear(&self) -> bool {
        self.flag(flag::GOT_POKEGEAR)
    }
    /// `CheckGotMenuIconI(icon_idx)` — `icon_idx < 4`: bag, trainer
    /// card, save button, options button.
    fn menu_icon_unlocked(&self, icon_idx: u16) -> bool {
        debug_assert!(icon_idx < 4, "CheckGotMenuIconI asserts icon_idx < 4");
        self.flag(flag::GOT_BAG + icon_idx)
    }
    /// `Save_VarsFlags_CheckSafariSysFlag`.
    fn safari_active(&self) -> bool {
        self.flag(flag::SYS_SAFARI)
    }
    /// `Save_VarsFlags_CheckBugContestFlag`.
    fn bug_contest_active(&self) -> bool {
        self.flag(flag::BUG_CONTEST)
    }
    /// `Save_VarsFlags_CheckPalParkSysFlag`.
    fn pal_park_active(&self) -> bool {
        self.flag(flag::SYS_PAL_PARK)
    }
}

/// `FieldSystem_ShouldDrawStartMenuIcon` (`:535-558`) over the host.
#[must_use]
pub fn should_draw_icon(host: &dyn StartMenuHost, icon: StartMenuIcon) -> bool {
    match icon {
        StartMenuIcon::Pokedex => host.has_pokedex(),
        StartMenuIcon::Pokemon => host.has_starter(),
        StartMenuIcon::Bag => host.menu_icon_unlocked(0),
        StartMenuIcon::Pokegear => host.has_pokegear(),
        StartMenuIcon::TrainerCard => host.menu_icon_unlocked(1),
        StartMenuIcon::Save => host.menu_icon_unlocked(2),
        StartMenuIcon::Options => host.menu_icon_unlocked(3),
        StartMenuIcon::RunningShoes => host.has_running_shoes(),
    }
}

/// `FieldSystem_StartMenuActionIsAvailable` (`:531`) —
/// `ShouldDrawStartMenuIcon(sActionToIconIndex[action])`, the `100`
/// entries defaulting to available.
#[must_use]
pub fn action_available(host: &dyn StartMenuHost, action: StartMenuAction) -> bool {
    action
        .icon()
        .is_none_or(|icon| should_draw_icon(host, icon))
}

/// What one tick of the open menu produced for its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMenuEvent {
    /// Still open (or already finished — every tick after a terminal
    /// event is a no-op).
    None,
    /// The menu closed this tick — `START_MENU_STATE_CLOSE`'s pass
    /// (`:429-436`): the top-screen bar and cursor are gone and the
    /// base frame is restored. The tick after a B/X edge, an EXIT
    /// pick, or the union-room LOG's two-state exit.
    Closed,
    /// The tick `start_menu.c` hands control to the picked action:
    /// after the six-step brightness fade for the app launches
    /// (`Task_StartMenu_WaitFade`, `:707` — the frame is black, the
    /// bar cleared), immediately for SAVE (the sub-screen app switch,
    /// `:1126`), RETIRE (the script start, `:1252`, bar cleared) and
    /// the union-room LOG (`:1158`, which then closes two ticks
    /// later).
    Selected(StartMenuAction),
}

/// The scene's states — the `StartMenuState` values this slice walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// `START_MENU_STATE_HANDLE_INPUT`.
    HandleInput,
    /// `START_MENU_STATE_WAIT_FADE` — the fade-out before the launch.
    WaitFade(StartMenuAction),
    /// `START_MENU_STATE_19` (`sub_0203D2EC`, `:1167`).
    State19,
    /// `START_MENU_STATE_20` (`sub_0203D304`, `:1174`).
    State20,
    /// `START_MENU_STATE_CLOSE`.
    Close,
    /// The task returned TRUE (or handed off to an app).
    Done,
}

/// `ov27_0225B404`'s direction order (`:2529-2551`): UP, DOWN, LEFT,
/// RIGHT — the first held new key wins in that order.
const DIRECTIONS: [(u16, usize); 4] = [
    (key::UP, 0),
    (key::DOWN, 1),
    (key::LEFT, 2),
    (key::RIGHT, 3),
];

/// `ov27_0225D0B4` (`overlay_27.s:6073-6079`) — the d-pad walk: per
/// grid slot, per direction, three candidate slots tried in order
/// (`ov27_0225B360`, `:2440`, takes the first visible one).
const NAV: [[[u8; 3]; 4]; 7] = [
    [[3, 2, 1], [1, 2, 3], [4, 0, 0], [4, 0, 0]],
    [[0, 3, 2], [2, 3, 0], [5, 1, 0], [5, 1, 0]],
    [[1, 0, 3], [3, 0, 1], [6, 2, 0], [6, 2, 0]],
    [[2, 1, 0], [0, 1, 2], [6, 3, 0], [6, 3, 0]],
    [[6, 5, 4], [5, 6, 4], [0, 4, 0], [0, 4, 0]],
    [[4, 6, 5], [6, 4, 5], [1, 5, 0], [1, 5, 0]],
    [[5, 4, 6], [4, 5, 6], [2, 6, 0], [2, 6, 0]],
];

/// `ov27_0225D038` (`:6056-6059`) — the seven icon sprites' origins
/// (the registered-item pair at (220, 11)/(220, 51) follows; closed
/// state, not here).
const ICON_POSITIONS: [(i16, i16); 7] = [
    (24, 22),
    (24, 62),
    (24, 102),
    (24, 142),
    (104, 22),
    (104, 62),
    (104, 102),
];

/// `ov27_0225D074` (`:6066-6071`) — the label windows' (left, top) in
/// tiles; `ov27_0225AC00` adds each 9×2 at palette 4, base tile
/// `0xF6 + 0x12·i` (`:1660-1676`).
const LABEL_WINDOWS: [(u8, u8); 8] = [
    (1, 6),
    (1, 11),
    (1, 16),
    (1, 21),
    (11, 6),
    (11, 11),
    (11, 16),
    (11, 21),
];
/// The label windows' width in tiles (`ov27_0225AC00`, `mov r0, #9`).
const LABEL_WIDTH: u8 = 9;
/// The label windows' height in tiles.
const LABEL_HEIGHT: u8 = 2;
/// The label windows' palette bank (`mov r0, #4`).
const LABEL_PALETTE: u8 = 4;
/// The first label window's base tile (`mov r6, #0xf6`).
const LABEL_BASE_TILE: u16 = 0xF6;
/// The base-tile stride between label windows (`add r6, #0x12`).
const LABEL_TILE_STRIDE: u16 = 0x12;
/// `ov27_0225BCE8`'s print color `0x000E0200` — `MAKE_TEXT_COLOR(14, 2, 0)`.
const LABEL_COLOR: TextColor = TextColor::new(14, 2, 0);
/// `ov27_0225BCE8`'s centering width — `0x48`, the window's 72 pixels.
const LABEL_CENTER_WIDTH: i32 = 0x48;

/// The `MENU` header window — `ov27_0225AC00:1633-1640`: SUB_1, left
/// 9, top 0, 10×2, palette 4, base tile `0xE2`.
const HEADER_WINDOW: (u8, u8, u8, u8, u16) = (9, 0, 10, 2, 0xE2);
/// The header's print color `0x000F0100` — `MAKE_TEXT_COLOR(15, 1, 0)`
/// (`ov27_02259F80:145-160`, font 4).
const HEADER_COLOR: TextColor = TextColor::new(15, 1, 0);
/// The header's message — `msg_0196_00012` (`ov27_0225BB6C:3503`).
const HEADER_MSG: usize = 12;
/// The `(X)` glyph beside the header — sprite 14 of `ov27_0225B010`,
/// at `ov27_0225D05C[5]` (54, 0), sequence 12 (`:2300-2306`).
const HEADER_GLYPH: (i16, i16, usize) = (54, 0, 12);

/// `ov27_0225CFC8` (`:6038-6045`) — per layout row, the resource index
/// shown in each of the eight sub-screen slots (`0xD` is empty).
const LAYOUT_SLOTS: [[u8; 8]; 7] = [
    [0, 1, 2, 3, 4, 5, 6, 0xD],
    [7, 0, 1, 2, 3, 4, 6, 8],
    [7, 0, 1, 3, 4, 6, 0xA, 0xD],
    [7, 0, 1, 3, 4, 6, 9, 0xD],
    [0xB, 0, 1, 2, 0xC, 4, 6, 0xD],
    [1, 2, 4, 6, 0xD, 0xD, 0xD, 0xD],
    [1, 4, 6, 0xD, 0xD, 0xD, 0xD, 0xD],
];
/// The empty resource index.
const RESOURCE_NONE: u8 = 0xD;

/// `ov27_0225CF94` (`:6023-6036`) — per resource index, the label
/// message in bank 196 and the `0x8000` "is a selectable icon" bit
/// (`ov27_0225C10C`, `:4194`, reads it for resources ≥ 7).
const RESOURCE_LABELS: [u16; 13] = [
    0 | 0x8000,
    1 | 0x8000,
    2 | 0x8000,
    14 | 0x8000,
    3 | 0x8000,
    4 | 0x8000,
    5 | 0x8000,
    8 | 0x8000,
    32,
    32,
    32,
    34 | 0x8000,
    35 | 0x8000,
];

/// `ov27_0225CF68` (`:6017-6021`) — the open-menu touch hitboxes as
/// `(top, bottom, left, right)`: the header strip (closes), the seven
/// icon slots, the two registered-item buttons.
const HITBOXES: [(u8, u8, u8, u8); 10] = [
    (0x00, 0x10, 0x08, 0xA0),
    (0x16, 0x36, 0x10, 0x4C),
    (0x3E, 0x5E, 0x10, 0x4C),
    (0x66, 0x86, 0x10, 0x4C),
    (0x8E, 0xAE, 0x10, 0x4C),
    (0x16, 0x36, 0x60, 0x9C),
    (0x3E, 0x5E, 0x60, 0x9C),
    (0x66, 0x86, 0x60, 0x9C),
    (0x08, 0x27, 0xCB, 0xFF),
    (0x2E, 0x4D, 0xCB, 0xFF),
];

/// The icon sprites' animation sequences (member 17): idle, the
/// left/right bounce, the pressed jump, the up/down bounce.
const ICON_SEQ_IDLE: usize = 0;
const ICON_SEQ_LEFT_RIGHT: usize = 1;
const ICON_SEQ_PRESSED: usize = 2;
const ICON_SEQ_UP_DOWN: usize = 3;

/// The field's MAIN BG3 — `sBgTemplate_3` (`src/field/fieldmap.c:479`):
/// 256×256 4bpp, screen base `0x1000`, char base `0x08000`, priority 0.
const MAIN3_BLOCK: u8 = (0x08000 / 0x8000) as u8;
const MAIN3_PRIORITY: u8 = 0;
/// Overlay 27's two sub-screen layers — `ov27_0225D000`/`ov27_0225D01C`
/// (`:6047-6054`): both 256×256 4bpp, char base 0; SUB_0 at screen
/// base `0xD`, priority 2; SUB_1 at `0xE`, priority 0.
const SUB0_PRIORITY: u8 = 2;
const SUB1_PRIORITY: u8 = 0;
/// `StartMenu_CreateCursor`'s position (`:698-699`) — `FX32_CONST(100)`,
/// `FX32_CONST(144)`.
const CURSOR_POSITION: (i16, i16) = (100, 144);
/// `FieldMap_FadeScreen` (`fieldmap.c:644-652`) — six steps of one
/// frame, both screens, black.
const FADE_STEPS: i32 = 6;
const FADE_FRAMES_PER_STEP: u8 = 1;
/// `G2x_SetBlendAlpha_(&reg_G2S_BLDCNT, 0, BG0|BG1|BD, 6, 9)`
/// (`ov27_0225A8E8:1257-1264`) — the open menu's sub blend unit.
const SUB_BLEND: Blend = Blend {
    plane1: 0,
    effect: BlendEffect::Alpha,
    plane2: plane::BG0 | plane::BG1 | plane::BD,
    eva: 6,
    ebv: 9,
    evy: 0,
};

/// One icon slot's sprite assets and animation state.
#[derive(Debug, Clone)]
struct IconSlot {
    /// The resource index shown here (`ov27_0225CFC8[layout][slot]`).
    resource: u8,
    /// `data+0x514[slot]` / `data+0x470[slot*8]` — drawn and navigable.
    visible: bool,
    /// The char loaded for the resource, when it is an icon.
    tiles: Option<AssetId>,
    /// The label, expanded (`data+0x474[slot*8]`), when the resource
    /// has one.
    label: Option<GameString>,
    /// The sprite's animation sequence (`Sprite_SetAnimCtrlSeq`).
    sequence: usize,
    /// Ticks since the sequence was set.
    elapsed: u32,
}

/// The field start menu, open over a base frame.
pub struct StartMenu {
    /// The caller's frame the menu draws over and restores on close.
    base: LogicalFrame,
    /// The logical frame the last tick produced.
    frame: LogicalFrame,
    /// `FieldMap_FadeScreen`'s both-screen brightness fade.
    fade: BrightnessFade,
    /// The label printers' flag block.
    flags: TextFlags,
    /// Font 0 (the labels) and font 4 (the header), cloned at open.
    font0: Font,
    font4: Font,
    font0_asset: AssetId,
    font4_asset: AssetId,
    focus_asset: AssetId,
    // Top screen.
    top_char: AssetId,
    top_screen: AssetId,
    top_palette: AssetId,
    cursor_char: AssetId,
    cursor_palette: AssetId,
    cursor_cells: AssetId,
    cursor_anim: AssetId,
    /// `SpriteList_RenderAndAnimateSprites`'s ticks on the cursor.
    cursor_elapsed: u32,
    /// `sub_0203C38C` ran — bar tilemap cleared, cursor gone.
    top_cleared: bool,
    // Sub screen.
    sub_char: AssetId,
    sub_screen: AssetId,
    sub_palette: AssetId,
    icon_cells: AssetId,
    icon_anim: AssetId,
    icon_palette: AssetId,
    sheet_char: AssetId,
    sheet_palette: AssetId,
    sheet_cells: AssetId,
    sheet_anim: AssetId,
    /// The `(X)` glyph's animation ticks.
    glyph_elapsed: u32,
    /// The seven grid slots.
    slots: [IconSlot; 7],
    /// Which of the eight label windows `ov27_0225BC84` copied to
    /// VRAM — `ShouldDrawStartMenuIcon(i)` by *icon* index, as the
    /// asm does even in the layouts where slot ≠ icon.
    label_shown: [bool; 8],
    /// Whether the header window and `(X)` glyph show (any of the
    /// eight icon gates open, `ov27_0225BC84:3626-3639`).
    header_shown: bool,
    /// The `MENU` header string.
    header: GameString,
    /// `ov27_0225BD50`'s layout row.
    layout: usize,
    /// `data+0x14` — the grid slot under the cursor, `-1` when none.
    selected_slot: i32,
    // The C task.
    /// `StartMenuTaskData.inhibitIconFlags`.
    inhibit: u32,
    /// `StartMenuTaskData.unk_350` — the union-room variant.
    unk_350: bool,
    /// `StartMenuTaskData.insertionOrder`.
    insertion_order: [u8; 10],
    /// `StartMenuTaskData.selectionToAction` — the display order.
    selection_to_action: [u8; 10],
    /// `StartMenuTaskData.numActiveButtons`.
    num_active: u32,
    /// `fieldSystem->unkD3` — the compact selection index.
    unk_d3: u8,
    /// `fieldSystem->lastTouchMenuInput`.
    last_touch_menu_input: u16,
    /// `fieldSystem->lastStartMenuAction`, once a pick ran.
    last_action: Option<StartMenuAction>,
    /// `menuInputState` — TOUCH after a touch pick, BUTTONS after A.
    input_touch_mode: bool,
    /// `ov27`'s `data+0x51C` bit 5 (`ov01_021F6B50` → `ov27_0225A2CC`):
    /// the pressed animation is due on the selected icon.
    pressed_pending: bool,
    /// The task state.
    state: State,
    /// The keys held on the previous tick, for press edges.
    prev_keys: Keys,
    /// Whether the stylus was down on the previous tick.
    prev_touch: bool,
}

impl StartMenu {
    /// `StartMenu_Init` (`:216`) — the X-button open: the inhibit mask
    /// by the safari / bug-contest / pal-park flags and the battle
    /// tower room, else the normal flag-gated mask; the C task's
    /// `INIT` → `DrawCursor` pass and overlay 27's open pass both
    /// land in the constructed frame.
    ///
    /// `base` is the field's frame as it stands: the menu adds MAIN
    /// BG3's bar, the cursor sprite, and replaces the sub engine with
    /// the grid; [`StartMenuEvent::Closed`] restores it.
    ///
    /// # Errors
    /// Returns the store's [`AssetsError`] when a member is missing or
    /// corrupt — unreachable in practice against the pinned dump.
    pub fn open(
        store: &Mutex<AssetStore>,
        host: &dyn StartMenuHost,
        base: &LogicalFrame,
    ) -> Result<Self, AssetsError> {
        let inhibit = if host.safari_active() {
            inhibit_safari()
        } else if host.bug_contest_active() {
            inhibit_bug_contest()
        } else if host.pal_park_active() {
            inhibit_pal_park()
        } else if host.map_is_battle_tower_multi_partner_select_room() {
            inhibit_battle_tower_multi_partner_select_room()
        } else {
            inhibit_normal(host)
        };
        Self::open_with(store, host, base, inhibit, false)
    }

    /// `sub_0203BD20` (`:246`) — the colosseum open: `sub_0203BEE8`'s
    /// mask (no dex, save, easy chat, retire, or gear).
    ///
    /// # Errors
    /// As [`Self::open`].
    pub fn open_colosseum(
        store: &Mutex<AssetStore>,
        host: &dyn StartMenuHost,
        base: &LogicalFrame,
    ) -> Result<Self, AssetsError> {
        Self::open_with(store, host, base, inhibit_colosseum(), false)
    }

    /// `sub_0203BCDC` (`:236`) — the union-room open: `sub_0203BEE0`'s
    /// mask (no save or retire) with `unk_350` set, so the gear slot
    /// carries the LOG (`START_MENU_ACTION_12`).
    ///
    /// # Errors
    /// As [`Self::open`].
    pub fn open_union_room(
        store: &Mutex<AssetStore>,
        host: &dyn StartMenuHost,
        base: &LogicalFrame,
    ) -> Result<Self, AssetsError> {
        Self::open_with(store, host, base, inhibit_union(), true)
    }

    fn open_with(
        store: &Mutex<AssetStore>,
        host: &dyn StartMenuHost,
        base: &LogicalFrame,
        inhibit: u32,
        unk_350: bool,
    ) -> Result<Self, AssetsError> {
        // StartMenu_BuildActionLists (:483) — the C's lists.
        let (insertion_order, selection_to_action, num_active) =
            build_action_lists(inhibit, unk_350);
        // ov27_0225BD50 (:3699) — the sub-screen layout row.
        let layout = layout_row(host);

        let mut store = store
            .lock()
            .expect("the asset store is only locked at scene construction");
        let font0_asset = store.load_font(font_narc::NARC, font_narc::FONT0)?;
        let font4_asset = store.load_font(font_narc::NARC, font_narc::FONT4)?;
        let focus_asset = store.load_tiles(font_narc::NARC, font_narc::FOCUS_INDICATOR)?;
        let font0 = store
            .font(font0_asset)
            .expect("the just-loaded font")
            .clone();
        let font4 = store
            .font(font4_asset)
            .expect("the just-loaded font")
            .clone();
        // Task_StartMenu_DrawCursor (:465-467) + StartMenu_CreateCursor (:668-671).
        let top_char = store.load_tiles(narc::NARC, narc::TOP_BAR_CHAR)?;
        let top_screen = store.load_screen(narc::NARC, narc::TOP_BAR_SCREEN)?;
        let top_palette = store.load_palette(narc::NARC, narc::TOP_BAR_PALETTE)?;
        let cursor_char = store.load_tiles(narc::NARC, narc::CURSOR_CHAR)?;
        let cursor_palette = store.load_palette(narc::NARC, narc::CURSOR_PALETTE)?;
        let cursor_cells = store.load_cells(narc::NARC, narc::CURSOR_CELLS)?;
        let cursor_anim = store.load_animation(narc::NARC, narc::CURSOR_ANIM)?;
        // ov27_0225AC00 (:1561) — the sub-screen BG; ov27_0225AD0C (:1687)
        // — the shared icon cells/anims and the button sheet.
        let sub_char = store.load_tiles(narc::NARC, narc::SUB_BG_CHAR)?;
        let sub_screen = store.load_screen(narc::NARC, narc::SUB_BG_SCREEN)?;
        let sub_palette = store.load_palette(narc::NARC, narc::SUB_BG_PALETTE)?;
        let icon_cells = store.load_cells(narc::NARC, narc::ICON_CELLS)?;
        let icon_anim = store.load_animation(narc::NARC, narc::ICON_ANIM)?;
        let icon_palette = store.load_palette(narc::NARC, narc::ICON_PALETTE)?;
        let sheet_char = store.load_tiles(narc::NARC, narc::SHEET_CHAR)?;
        let sheet_palette = store.load_palette(narc::NARC, narc::SHEET_PALETTE)?;
        let sheet_cells = store.load_cells(narc::NARC, narc::SHEET_CELLS)?;
        let sheet_anim = store.load_animation(narc::NARC, narc::SHEET_ANIM)?;
        // ov27_02259F80:96 — the menu's message bank; ov27_0225BB6C
        // (:3455) — the player's name into field 0, every slot's label
        // expanded, the MENU header.
        let bank = store.load_msg_bank(msg_narc::NARC, narc::MSG_BANK)?;
        let mut format = MessageFormat::new(8);
        format.set_string(0, &host.player_name());
        let (header, labels) = {
            let text = store.msg_bank(bank).expect("the just-loaded bank");
            let header = format
                .expand_placeholders(text.message(HEADER_MSG).expect("the MENU header"))
                .expect("the header expands");
            // ov27_0225BB6C: the label for every non-empty slot.
            let labels: [Option<GameString>; 7] = std::array::from_fn(|slot| {
                let resource = LAYOUT_SLOTS[layout][slot];
                if resource == RESOURCE_NONE {
                    return None;
                }
                let msg = usize::from(RESOURCE_LABELS[usize::from(resource)] & 0x7FFF);
                let units = text.message(msg).expect("bank 196 carries the label");
                Some(
                    format
                        .expand_placeholders(units)
                        .expect("the label expands"),
                )
            });
            (header, labels)
        };
        let gender = host.player_gender();
        let mut slots: [IconSlot; 7] = std::array::from_fn(|_| IconSlot {
            resource: RESOURCE_NONE,
            visible: false,
            tiles: None,
            label: None,
            sequence: ICON_SEQ_IDLE,
            elapsed: 0,
        });
        for (slot, (entry, label)) in slots.iter_mut().zip(labels).enumerate() {
            let resource = LAYOUT_SLOTS[layout][slot];
            entry.resource = resource;
            if resource == RESOURCE_NONE {
                continue;
            }
            let resource_index = usize::from(resource);
            // ov27_0225C10C (:4194): icons 0-6 gate on ShouldDrawStartMenuIcon,
            // the rest on the label table's 0x8000 bit.
            entry.visible = if resource < 7 {
                should_draw_icon(host, StartMenuIcon::ALL[resource_index])
            } else {
                RESOURCE_LABELS[resource_index] & 0x8000 != 0
            };
            entry.label = label;
            // ov27_0225AEA8 (:1886): the char member — the BAG by gender,
            // the bug-contest mon (0xA, an item/mon icon) not here.
            let member = match resource {
                2 if gender == 1 => narc::BAG_FEMALE_CHAR,
                _ => narc::ICON_CHARS[resource_index],
            };
            if member != usize::MAX {
                entry.tiles = Some(store.load_tiles(narc::NARC, member)?);
            }
        }
        drop(store);

        // ov27_0225BC84 (:3587): label windows and the header by icon gate.
        let label_shown: [bool; 8] =
            std::array::from_fn(|i| should_draw_icon(host, StartMenuIcon::ALL[i]));
        let header_shown = label_shown.iter().any(|&shown| shown);

        let mut menu = Self {
            base: base.clone(),
            frame: base.clone(),
            fade: BrightnessFade::default(),
            flags: TextFlags::default(),
            font0,
            font4,
            font0_asset,
            font4_asset,
            focus_asset,
            top_char,
            top_screen,
            top_palette,
            cursor_char,
            cursor_palette,
            cursor_cells,
            cursor_anim,
            cursor_elapsed: 0,
            top_cleared: false,
            sub_char,
            sub_screen,
            sub_palette,
            icon_cells,
            icon_anim,
            icon_palette,
            sheet_char,
            sheet_palette,
            sheet_cells,
            sheet_anim,
            glyph_elapsed: 0,
            slots,
            label_shown,
            header_shown,
            header,
            layout,
            selected_slot: -1,
            inhibit,
            unk_350,
            insertion_order,
            selection_to_action,
            num_active,
            unk_d3: host.last_menu_selection(),
            last_touch_menu_input: 0,
            last_action: None,
            input_touch_mode: false,
            pressed_pending: false,
            state: State::HandleInput,
            prev_keys: Keys::IDLE,
            prev_touch: false,
        };
        // ov27_02259F80:122-131 — the cursor from the field's unkD3,
        // then ov27_0225C1EC's fallback to the first visible slot.
        menu.selected_slot = menu.slot_for_index(menu.unk_d3);
        menu.c1ec();
        menu.draw_open();
        Ok(menu)
    }

    /// The logical frame the last tick produced (the open menu over
    /// the base frame; the base itself once closed).
    #[must_use]
    pub fn frame(&self) -> &LogicalFrame {
        &self.frame
    }

    /// Whether the task is still running (no terminal event yet).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.state != State::Done
    }

    /// `StartMenuTaskData.inhibitIconFlags` — the [`disable`] bits.
    #[must_use]
    pub fn inhibit_flags(&self) -> u32 {
        self.inhibit
    }

    /// `selectionToAction[..numActiveButtons]` — the display order the
    /// compact selection index addresses.
    #[must_use]
    pub fn actions(&self) -> Vec<StartMenuAction> {
        self.selection_to_action[..self.num_active as usize]
            .iter()
            .map(|&a| StartMenuAction::from_index(a).expect("the lists hold actions"))
            .collect()
    }

    /// `insertionOrder[..numActiveButtons]`.
    #[must_use]
    pub fn insertion_order(&self) -> Vec<StartMenuAction> {
        self.insertion_order[..self.num_active as usize]
            .iter()
            .map(|&a| StartMenuAction::from_index(a).expect("the lists hold actions"))
            .collect()
    }

    /// The sub-screen grid: per slot 0–6, the resource index shown
    /// (`0xD` empty) and whether it is drawn and navigable.
    #[must_use]
    pub fn grid(&self) -> [(u8, bool); 7] {
        std::array::from_fn(|i| (self.slots[i].resource, self.slots[i].visible))
    }

    /// `StartMenuTaskData.unk_350` — the union-room variant, whose
    /// gear slot carries the LOG (`START_MENU_ACTION_12`).
    #[must_use]
    pub fn is_union_room(&self) -> bool {
        self.unk_350
    }

    /// The layout row overlay 27 picked (`ov27_0225BD50`): 0 normal,
    /// 1 safari, 2 bug contest, 3 pal park, 4 union room, 5 colosseum,
    /// 6 the battle tower partner room.
    #[must_use]
    pub fn layout(&self) -> usize {
        self.layout
    }

    /// The grid slot under the cursor, `None` when no icon is visible.
    #[must_use]
    pub fn selected_slot(&self) -> Option<usize> {
        usize::try_from(self.selected_slot).ok()
    }

    /// `fieldSystem->unkD3` — the compact selection index the C task
    /// picks by; carry it back into the field for the next open.
    #[must_use]
    pub fn selection_index(&self) -> u8 {
        self.unk_d3
    }

    /// `fieldSystem->lastStartMenuAction`, once a pick ran.
    #[must_use]
    pub fn last_action(&self) -> Option<StartMenuAction> {
        self.last_action
    }

    /// `fieldSystem->lastTouchMenuInput` as the tick left it.
    #[must_use]
    pub fn last_touch_menu_input(&self) -> u16 {
        self.last_touch_menu_input
    }

    /// `MenuInputStateMgr_GetState` — TOUCH after a stylus pick.
    #[must_use]
    pub fn input_touch_mode(&self) -> bool {
        self.input_touch_mode
    }

    /// One frame: the C task's pass (`Task_StartMenu`, `:333`), then
    /// overlay 27's sys task (`ov27_0225A320`, `:420` — its case-2
    /// touch and d-pad handling), then the sprite animation step and
    /// the fade's post-vblank update. Returns what the tick produced.
    pub fn tick(
        &mut self,
        _frame: crate::Frame,
        input: Input,
        host: &dyn StartMenuHost,
    ) -> StartMenuEvent {
        let new_keys = input.keys.pressed(self.prev_keys);
        self.prev_keys = input.keys;
        let touch_new = input.touch.is_some() && !self.prev_touch;
        self.prev_touch = input.touch.is_some();

        let mut event = StartMenuEvent::None;
        match self.state {
            State::HandleInput => {
                // Task_StartMenu_HandleInput (:564-589): A first, then
                // the B/X edge when no touch input is queued, else the
                // queued touch input.
                if new_keys.any(key::A) {
                    event = self.handle_key_input(host);
                } else if self.last_touch_menu_input == 0 {
                    if new_keys.any(key::B | key::X) {
                        // PlaySE(SEQ_SE_GS_GEARCANCEL) — audio deferred.
                        self.state = State::Close;
                    }
                } else {
                    event = self.handle_touch_input(host);
                }
            }
            State::WaitFade(action) => {
                // Task_StartMenu_WaitFade (:707-717): the bar cleared,
                // unkD2_0 = 0, the exit task launches the app.
                if self.fade.is_finished() {
                    self.clear_top();
                    self.state = State::Done;
                    event = StartMenuEvent::Selected(action);
                }
            }
            State::State19 => self.state = State::State20,
            State::State20 => self.state = State::Close,
            State::Close => {
                // :429-436 — sub_0203C38C, FillBgTilemapRect(MAIN_3),
                // Heap_Free: the base frame returns.
                self.frame = self.base.clone();
                self.state = State::Done;
                event = StartMenuEvent::Closed;
            }
            State::Done => {}
        }

        if self.state == State::Done {
            return event;
        }

        // ov27_0225A320: the sub-screen task's case 2 runs while the
        // field task is up, the fade is finished, and no pick is
        // pending (the `0x51C` bit-5 gate in ov27_0225B4D8).
        if matches!(self.state, State::HandleInput | State::Close) && !self.pressed_pending {
            self.sub_screen_input(input, new_keys, touch_new);
        }
        // ov27_0225A320:366-389 — bit 5: Sprite_TryChangeAnimSeq(icon, 2).
        if self.pressed_pending {
            self.pressed_pending = false;
            if let Ok(slot) = usize::try_from(self.selected_slot) {
                self.set_icon_sequence(slot, ICON_SEQ_PRESSED, false);
            }
        }

        // SpriteList_RenderAndAnimateSprites on both lists.
        self.cursor_elapsed = self.cursor_elapsed.saturating_add(1);
        self.glyph_elapsed = self.glyph_elapsed.saturating_add(1);
        for slot in &mut self.slots {
            slot.elapsed = slot.elapsed.saturating_add(1);
        }
        self.draw_sprites();

        // The post-vblank fade step; both engines carry the brightness.
        self.fade.update();
        let brightness = self.fade.brightness();
        self.frame.main.brightness = brightness;
        self.frame.sub.brightness = brightness;
        event
    }

    /// `StartMenu_HandleKeyInput` (`:591-611`) — the A press.
    fn handle_key_input(&mut self, host: &dyn StartMenuHost) -> StartMenuEvent {
        if u32::from(self.unk_d3) >= self.num_active {
            return StartMenuEvent::None;
        }
        // PlaySE(SEQ_SE_DP_SELECT) — audio deferred.
        self.input_touch_mode = false;
        self.select(host, self.unk_d3, false)
    }

    /// `StartMenu_HandleTouchInput` (`:613-663`) — the queued
    /// `lastTouchMenuInput`: 1 closes, 2–10 pick display slot
    /// `input - 2`.
    fn handle_touch_input(&mut self, host: &dyn StartMenuHost) -> StartMenuEvent {
        self.input_touch_mode = true;
        match self.last_touch_menu_input {
            1 => {
                // PlaySE(SEQ_SE_GS_GEARCANCEL) — audio deferred.
                self.state = State::Close;
                self.last_touch_menu_input = 0;
                StartMenuEvent::None
            }
            2..=10 => {
                self.unk_d3 = (self.last_touch_menu_input - 2) as u8;
                if u32::from(self.unk_d3) >= self.num_active {
                    return StartMenuEvent::None;
                }
                // PlaySE(SEQ_SE_DP_SELECT) — audio deferred.
                self.select(host, self.unk_d3, true)
            }
            _ => StartMenuEvent::None,
        }
    }

    /// The shared pick: `sStartMenuActions[selectionToAction[index]]`
    /// — CANCEL closes, an available action runs its selection
    /// handler (`:597-608` / `:641-654`).
    fn select(&mut self, host: &dyn StartMenuHost, index: u8, by_touch: bool) -> StartMenuEvent {
        let action = StartMenuAction::from_index(self.selection_to_action[usize::from(index)])
            .expect("the lists hold actions");
        if action == StartMenuAction::RunningShoes {
            // STARTMENUTASKFUNC_CANCEL.
            self.state = State::Close;
            self.last_touch_menu_input = 0;
            return StartMenuEvent::None;
        }
        // No entry carries STARTMENUTASKFUNC_NONE.
        if !action_available(host, action) {
            if by_touch {
                self.last_touch_menu_input = 0;
            }
            return StartMenuEvent::None;
        }
        // sub_0203DF64(fieldSystem, by_touch) — the bag's input-mode
        // memory, deferred with the bag. ov01_021F6B50: the sub-screen
        // plays the pressed animation.
        self.pressed_pending = true;
        self.last_touch_menu_input = 0;
        self.last_action = Some(action);
        self.run_selection(action)
    }

    /// The selection handlers (`:736-1300`): the app launches fade
    /// out first; SAVE switches the sub-screen app; RETIRE clears the
    /// bar and starts a script; the union-room LOG walks states
    /// 19 → 20 → CLOSE.
    fn run_selection(&mut self, action: StartMenuAction) -> StartMenuEvent {
        match action {
            StartMenuAction::Pokedex
            | StartMenuAction::Pokemon
            | StartMenuAction::Bag
            | StartMenuAction::TrainerCard
            | StartMenuAction::Options
            | StartMenuAction::Action7
            | StartMenuAction::Action9
            | StartMenuAction::Action10
            | StartMenuAction::Pokegear => {
                // FieldMap_FadeScreen(FADE_TYPE_BRIGHTNESS_OUT).
                self.fade.begin(
                    FadeType::BrightnessOut,
                    FadeColor::Black,
                    FADE_STEPS,
                    FADE_FRAMES_PER_STEP,
                );
                self.state = State::WaitFade(action);
                StartMenuEvent::None
            }
            StartMenuAction::Save => {
                // ov01_021F6A9C(fieldSystem, 1, NULL) + STATE_SAVE — the
                // touch save app is a later slice; the caller owns it.
                self.state = State::Done;
                StartMenuEvent::Selected(action)
            }
            StartMenuAction::Retire => {
                // sub_0203C38C, unkD2_0 = 0, StartScriptFromMenu (:1252).
                self.clear_top();
                self.state = State::Done;
                StartMenuEvent::Selected(action)
            }
            StartMenuAction::Action12 => {
                // sub_0203D2CC (:1158): ov01_021F6A9C(fieldSystem, 8, NULL)
                // and the two-state walk to CLOSE.
                self.state = State::State19;
                StartMenuEvent::Selected(action)
            }
            StartMenuAction::RunningShoes => unreachable!("CANCEL is handled by the caller"),
        }
    }

    /// `ov27_0225A7FC` case 2 (`:1076-1090`): `ov27_0225B4D8`'s touch
    /// pass, then — when it did not consume the frame —
    /// `ov27_0225B404`'s d-pad walk.
    fn sub_screen_input(&mut self, input: Input, new_keys: Keys, touch_new: bool) {
        // `_0225A85C`: `bl ov27_0225B4D8; cmp r0, #0; beq _0225A86A;
        // bl ov27_0225B404` — the walk runs on a nonzero return.
        if self.b4d8(input, new_keys, touch_new) {
            self.b404(new_keys);
        }
    }

    /// `ov27_0225B4D8` (`:2646-2760`) — returns TRUE (the d-pad walk
    /// runs) unless a stylus press on the grid was handled here.
    fn b4d8(&mut self, input: Input, new_keys: Keys, touch_new: bool) -> bool {
        // A moving player or a new d-pad key: straight to the walk.
        if new_keys.any(key::UP | key::DOWN | key::LEFT | key::RIGHT) {
            return true;
        }
        // (The colosseum row's link checks, the fade and bit-5 gates:
        // the fade is finished and no pick pending whenever this runs.)
        let hit = touch_new
            .then_some(input.touch)
            .flatten()
            .and_then(hitbox_at);
        let Some(hit) = hit else {
            return true;
        };
        if (1..8).contains(&hit) && !self.slots[hit - 1].visible {
            // An unlit slot swallows the press.
            return false;
        }
        if hit >= 8 {
            // The registered-item buttons: the field UI is hidden
            // (bit 0 clear) while the menu is open.
            return true;
        }
        if hit == 0 {
            // The header strip: *lastTouchMenuInput = 1.
            self.last_touch_menu_input = 1;
            return false;
        }
        let slot = hit - 1;
        self.selected_slot = slot as i32;
        self.unk_d3 = self.c170(slot);
        // ov27_0225B398 — the highlight palette follows selected_slot.
        self.last_touch_menu_input = u16::from(self.c170(slot)) + 2;
        false
    }

    /// `ov27_0225B404` (`:2529-2617`) — the d-pad walk: the first new
    /// direction, `ov27_0225B360`'s candidate pick, the bounce
    /// animation, the highlight, and `unkD3` from `ov27_0225C170`.
    fn b404(&mut self, new_keys: Keys) {
        let Ok(current) = usize::try_from(self.selected_slot) else {
            // No visible slot: the asm walks a negative table offset;
            // the port treats it as no move.
            return;
        };
        let direction = DIRECTIONS
            .iter()
            .find(|&&(bit, _)| new_keys.any(bit))
            .map(|&(_, dir)| dir);
        let Some(direction) = direction else {
            return;
        };
        let Some(next) = self.b360(current, direction) else {
            return;
        };
        if next == current {
            return;
        }
        self.selected_slot = next as i32;
        // PlaySE(0x5E0) — audio deferred.
        let sequence = if direction <= 1 {
            ICON_SEQ_UP_DOWN
        } else {
            ICON_SEQ_LEFT_RIGHT
        };
        self.set_icon_sequence(next, sequence, true);
        self.unk_d3 = self.c170(next);
    }

    /// `ov27_0225B360` (`:2440-2472`) — the first visible candidate of
    /// `NAV[current][direction]`.
    fn b360(&self, current: usize, direction: usize) -> Option<usize> {
        NAV[current][direction]
            .iter()
            .map(|&slot| usize::from(slot))
            .find(|&slot| self.slots[slot].visible)
    }

    /// `ov27_0225C170` (`:4247-4283`) — the compact index of `slot`:
    /// the visible slots up to and including it, minus one (7 and 8,
    /// the registered items, pass through).
    fn c170(&self, slot: usize) -> u8 {
        if slot == 7 || slot == 8 {
            return slot as u8;
        }
        let visible = self.slots[..=slot]
            .iter()
            .filter(|entry| entry.visible)
            .count();
        debug_assert!(visible > 0, "GF_ASSERT: the slot is visible");
        visible.saturating_sub(1) as u8
    }

    /// `ov27_0225C1AC` (`:4285-4324`) — the `index`-th visible slot,
    /// or `-1` (writing `unkD3 = 0`) when there is none.
    fn slot_for_index(&mut self, index: u8) -> i32 {
        let mut seen = 0u8;
        for (slot, entry) in self.slots.iter().enumerate() {
            if !entry.visible {
                continue;
            }
            if seen == index {
                return slot as i32;
            }
            seen += 1;
        }
        self.unk_d3 = 0;
        -1
    }

    /// `ov27_0225C1EC` (`:4326-4368`) — when the cursor's slot is not
    /// visible: `unkD3` takes the first visible slot's *index* and the
    /// cursor re-resolves through `ov27_0225C1AC` (the asm's quirk,
    /// kept: the cursor lands on the k-th visible slot for first
    /// visible slot k).
    fn c1ec(&mut self) {
        let visible = usize::try_from(self.selected_slot)
            .ok()
            .is_some_and(|slot| self.slots[slot].visible);
        if visible {
            return;
        }
        if let Some(first) = self.slots.iter().position(|entry| entry.visible) {
            self.unk_d3 = first as u8;
            self.selected_slot = self.slot_for_index(self.unk_d3);
        }
    }

    /// `Sprite_SetAnimCtrlSeq` (`force`) / `Sprite_TryChangeAnimSeq`
    /// (only when the sequence differs) on a grid icon.
    fn set_icon_sequence(&mut self, slot: usize, sequence: usize, force: bool) {
        let entry = &mut self.slots[slot];
        if force || entry.sequence != sequence {
            entry.sequence = sequence;
            entry.elapsed = 0;
        }
    }

    /// `sub_0203C38C` (`:525-529`) — `StartMenu_DestroyCursor` and the
    /// MAIN BG3 tilemap fill with 0: the bar's map goes, the cursor
    /// sprite goes, the char and palette loads stay.
    fn clear_top(&mut self) {
        self.top_cleared = true;
        self.frame.main.bgs[3].screen = None;
        self.draw_sprites();
    }

    /// The open pass: `Task_StartMenu_DrawCursor` (`:455-472`) on the
    /// main engine, overlay 27's init (`ov27_02259F80`, `:7`) on the
    /// sub engine — its BG, palette, windows, blend register — and
    /// the sprites of both.
    fn draw_open(&mut self) {
        // GfGfxLoader_LoadCharData(NARC_a_0_1_4, 12, …, MAIN_3, 0, 0, TRUE)
        // + GXLoadPal(…, 15, MAIN_BG, 0x1C0, 0x20) + LoadScrnData(…, 13, MAIN_3).
        let main = &mut self.frame.main;
        main.bgs[3] = BgLayer {
            enabled: true,
            char_base: MAIN3_BLOCK,
            screen: Some(self.top_screen),
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH256,
            scroll_x: 0,
            scroll_y: 0,
            priority: MAIN3_PRIORITY,
            hidden_rect: None,
        };
        main.char_blocks[usize::from(MAIN3_BLOCK)].push(TilePlacement {
            asset: self.top_char,
            tile: 0,
        });
        main.palette_loads.push(PaletteLoad {
            asset: self.top_palette,
            offset: u16::from(narc::TOP_BAR_BANK) * 16,
            colors: 16,
        });
        main.tilemap_edits.retain(|edit| match edit {
            crate::frame::TilemapEdit::Palette { bg, .. }
            | crate::frame::TilemapEdit::Fill { bg, .. } => *bg != 3,
        });

        // ov27_02259F80: InitBgFromTemplate(4, ov27_0225D000) /
        // (5, ov27_0225D01C); ov27_0225AC00: the char, screen, and
        // 0x200-byte palette; EngineBTogglePlanes: BG0/BG1/OBJ on,
        // BG2/BG3 off.
        let mut sub = EngineFrame::default();
        sub.bgs[0] = BgLayer {
            enabled: true,
            char_base: 0,
            screen: Some(self.sub_screen),
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH256,
            scroll_x: 0,
            scroll_y: 0,
            priority: SUB0_PRIORITY,
            hidden_rect: None,
        };
        sub.bgs[1] = BgLayer {
            enabled: true,
            char_base: 0,
            screen: None,
            color_mode: ColorMode::Bpp4,
            size: ScreenSize::W256xH256,
            scroll_x: 0,
            scroll_y: 0,
            priority: SUB1_PRIORITY,
            hidden_rect: None,
        };
        sub.char_blocks[0].push(TilePlacement {
            asset: self.sub_char,
            tile: 0,
        });
        sub.palette_loads.push(PaletteLoad {
            asset: self.sub_palette,
            offset: 0,
            colors: 256,
        });
        // ov27_0225A8E8(1) — the open menu's blend register.
        sub.blend = SUB_BLEND;
        sub.backdrop = self.base.sub.backdrop;
        self.frame.sub = sub;

        // ov27_0225BCE8 (:3641): every slot's label, centered in its
        // 9×2 window; ov27_0225BC84 (:3587): only the icon-gated ones
        // reach VRAM, plus the header when any does.
        let input = Input::default();
        for slot in 0..7 {
            if !self.label_shown[slot] {
                continue;
            }
            let Some(label) = self.slots[slot].label.clone() else {
                continue;
            };
            let (left, top) = LABEL_WINDOWS[slot];
            let mut window = add_window(
                1,
                left,
                top,
                LABEL_WIDTH,
                LABEL_HEIGHT,
                LABEL_PALETTE,
                LABEL_BASE_TILE + LABEL_TILE_STRIDE * slot as u16,
            );
            let width = self.font0.string_width(label.units(), 0) as i32;
            let x = (LABEL_CENTER_WIDTH - width) / 2;
            let mut printer = TextPrinter::new(
                0,
                self.font0_asset,
                self.focus_asset,
                label,
                x.max(0) as u16,
                0,
                LABEL_COLOR,
                TEXT_SPEED_NOTRANSFER,
                0,
            );
            printer.render_instant(&self.font0, &mut window, input, &mut self.flags);
            self.frame.sub.windows.push(window);
        }
        if self.header_shown {
            let (left, top, width, height, base_tile) = HEADER_WINDOW;
            let mut window = add_window(1, left, top, width, height, LABEL_PALETTE, base_tile);
            let mut printer = TextPrinter::new(
                4,
                self.font4_asset,
                self.focus_asset,
                self.header.clone(),
                0,
                0,
                HEADER_COLOR,
                TEXT_SPEED_INSTANT,
                0,
            );
            printer.render_instant(&self.font4, &mut window, input, &mut self.flags);
            self.frame.sub.windows.push(window);
        }
        self.draw_sprites();
    }

    /// The sprite lists as this tick leaves them: the base frame's
    /// main sprites plus the cursor (until `sub_0203C38C`), the grid's
    /// visible icons (`ov27_0225AA7C`), the selected one on its
    /// highlight bank (`ov27_0225B398`), and the header glyph.
    fn draw_sprites(&mut self) {
        let mut main = self.base.main.sprites.clone();
        if !self.top_cleared {
            main.push(Sprite {
                tiles: self.cursor_char,
                palette: self.cursor_palette,
                cells: self.cursor_cells,
                animation: self.cursor_anim,
                sequence: 0,
                elapsed: self.cursor_elapsed,
                x: CURSOR_POSITION.0,
                y: CURSOR_POSITION.1,
                priority: 0,
                palette_bank: 0,
            });
        }
        self.frame.main.sprites = main;

        let mut sub = Vec::with_capacity(8);
        for (slot, entry) in self.slots.iter().enumerate() {
            let (Some(tiles), true) = (entry.tiles, entry.visible) else {
                continue;
            };
            let (x, y) = ICON_POSITIONS[slot];
            sub.push(Sprite {
                tiles,
                palette: self.icon_palette,
                cells: self.icon_cells,
                animation: self.icon_anim,
                sequence: entry.sequence,
                elapsed: entry.elapsed,
                x,
                y,
                priority: 0,
                palette_bank: u8::from(self.selected_slot == slot as i32),
            });
        }
        if self.header_shown {
            let (x, y, sequence) = HEADER_GLYPH;
            sub.push(Sprite {
                tiles: self.sheet_char,
                palette: self.sheet_palette,
                cells: self.sheet_cells,
                animation: self.sheet_anim,
                sequence,
                elapsed: self.glyph_elapsed,
                x,
                y,
                priority: 0,
                palette_bank: 0,
            });
        }
        self.frame.sub.sprites = sub;
    }
}

/// `FieldSystem_GetStartMenuButtonInhibitFlags_Normal` (`:288-307`).
#[must_use]
pub fn inhibit_normal(host: &dyn StartMenuHost) -> u32 {
    let mut ret = 0;
    if !host.has_pokedex() {
        ret |= disable::POKEDEX;
    }
    if !host.has_starter() {
        ret |= disable::POKEMON;
    }
    if !host.menu_icon_unlocked(0) {
        ret |= disable::BAG;
    }
    if !host.has_pokegear() {
        ret |= disable::POKEGEAR;
    }
    if host.map_is_amity_square() {
        ret |= disable::POKEMON | disable::BAG;
    }
    ret | disable::ACTION_7 | disable::RETIRE
}

/// `FieldSystem_GetStartMenuButtonInhibitFlags_Safari` (`:309`).
#[must_use]
pub const fn inhibit_safari() -> u32 {
    disable::SAVE | disable::ACTION_7
}

/// `FieldSystem_GetStartMenuButtonInhibitFlags_BugContest` (`:313`).
#[must_use]
pub const fn inhibit_bug_contest() -> u32 {
    disable::BAG | disable::SAVE | disable::ACTION_7
}

/// `FieldSystem_GetStartMenuButtonInhibitFlags_PalPark` (`:317`).
#[must_use]
pub const fn inhibit_pal_park() -> u32 {
    disable::BAG | disable::SAVE | disable::ACTION_7
}

/// `FieldSystem_GetStartMenuButtonInhibitFlags_BattleTowerMultiPartnerSelectRoom`
/// (`:321`).
#[must_use]
pub const fn inhibit_battle_tower_multi_partner_select_room() -> u32 {
    disable::POKEDEX
        | disable::BAG
        | disable::SAVE
        | disable::ACTION_7
        | disable::RETIRE
        | disable::POKEGEAR
}

/// `sub_0203BEE0` (`:325`) — the union room.
#[must_use]
pub const fn inhibit_union() -> u32 {
    disable::SAVE | disable::RETIRE
}

/// `sub_0203BEE8` (`:329`) — the colosseum.
#[must_use]
pub const fn inhibit_colosseum() -> u32 {
    disable::POKEDEX | disable::SAVE | disable::ACTION_7 | disable::RETIRE | disable::POKEGEAR
}

/// `StartMenu_BuildActionLists` (`:483-523`): the gated inserts in
/// table order — RETIRE, 7, POKéDEX, POKéMON, BAG, the gear slot
/// (LOG when `unk_350`), TRAINER CARD, SAVE, OPTIONS, RUNNING SHOES —
/// then `START_MENU_ACTION_9`/`10` forced into display slots 7 and 8
/// (`StartMenuButton_Insert`'s explicit `position`, `:474-481`).
/// Returns `(insertionOrder, selectionToAction, numActiveButtons)`.
#[must_use]
pub fn build_action_lists(inhibit: u32, unk_350: bool) -> ([u8; 10], [u8; 10], u32) {
    let mut insertion = [0u8; 10];
    let mut display = [0u8; 10];
    let mut len = 0u32;
    let mut insert = |item: StartMenuAction, position: Option<usize>| {
        insertion[len as usize] = item.index();
        display[position.unwrap_or(len as usize)] = item.index();
        len += 1;
    };
    if inhibit & disable::RETIRE == 0 {
        insert(StartMenuAction::Retire, None);
    }
    if inhibit & disable::ACTION_7 == 0 {
        insert(StartMenuAction::Action7, None);
    }
    if inhibit & disable::POKEDEX == 0 {
        insert(StartMenuAction::Pokedex, None);
    }
    if inhibit & disable::POKEMON == 0 {
        insert(StartMenuAction::Pokemon, None);
    }
    if inhibit & disable::BAG == 0 {
        insert(StartMenuAction::Bag, None);
    }
    if inhibit & disable::POKEGEAR == 0 {
        if unk_350 {
            insert(StartMenuAction::Action12, None);
        } else {
            insert(StartMenuAction::Pokegear, None);
        }
    }
    if inhibit & disable::TRAINER_CARD == 0 {
        insert(StartMenuAction::TrainerCard, None);
    }
    if inhibit & disable::SAVE == 0 {
        insert(StartMenuAction::Save, None);
    }
    if inhibit & disable::OPTIONS == 0 {
        insert(StartMenuAction::Options, None);
    }
    if inhibit & disable::RUNNING_SHOES == 0 {
        insert(StartMenuAction::RunningShoes, None);
    }
    insert(StartMenuAction::Action9, Some(7));
    insert(StartMenuAction::Action10, Some(8));
    (insertion, display, len)
}

/// `ov27_0225BD50` (`:3699-3748`) — the sub-screen layout row: the
/// battle tower partner room 6, safari 1, bug contest 2, pal park 3,
/// `bottomScreenType == 3` 4, the colosseum 5, else 0.
#[must_use]
pub fn layout_row(host: &dyn StartMenuHost) -> usize {
    if host.map_is_battle_tower_multi_partner_select_room() {
        6
    } else if host.safari_active() {
        1
    } else if host.bug_contest_active() {
        2
    } else if host.pal_park_active() {
        3
    } else if host.bottom_screen_type() == 3 {
        4
    } else if host.map_load_type() == MapLoadType::Colosseum {
        5
    } else {
        0
    }
}

/// `TouchscreenHitbox_FindRectAtTouchNew(ov27_0225CF68)` — the first
/// hitbox containing the stylus, if any.
fn hitbox_at(touch: Touch) -> Option<usize> {
    HITBOXES.iter().position(|&(top, bottom, left, right)| {
        touch.x >= u16::from(left)
            && touch.x < u16::from(right)
            && touch.y >= u16::from(top)
            && touch.y < u16::from(bottom)
    })
}

/// `AddWindowParameterized` — one window from its template
/// parameters, filled with 0 (`FillWindowPixelBuffer(window, 0)`).
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
        fills: Vec::new(),
        scroll: 0,
        frame: None,
        arrow: None,
        focus: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flag-set host with the profile a test names.
    struct Host {
        flags: Vec<u16>,
        running_shoes: bool,
        battle_tower: bool,
        load_type: MapLoadType,
    }

    impl StartMenuHost for Host {
        fn flag(&self, id: u16) -> bool {
            self.flags.contains(&id)
        }
        fn var(&self, _id: u16) -> u16 {
            0
        }
        fn party_count(&self) -> u8 {
            0
        }
        fn player_name(&self) -> GameString {
            GameString::new()
        }
        fn player_gender(&self) -> u8 {
            0
        }
        fn has_running_shoes(&self) -> bool {
            self.running_shoes
        }
        fn map_load_type(&self) -> MapLoadType {
            self.load_type
        }
        fn map_is_battle_tower_multi_partner_select_room(&self) -> bool {
            self.battle_tower
        }
    }

    fn host(flags: &[u16]) -> Host {
        Host {
            flags: flags.to_vec(),
            running_shoes: false,
            battle_tower: false,
            load_type: MapLoadType::Overworld,
        }
    }

    #[test]
    fn the_action_table_restates_s_start_menu_actions() {
        // :48-62 and :176-190: thirteen rows, the gear quartet on
        // msg_0196_00014, EXIT on 6.
        assert_eq!(StartMenuAction::ALL.len(), 13);
        for (i, action) in StartMenuAction::ALL.iter().enumerate() {
            assert_eq!(usize::from(action.index()), i);
            assert_eq!(StartMenuAction::from_index(i as u8), Some(*action));
        }
        assert_eq!(StartMenuAction::RunningShoes.label_msg(), 6);
        assert_eq!(StartMenuAction::Pokegear.label_msg(), 14);
        assert_eq!(StartMenuAction::Action12.label_msg(), 14);
        // sActionToIconIndex: 0, 1, 2, 4, 5, 6, 100×5, 3.
        assert_eq!(
            StartMenuAction::TrainerCard.icon(),
            Some(StartMenuIcon::TrainerCard)
        );
        assert_eq!(
            StartMenuAction::Pokegear.icon(),
            Some(StartMenuIcon::Pokegear)
        );
        assert_eq!(StartMenuAction::RunningShoes.icon(), None);
        assert_eq!(StartMenuAction::Action9.icon(), None);
    }

    #[test]
    fn the_normal_mask_gates_on_the_four_flags() {
        let fresh = host(&[]);
        assert_eq!(
            inhibit_normal(&fresh),
            disable::POKEDEX
                | disable::POKEMON
                | disable::BAG
                | disable::POKEGEAR
                | disable::ACTION_7
                | disable::RETIRE
        );
        let full = host(&[
            flag::GOT_POKEDEX,
            flag::GOT_STARTER,
            flag::GOT_BAG,
            flag::GOT_POKEGEAR,
        ]);
        assert_eq!(inhibit_normal(&full), disable::ACTION_7 | disable::RETIRE);
    }

    #[test]
    fn the_action_lists_follow_build_action_lists() {
        // The full normal menu: seven icons and EXIT in insertion
        // order; 9 and 10 land on display slots 7 and 8, so EXIT's
        // display slot is overwritten and the count is ten.
        let (insertion, display, count) =
            build_action_lists(disable::ACTION_7 | disable::RETIRE, false);
        assert_eq!(count, 10);
        let idx = |a: StartMenuAction| a.index();
        assert_eq!(
            insertion,
            [
                idx(StartMenuAction::Pokedex),
                idx(StartMenuAction::Pokemon),
                idx(StartMenuAction::Bag),
                idx(StartMenuAction::Pokegear),
                idx(StartMenuAction::TrainerCard),
                idx(StartMenuAction::Save),
                idx(StartMenuAction::Options),
                idx(StartMenuAction::RunningShoes),
                idx(StartMenuAction::Action9),
                idx(StartMenuAction::Action10),
            ]
        );
        assert_eq!(
            display,
            [
                idx(StartMenuAction::Pokedex),
                idx(StartMenuAction::Pokemon),
                idx(StartMenuAction::Bag),
                idx(StartMenuAction::Pokegear),
                idx(StartMenuAction::TrainerCard),
                idx(StartMenuAction::Save),
                idx(StartMenuAction::Options),
                idx(StartMenuAction::Action9),
                idx(StartMenuAction::Action10),
                0,
            ]
        );
        // The fresh bedroom: no dex, starter, bag, or gear.
        let (_, display, count) = build_action_lists(inhibit_normal(&host(&[])), false);
        assert_eq!(count, 6);
        assert_eq!(
            &display[..4],
            &[
                idx(StartMenuAction::TrainerCard),
                idx(StartMenuAction::Save),
                idx(StartMenuAction::Options),
                idx(StartMenuAction::RunningShoes),
            ]
        );
        assert_eq!(display[7], idx(StartMenuAction::Action9));
        assert_eq!(display[8], idx(StartMenuAction::Action10));
        // Safari: RETIRE leads, SAVE is gone.
        let (insertion, _, count) = build_action_lists(inhibit_safari(), false);
        assert_eq!(count, 10);
        assert_eq!(insertion[0], idx(StartMenuAction::Retire));
        assert!(!insertion[..count as usize].contains(&idx(StartMenuAction::Save)));
        // The union room swaps the gear for the LOG.
        let (insertion, _, _) = build_action_lists(inhibit_union(), true);
        assert_eq!(insertion[4], idx(StartMenuAction::Action12));
        assert_eq!(insertion[0], idx(StartMenuAction::Action7));
    }

    #[test]
    fn the_layout_row_follows_ov27_0225bd50() {
        assert_eq!(layout_row(&host(&[])), 0);
        assert_eq!(layout_row(&host(&[flag::SYS_SAFARI])), 1);
        assert_eq!(layout_row(&host(&[flag::BUG_CONTEST])), 2);
        assert_eq!(layout_row(&host(&[flag::SYS_PAL_PARK])), 3);
        let mut colosseum = host(&[]);
        colosseum.load_type = MapLoadType::Colosseum;
        assert_eq!(layout_row(&colosseum), 5);
        let mut tower = host(&[flag::SYS_SAFARI]);
        tower.battle_tower = true;
        assert_eq!(layout_row(&tower), 6, "the tower room wins over safari");
        // The layout rows and the C's insertion order agree slot for
        // slot in every mode (the grid slot is the display slot).
        assert_eq!(LAYOUT_SLOTS[1][0], 7, "safari: RETIRE leads");
        assert_eq!(
            LAYOUT_SLOTS[6][..3],
            [1, 4, 6],
            "tower: POKéMON, card, options"
        );
    }

    #[test]
    fn hitboxes_and_navigation_tables_are_the_roms() {
        // The header strip closes; icon 0's box sits under its sprite.
        assert_eq!(hitbox_at(Touch { x: 100, y: 5 }), Some(0));
        assert_eq!(hitbox_at(Touch { x: 30, y: 30 }), Some(1));
        assert_eq!(hitbox_at(Touch { x: 120, y: 110 }), Some(7));
        assert_eq!(hitbox_at(Touch { x: 230, y: 20 }), Some(8));
        assert_eq!(hitbox_at(Touch { x: 200, y: 180 }), None);
        // UP from the top-left wraps to the bottom of its column.
        assert_eq!(NAV[0][0][0], 3);
        assert_eq!(NAV[3][1][0], 0);
        // LEFT/RIGHT cross columns.
        assert_eq!(NAV[1][3][0], 5);
        assert_eq!(NAV[5][2][0], 1);
    }
}
