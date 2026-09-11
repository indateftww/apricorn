//! `ScriptHost` — what the VM asks of the field it runs in.
//!
//! pret's command handlers reach into `FieldSystem` directly: the save
//! (`Save_VarsFlags_Get`, the bag, the party, the profile), the map
//! object manager, the dialogue window, the sound driver, the palette
//! fader, the task manager (warps, applications), `gSystem.newKeys`.
//! Every such reach is a method here, so the VM has no field
//! dependency and a test can answer it from a table. The trait is
//! deliberately narrow: three typed enums ([`FieldAction`],
//! [`FieldQuery`], [`WaitFor`]) carry the long tail of one-off
//! commands instead of a method each.
//!
//! [`RecordingHost`] is the mock — it logs every call as a
//! [`HostEvent`], answers queries and waits from tables, and holds a
//! real [`VarsFlags`] block, so both the unit tests and the ROM-gated
//! runs drive the same code.

use std::collections::{HashMap, VecDeque};
use std::fmt;

use crate::input::Keys;
use crate::rng::Lcrng;
use crate::save::vars_flags::{SIZE as VARS_FLAGS_SIZE, TempFlags, VarsFlags};
use crate::text::string::GameString;

use super::bank::MapBanks;
use super::commands::MovementCommand;

/// `obj_player` (`include/constants/scrcmd.h:41`) — the player as a
/// movement target.
pub const OBJ_PLAYER: u16 = 255;
/// `obj_partner_poke` (`scrcmd.h:40`) — the following Pokémon.
pub const OBJ_PARTNER_POKE: u16 = 253;

/// `DIR_*` (`include/constants/global_fieldmap.h:5-8`).
pub mod dir {
    /// `DIR_NORTH`.
    pub const NORTH: u16 = 0;
    /// `DIR_SOUTH`.
    pub const SOUTH: u16 = 1;
    /// `DIR_WEST`.
    pub const WEST: u16 = 2;
    /// `DIR_EAST`.
    pub const EAST: u16 = 3;
}

/// One party member's script-visible attributes (`GetMonData`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyMon {
    /// `MON_DATA_SPECIES`.
    pub species: u16,
    /// `MON_DATA_FORM`.
    pub form: u8,
    /// `MON_DATA_IS_EGG`.
    pub is_egg: bool,
    /// `MON_DATA_FRIENDSHIP`.
    pub friendship: u8,
    /// `MON_DATA_NICKNAME_STRING`.
    pub nickname: GameString,
}

/// Where a print goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrintTarget {
    /// The dialogue box (`DialogBox_PrintMessageEx`).
    Dialog,
    /// The signpost window (`Signpost_GetWindow`).
    Signpost,
}

/// The parameters of a print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrintParams {
    /// The font id (1 for field dialogue).
    pub font: u8,
    /// Frames per character (`Options_GetTextFrameDelay`); ignored when
    /// `instant`.
    pub frame_delay: u32,
    /// `canABSpeedUp`.
    pub can_ab_speed_up: bool,
    /// pret's `unk1` / `a4` flag of `DialogBox_PrintMessageEx`.
    pub flag: u8,
    /// `TEXT_SPEED_INSTANT` — draw the whole string at once.
    pub instant: bool,
    /// `MAKE_TEXT_COLOR(fg, shadow, bg)` when not the default.
    pub color: Option<[u8; 3]>,
}

/// A side effect on the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldAction {
    /// `LockAll`: pause every object's movement. With a last-interacted
    /// object this is `LockLastTalked`. Return non-zero to have the VM
    /// wait for [`WaitFor::LockSettled`].
    LockAll {
        /// The object the player interacted with, if any.
        last_interacted: Option<u16>,
    },
    /// `ReleaseAll` — `MapObjectManager_UnpauseAllMovement`.
    ReleaseAll,
    /// `Lock`/`Release` one object (`MapObject_PauseMovement`).
    LockObject(u16),
    /// `Release` one object.
    ReleaseObject(u16),
    /// `ShowPerson` — create the map object from its event.
    ShowObject(u16),
    /// `HidePerson` — delete the map object.
    HideObject(u16),
    /// `FacePlayer` — turn `object` to face the player.
    FacePlayer {
        /// The last-interacted object.
        object: u16,
    },
    /// `PlayerAvatar_SetFacingDirection` (from the wait-button natives).
    SetPlayerFacing(u16),
    /// `MovePersonFacing` — `MapObject_SetPositionFromXYZAndDirection`.
    SetObjectPosition {
        /// The object.
        object: u16,
        /// Tile x.
        x: u16,
        /// Height.
        y: u16,
        /// Tile z.
        z: u16,
        /// Facing.
        direction: u16,
    },
    /// `FadeScreen` — `BeginNormalPaletteFade` on both screens.
    FadeScreen {
        /// Steps.
        duration: u16,
        /// Frames per step.
        speed: u16,
        /// `enum FadeType`.
        kind: u16,
        /// The fade colour (BGR555).
        color: u16,
    },
    /// `Warp` — `CallTask_ScriptWarp(map, -1, x, y, direction)`.
    Warp {
        /// The destination map.
        map: u16,
        /// Tile x.
        x: u16,
        /// Tile y (z).
        y: u16,
        /// Arrival facing.
        direction: u16,
    },
    /// `RestoreOverworld` — `CallTask_RestoreOverworld`.
    RestoreOverworld,
    /// `ScrCmd_436` — `CallTask_LeaveOverworld`.
    LeaveOverworld,
    /// `PlaySE`.
    PlaySe(u16),
    /// `StopBGM` — stop the current BGM.
    StopBgm,
    /// `TempBGM` — `sub_02005E44`.
    TempBgm(u16),
    /// `PlayBGM`.
    PlayBgm(u16),
    /// `ResetBGM` — replay the map's own BGM.
    ResetBgm,
    /// `FadeOutBGM` — `GF_SndStartFadeOutBGM(seq, length)`.
    FadeOutBgm {
        /// The sequence.
        seq: u16,
        /// The fade length.
        length: u16,
    },
    /// `FadeInBGM` — `GF_SndStartFadeInBGM(0x7f, length, 0)`.
    FadeInBgm(u16),
    /// `PlayCry` — `PlayCryEx(species, form, 0, 100, 0x20, 0)`.
    PlayCry {
        /// The species (the command's second operand).
        species: u16,
        /// The form (the command's first operand).
        form: u16,
    },
    /// `PlayFanfare`.
    PlayFanfare(u16),
    /// `Signpost_SetParam(kind, map)`.
    SignpostSet {
        /// The signpost type.
        kind: u8,
        /// The map it names.
        map: u16,
    },
    /// `Signpost_SetCommand` (`MAPSIGNCOMMAND_*`).
    SignpostCommand(u8),
    /// `Signpost_DoCurrentCommand`.
    SignpostDoCurrent,
    /// `RemoveTextPrinter` on the last print.
    RemoveTextPrinter,
    /// `HoldMsg` — remove the dialogue window without clearing it.
    DialogHold,
    /// `ScrCmd_ToggleFollowingPokemonMovement` (only while a follower
    /// is active): pause (`true`) or unpause it.
    FollowMonPause(bool),
    /// `ScrCmd_FollowingPokemonMovement` — `sub_0205FC94(follower, movement)`.
    FollowMonMovement(u16),
    /// `ScrCmd_605` — `ov01_02205720(player, follower, a, b)`.
    FollowMonEffect605 {
        /// First byte operand.
        a: u8,
        /// Second byte operand.
        b: u8,
    },
    /// `ScrCmd_608` — `ov01_02205784(follower)`.
    FollowMonEffect608,
    /// `ScrCmd_609` — `sub_020659CC(follower)`.
    FollowMonEffect609,
    /// `Bag_AddItem` — returns 1 on success.
    BagAdd {
        /// The item.
        item: u16,
        /// How many.
        quantity: u16,
    },
    /// `Bag_TakeItem` — returns 1 on success.
    BagTake {
        /// The item.
        item: u16,
        /// How many.
        quantity: u16,
    },
    /// `SavePokegear_RegisterPhoneNumber`.
    RegisterPhoneNumber(u8),
    /// `PhoneCallPersistentState_ClearCallTriggerFlag`.
    ClearPhoneCallTrigger(u8),
    /// `HealParty`.
    HealParty,
    /// `GiveRibbon` — `SetMonData(mon, ribbon attr, TRUE)`.
    GiveRibbon {
        /// Party slot.
        slot: u16,
        /// Ribbon id.
        ribbon: u16,
    },
    /// `ScrCmd_582` — set the special spawn (map, x, y; warp -1, south).
    SetSpecialSpawn {
        /// The map.
        map: u16,
        /// Tile x.
        x: u16,
        /// Tile y.
        y: u16,
    },
    /// `MapPropManager_LoadOne(prop, {x, 0, z})`.
    LoadMapProp {
        /// The prop model id.
        prop: u16,
        /// World x (tiles).
        x: u16,
        /// World z (tiles).
        z: u16,
    },
    /// `CameronPhoto` — `FieldSystem_TakePhoto`.
    TakePhoto(u16),
    /// `ScrCmd_795` — show the money box at a tile position.
    MoneyBoxShow {
        /// x.
        x: u8,
        /// y.
        y: u8,
    },
    /// `ScrCmd_796` — hide the money box.
    MoneyBoxHide,
    /// `ov01_021F6A9C(mode, 0)` — the lower-screen field menu: 0 show,
    /// 3 hide.
    TouchscreenMenu(u8),
    /// `ov01_021F6ABC(3, 3, ...)` — start reading the lower-screen
    /// menu's choice.
    MenuChoiceBegin,
    /// `MenuInit` — open the scripted list menu (`ov01_021EDF78`).
    MenuInit {
        /// Window x.
        x: u8,
        /// Window y.
        y: u8,
        /// Initial cursor.
        cursor: u8,
        /// Whether B cancels.
        cancellable: u8,
        /// The variable the menu writes its choice to (`ret_p`).
        result_var: u16,
    },
    /// `MenuItemAdd` — `MoveTutorMenu_SetListItem`.
    MenuAddItem {
        /// The item's text (the context bank's message).
        text: GameString,
        /// Where in the list.
        position: u16,
        /// The value it selects.
        value: u16,
    },
    /// `MenuExec` — run the list menu.
    MenuExec,
    /// `BankTransaction` — Mom's savings deposit/withdraw UI.
    BankTransaction {
        /// The transaction mode.
        mode: u16,
    },
    /// `YesNo` — open the yes/no menu.
    YesNoOpen,
    /// `ScrCmd_307` — `ov01_021E9AE8(x, y, kind)` (an overlay-1 field
    /// effect pret has not named).
    Effect307 {
        /// x (tiles, block-relative already applied).
        x: u32,
        /// y.
        y: u32,
        /// kind.
        kind: u8,
    },
    /// `ScrCmd_308` — `ov01_021E9C00(arg)`.
    Effect308(u8),
    /// `ScrCmd_309` — `ov01_021E9C20(arg)`.
    Effect309(u8),
    /// `ScrCmd_310` — `ov01_021E9BB8(arg)`.
    Effect310(u8),
    /// `ScrCmd_311` — `ov01_021E9BDC(arg)`.
    Effect311(u8),
}

/// A read of field or save state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldQuery {
    /// `PlayerProfile_GetTrainerGender` (0 male, 1 female).
    PlayerGender,
    /// `PlayerAvatar_GetFacingDirection`.
    PlayerFacing,
    /// `Party_GetCount`.
    PartyCount,
    /// `PlayerProfile_GetMoney`.
    Money,
    /// `Mailbox_CountMessages(mailbox, 0)`.
    MailboxCount,
    /// `Field_GetTimeOfDay`.
    TimeOfDay,
    /// `GF_RTC_CopyDate(&date).week`.
    Weekday,
    /// `Save_GetPartyLeadAlive`.
    PartyLeadAlive,
    /// `PhotoAlbum_GetNumSaved`.
    PhotoCount,
    /// `PlayerProfile_TestBadgeFlag(badge)`.
    HasBadge(u16),
    /// Mom's savings balance (`MOMS_BALANCE_GET`).
    BankBalance,
    /// `FollowMon_IsActive`.
    FollowMonActive,
    /// `ov01_022055DC(follower)` (`ScrCmd_596`).
    FollowMonState596,
    /// `ov01_02205D68(fieldSystem)` (`ScrCmd_600`).
    FollowMonState600,
    /// `Options_GetTextFrameDelay`.
    TextFrameDelay,
    /// `GetItemAttr(item, ITEMATTR_FIELD_POCKET)`.
    ItemPocket(u16),
    /// `Bag_HasItem(item, quantity)`.
    BagHasItem {
        /// The item.
        item: u16,
        /// How many.
        quantity: u16,
    },
    /// `Bag_HasSpaceForItem(item, quantity)`.
    BagHasSpace {
        /// The item.
        item: u16,
        /// How many.
        quantity: u16,
    },
    /// `GetMonData(mon, ribbon attr)`.
    MonHasRibbon {
        /// Party slot.
        slot: u16,
        /// Ribbon id.
        ribbon: u16,
    },
    /// `ov01_021F6B00` — the lower-screen menu's current mode.
    TouchscreenMenuMode,
}

/// A NATIVE-mode wait the host answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaitFor {
    /// `DialogBox_IsPrintFinished` on the last print.
    PrintFinished,
    /// `activeMovementCounter == 0` — every applied movement done.
    MovementFinished,
    /// The `LockAll`/`LockLastTalked` pause dance has settled.
    LockSettled,
    /// `MapObject_IsMovementPaused(follower)`.
    FollowMonPaused,
    /// `!IsSEPlaying(seq)`.
    SeFinished(u16),
    /// `IsCryFinished`.
    CryFinished,
    /// `!IsFanfarePlaying`.
    FanfareFinished,
    /// `IsPaletteFadeFinished`.
    FadeFinished,
    /// `!GF_SndGetFadeTimer`.
    BgmFadeFinished,
    /// `Signpost_CommandIsFinished`.
    SignpostCommandFinished,
    /// The launched application returned (its result as the value).
    App,
    /// The `TaskManager_Call` child task the last action pushed (a
    /// warp, `CallTask_RestoreOverworld`, `FieldSystem_TakePhoto`, ...)
    /// has returned — the script task does not run while it is up.
    ChildTask,
    /// The yes/no menu chose (0 yes, 1 no).
    YesNo,
    /// The lower-screen menu reached `mode` (0 shown, 3 hidden).
    TouchscreenMenu(u8),
    /// The lower-screen menu's choice (the value).
    MenuChoice,
    /// The scripted list menu's result (the value).
    MenuExec,
    /// The bank-transaction result (the value).
    BankTransaction,
}

/// An application the script hands control to (`CallTask_*`,
/// `LaunchStarterChoiceScene`, ...); the VM waits on [`WaitFor::App`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppRequest {
    /// `CallTask_NamingScreen`.
    NamingScreen {
        /// `NAME_SCREEN_*` (1 rival, 2 Pokémon — pret's enum order).
        kind: u8,
        /// The species, for a Pokémon.
        species: u16,
        /// Maximum length.
        max_len: u8,
        /// The party slot, for a Pokémon.
        party_slot: u16,
        /// The initial text (the current nickname).
        initial: GameString,
    },
    /// `LaunchStarterChoiceScene`.
    ChooseStarter,
    /// `SetupAndStartTutorialBattle`.
    CatchingTutorial,
    /// `ScrCmd_376` — the mail application.
    Mail,
}

/// The field, as the VM sees it.
pub trait ScriptHost {
    /// The save's flags-and-vars block (`Save_VarsFlags_Get`).
    fn vars_flags(&mut self) -> VarsFlags<&mut [u8]>;
    /// The non-saved temporary flags (`sTempFlags`).
    fn temp_flags(&mut self) -> &mut TempFlags;
    /// The field RNG (`LCRandom`).
    fn rng(&mut self) -> &mut Lcrng;
    /// `gSystem.newKeys` — the buttons newly pressed this frame.
    fn new_keys(&self) -> Keys;

    /// The raw bytes of `a/0/1/2` member `bank`.
    fn load_scripts(&mut self, bank: u16) -> Option<Vec<u8>>;
    /// The raw bytes of `a/0/2/7` member `bank`.
    fn load_messages(&mut self, bank: u16) -> Option<Vec<u8>>;
    /// The current map header's script and message banks.
    fn current_map_banks(&self) -> MapBanks;

    /// `PlayerProfile_GetNamePtr`.
    fn player_name(&self) -> GameString;
    /// `Save_Misc_RivalName_Const_Get`.
    fn rival_name(&self) -> GameString;
    /// `Party_GetMonByIndex(slot)`, or `None` past the party.
    fn party_mon(&self, slot: u16) -> Option<PartyMon>;
    /// The tile position of `object` — the player for `None`; `None`
    /// when no such object is active.
    fn object_position(&self, object: Option<u16>) -> Option<(u16, u16)>;

    /// `DialogBox_AddWindowToLayer3` + `DialogBox_LoadFrame` — open the
    /// dialogue window.
    fn dialog_open(&mut self);
    /// `ClearFrameAndWindow2` + `RemoveWindow` — close it.
    fn dialog_close(&mut self);
    /// Start printing `text` (already placeholder-expanded).
    fn print(&mut self, target: PrintTarget, text: &GameString, params: PrintParams);
    /// `EventObjectMovementMan_Create` + schedule — start `object`
    /// walking `steps`; `false` when there is no such object.
    fn apply_movement(&mut self, object: u16, steps: &[MovementCommand]) -> bool;

    /// Perform `action`; the meaning of the return value is
    /// per-variant (0 where it has none).
    fn action(&mut self, action: FieldAction) -> u32;
    /// Answer `query`.
    fn query(&self, query: FieldQuery) -> u32;
    /// Poll `wait`: `None` while pending, `Some(value)` once satisfied
    /// (`0` for waits that carry no value).
    fn poll(&mut self, wait: WaitFor) -> Option<u16>;
    /// Hand control to an application; the VM then polls
    /// [`WaitFor::App`].
    fn launch(&mut self, app: AppRequest);
}

/// One call the [`RecordingHost`] saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// [`ScriptHost::dialog_open`].
    DialogOpen,
    /// [`ScriptHost::dialog_close`].
    DialogClose,
    /// [`ScriptHost::print`].
    Print {
        /// Where.
        target: PrintTarget,
        /// The expanded text.
        text: GameString,
        /// How.
        params: PrintParams,
    },
    /// [`ScriptHost::apply_movement`].
    Movement {
        /// The object.
        object: u16,
        /// The steps.
        steps: Vec<MovementCommand>,
    },
    /// [`ScriptHost::action`].
    Action(FieldAction),
    /// [`ScriptHost::launch`].
    Launch(AppRequest),
    /// [`ScriptHost::poll`] — one per frame while a native waits.
    Poll(WaitFor),
}

/// The bank loader a [`RecordingHost`] falls back to: `(narc path,
/// member) -> bytes`.
pub type BankLoader = Box<dyn FnMut(&str, u16) -> Option<Vec<u8>>>;

/// A mock host: every call is logged, every answer comes from a table.
pub struct RecordingHost {
    /// The `SaveVarsFlags` block.
    pub block: Vec<u8>,
    /// The temporary flags.
    pub temp_flags: TempFlags,
    /// The field RNG.
    pub rng: Lcrng,
    /// This frame's newly pressed keys.
    pub keys: Keys,
    /// Script banks by member id (consulted before `loader`).
    pub scripts: HashMap<u16, Vec<u8>>,
    /// Message banks by member id (consulted before `loader`).
    pub messages: HashMap<u16, Vec<u8>>,
    /// Where banks not in the tables come from (a ROM, in the
    /// ROM-gated tests).
    pub loader: Option<BankLoader>,
    /// The current map's banks.
    pub map_banks: MapBanks,
    /// The player's name.
    pub player_name: GameString,
    /// The rival's name.
    pub rival_name: GameString,
    /// The party.
    pub party: Vec<PartyMon>,
    /// The player's tile position.
    pub player_position: (u16, u16),
    /// Active objects' tile positions.
    pub objects: HashMap<u16, (u16, u16)>,
    /// Query answers (0 when absent).
    pub queries: HashMap<FieldQuery, u32>,
    /// Action return values, consumed in order (0 when exhausted).
    pub action_results: VecDeque<u32>,
    /// How many polls of a wait answer "pending" before it completes
    /// (0 when absent: immediately).
    pub pending: HashMap<WaitFor, u32>,
    /// The value a completed wait carries (0 when absent).
    pub poll_values: HashMap<WaitFor, u16>,
    /// Everything that happened.
    pub events: Vec<HostEvent>,
}

impl fmt::Debug for RecordingHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordingHost")
            .field("keys", &self.keys)
            .field("map_banks", &self.map_banks)
            .field("events", &self.events)
            .finish_non_exhaustive()
    }
}

impl Default for RecordingHost {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingHost {
    /// A blank host: zeroed flags and vars, RNG seed 0, no keys, no
    /// banks, empty names, an empty party, the player at (0, 0).
    #[must_use]
    pub fn new() -> Self {
        Self {
            block: vec![0; VARS_FLAGS_SIZE],
            temp_flags: TempFlags::new(),
            rng: Lcrng::new(0),
            keys: Keys::IDLE,
            scripts: HashMap::new(),
            messages: HashMap::new(),
            loader: None,
            map_banks: MapBanks::default(),
            player_name: GameString::new(),
            rival_name: GameString::new(),
            party: Vec::new(),
            player_position: (0, 0),
            objects: HashMap::new(),
            queries: HashMap::new(),
            action_results: VecDeque::new(),
            pending: HashMap::new(),
            poll_values: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// The logged actions, in order.
    #[must_use]
    pub fn actions(&self) -> Vec<&FieldAction> {
        self.events
            .iter()
            .filter_map(|e| match e {
                HostEvent::Action(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    /// The logged prints' texts, in order.
    #[must_use]
    pub fn prints(&self) -> Vec<&GameString> {
        self.events
            .iter()
            .filter_map(|e| match e {
                HostEvent::Print { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    /// The events other than polls, in order.
    #[must_use]
    pub fn events_without_polls(&self) -> Vec<&HostEvent> {
        self.events
            .iter()
            .filter(|e| !matches!(e, HostEvent::Poll(_)))
            .collect()
    }
}

impl ScriptHost for RecordingHost {
    fn vars_flags(&mut self) -> VarsFlags<&mut [u8]> {
        VarsFlags::new(self.block.as_mut_slice()).expect("RecordingHost block is a whole SaveVarsFlags")
    }

    fn temp_flags(&mut self) -> &mut TempFlags {
        &mut self.temp_flags
    }

    fn rng(&mut self) -> &mut Lcrng {
        &mut self.rng
    }

    fn new_keys(&self) -> Keys {
        self.keys
    }

    fn load_scripts(&mut self, bank: u16) -> Option<Vec<u8>> {
        if let Some(bytes) = self.scripts.get(&bank) {
            return Some(bytes.clone());
        }
        self.loader.as_mut()?(super::bank::SCRIPT_NARC, bank)
    }

    fn load_messages(&mut self, bank: u16) -> Option<Vec<u8>> {
        if let Some(bytes) = self.messages.get(&bank) {
            return Some(bytes.clone());
        }
        self.loader.as_mut()?(super::bank::MSG_NARC, bank)
    }

    fn current_map_banks(&self) -> MapBanks {
        self.map_banks
    }

    fn player_name(&self) -> GameString {
        self.player_name.clone()
    }

    fn rival_name(&self) -> GameString {
        self.rival_name.clone()
    }

    fn party_mon(&self, slot: u16) -> Option<PartyMon> {
        self.party.get(usize::from(slot)).cloned()
    }

    fn object_position(&self, object: Option<u16>) -> Option<(u16, u16)> {
        match object {
            None => Some(self.player_position),
            Some(id) => self.objects.get(&id).copied(),
        }
    }

    fn dialog_open(&mut self) {
        self.events.push(HostEvent::DialogOpen);
    }

    fn dialog_close(&mut self) {
        self.events.push(HostEvent::DialogClose);
    }

    fn print(&mut self, target: PrintTarget, text: &GameString, params: PrintParams) {
        self.events.push(HostEvent::Print {
            target,
            text: text.clone(),
            params,
        });
    }

    fn apply_movement(&mut self, object: u16, steps: &[MovementCommand]) -> bool {
        self.events.push(HostEvent::Movement {
            object,
            steps: steps.to_vec(),
        });
        object == OBJ_PLAYER || self.objects.contains_key(&object)
    }

    fn action(&mut self, action: FieldAction) -> u32 {
        self.events.push(HostEvent::Action(action));
        self.action_results.pop_front().unwrap_or(0)
    }

    fn query(&self, query: FieldQuery) -> u32 {
        self.queries.get(&query).copied().unwrap_or(0)
    }

    fn poll(&mut self, wait: WaitFor) -> Option<u16> {
        self.events.push(HostEvent::Poll(wait));
        match self.pending.get_mut(&wait) {
            Some(n) if *n > 0 => {
                *n -= 1;
                None
            }
            _ => Some(self.poll_values.get(&wait).copied().unwrap_or(0)),
        }
    }

    fn launch(&mut self, app: AppRequest) {
        self.events.push(HostEvent::Launch(app));
    }
}
