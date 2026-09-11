//! The command handlers — the early-game subset of `gScriptCmdTable`,
//! each a port of its `ScrCmd_*` (`src/scrcmd_c.c`, `src/scrcmd_sound.c`,
//! `src/scrcmd_items.c`, `src/scrcmd_strbuf.c`, `src/scrcmd_party.c`,
//! `src/scrcmd_money.c`, `src/scrcmd_lottery.c`, `src/scrcmd_17.c`,
//! `src/field/scrcmd_message.c`, `src/field/headbutt.c`,
//! `src/field/scrcmd_pokemon_misc.c`), plus the NATIVE-mode waits they
//! install.
//!
//! Every handler reads its operands in the C's order with the C's
//! widths (`ScriptReadByte`/`Halfword`/`Word`; `ScriptGetVar` resolves
//! a halfword through `FieldSystem_VarGet`, `ScriptGetVarPointer` names
//! a variable to write), and returns [`Flow::Yield`] exactly where the
//! C returns `TRUE`. Field effects go through the [`ScriptHost`].
//! Anything not listed here is [`ScriptError::Unimplemented`].
//!
//! The subset is the opcode inventory of the banks the early game runs
//! (`tests/script_hg.rs`: std init 149, New Bark Town 842, the player's
//! house 845/846, Elm's lab 843, Route 29 225, and the `std_misc`
//! scripts they `CallStd`), plus the trivial neighbours of the
//! control-flow, flag, variable, message and BGM families.

use crate::formats::MsgBank;
use crate::input::key;
use crate::save::vars_flags::{
    TRAINER_FLAG_BASE, VAR_LOTO_NUMBER_LO, VAR_PLAYER_STARTER, is_saved_var, is_special_var,
};
use crate::text::string::GameString;

use super::ScriptError;
use super::commands::{Opcode, decode_movement};
use super::context::{CONDITION_TABLE, NativeWait, ScriptContext, compare};
use super::env::ScriptEnvironment;
use super::host::{
    AppRequest, FieldAction, FieldQuery, PrintParams, PrintTarget, ScriptHost, WaitFor, dir,
};

/// What a handler returns: the C's `FALSE` (run the next command now)
/// or `TRUE` (yield the frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Keep executing.
    Continue,
    /// Yield the frame.
    Yield,
}

/// `PAD_BUTTON_A | PAD_BUTTON_B`.
const AB: u16 = key::A | key::B;
/// `PAD_KEY_UP | PAD_KEY_DOWN | PAD_KEY_LEFT | PAD_KEY_RIGHT`.
const DPAD: u16 = key::UP | key::DOWN | key::LEFT | key::RIGHT;

/// `ov01_022067C8` (`src/field/scrcmd_message.c:38`) — the banks
/// `GetStdMsgNaix` selects: day-of-week siblings, field moves,
/// Cameron the photographer, shops.
const STD_MSG_BANKS: [u16; 4] = [752, 211, 30, 435];
/// `NARC_msg_msg_0222_bin` — item names (`BufferItemName`).
const ITEM_NAMES_BANK: u16 = 222;
/// `NARC_msg_msg_0224_bin` — plural item names.
const ITEM_NAMES_PLURAL_BANK: u16 = 224;
/// `NARC_msg_msg_0226_bin` — pocket names.
const POCKET_NAMES_BANK: u16 = 226;
/// `NARC_msg_msg_0237_bin` — species names.
const SPECIES_NAMES_BANK: u16 = 237;
/// `NARC_msg_msg_0238_bin` — species names with an article.
const SPECIES_NAMES_ARTICLE_BANK: u16 = 238;
/// `NARC_msg_msg_0445_bin` — the friend's names (0 Ethan, 1 Lyra).
const FRIEND_NAMES_BANK: u16 = 445;
/// `GAME_VERSION` = `VERSION_HEARTGOLD` (`include/config.h:9,30`).
const GAME_VERSION: u16 = 7;
/// `SPRITE_HERO` / `SPRITE_HEROINE` (`include/constants/sprites.h`).
const SPRITE_HERO: u16 = 0;
const SPRITE_HEROINE: u16 = 97;
/// `NUM_PHONE_CONTACTS` (`include/constants/phone_contacts.h:80`).
const NUM_PHONE_CONTACTS: u16 = 75;
/// `MAX_MONEY` (`include/player_data.h:10`).
const MAX_MONEY: u32 = 999_999;
/// `MAPSIGNCOMMAND_SHOW` (`include/constants/scrcmd.h`).
const MAPSIGNCOMMAND_SHOW: u8 = 1;
/// `NAME_SCREEN_RIVAL` / `NAME_SCREEN_POKEMON` (`include/naming_screen.h`).
const NAME_SCREEN_RIVAL: u8 = 1;
const NAME_SCREEN_POKEMON: u8 = 2;
/// `PLAYER_NAME_LENGTH` / `POKEMON_NAME_LENGTH`.
const PLAYER_NAME_LENGTH: u8 = 7;
const POKEMON_NAME_LENGTH: u8 = 10;
/// `FLAG_GOT_TM51_FROM_FALKNER` / `FLAG_MET_PASSERBY_BOY`
/// (`include/constants/flags.h:134,172`) — `PlaceStarterBallsInElmsLab`.
const FLAG_GOT_TM51_FROM_FALKNER: u16 = 0x73;
const FLAG_MET_PASSERBY_BOY: u16 = 0x99;
/// The Poké Ball prop and its three desk positions
/// (`ScrCmd_PlaceStarterBallsInElmsLab`, `src/scrcmd_c.c:4762`).
const STARTER_BALL_PROP: u16 = 0x8D;
const STARTER_BALL_COORDS: [(u16, u16); 3] = [(131, 65), (141, 65), (136, 72)];
/// `MAKE_TEXT_COLOR(2, 10, 15)` — the signpost text colours.
const SIGNPOST_COLOR: [u8; 3] = [2, 10, 15];

/// One command's execution state: the context, the environment, the
/// host, and where the opcode sat (for errors).
struct Cmd<'a> {
    ctx: &'a mut ScriptContext,
    env: &'a mut ScriptEnvironment,
    host: &'a mut dyn ScriptHost,
    offset: usize,
}

impl Cmd<'_> {
    fn u8(&mut self) -> Result<u8, ScriptError> {
        self.ctx.read_u8()
    }

    fn u16(&mut self) -> Result<u16, ScriptError> {
        self.ctx.read_u16()
    }

    fn u32(&mut self) -> Result<u32, ScriptError> {
        self.ctx.read_u32()
    }

    fn bad_var(&self, var: u16) -> ScriptError {
        ScriptError::BadVar {
            var,
            offset: self.offset,
        }
    }

    /// `ScriptGetVar` — a halfword, resolved (`FieldSystem_VarGet`).
    fn var(&mut self) -> Result<u16, ScriptError> {
        let id = self.u16()?;
        self.env
            .var_get(self.host, id)
            .ok_or_else(|| self.bad_var(id))
    }

    /// `ScriptGetVarPointer` — a halfword naming a writable variable.
    fn var_ref(&mut self) -> Result<u16, ScriptError> {
        let id = self.u16()?;
        if is_saved_var(id) || is_special_var(id) {
            Ok(id)
        } else {
            Err(self.bad_var(id))
        }
    }

    /// `*pointer` for a variable named by [`Self::var_ref`].
    fn read(&mut self, var: u16) -> Result<u16, ScriptError> {
        self.env
            .var_get(self.host, var)
            .ok_or_else(|| self.bad_var(var))
    }

    /// `*pointer = value`.
    fn write(&mut self, var: u16, value: u16) -> Result<(), ScriptError> {
        if self.env.var_set(self.host, var, value) {
            Ok(())
        } else {
            Err(self.bad_var(var))
        }
    }

    fn query(&self, query: FieldQuery) -> u32 {
        self.host.query(query)
    }

    fn action(&mut self, action: FieldAction) -> u32 {
        self.host.action(action)
    }

    fn wait(&mut self, wait: WaitFor) -> Flow {
        self.ctx.set_native(NativeWait::Poll(wait));
        Flow::Yield
    }

    fn native(&mut self, wait: NativeWait) -> Flow {
        self.ctx.set_native(wait);
        Flow::Yield
    }

    /// `sConditionTable[condition][comparisonResult]`.
    fn condition(&self, condition: u8) -> bool {
        CONDITION_TABLE
            .get(usize::from(condition))
            .and_then(|row| row.get(usize::from(self.ctx.comparison())))
            .is_some_and(|&hit| hit == 1)
    }

    /// The context bank's message `id`, raw.
    fn message(&self, id: u16) -> Result<Vec<u16>, ScriptError> {
        self.ctx.message(id).map(<[u16]>::to_vec)
    }

    /// Message `id` of `a/0/2/7` member `bank`, raw
    /// (`NewMsgDataFromNarc` + `ReadMsgDataIntoString`).
    fn external_message(&mut self, bank: u16, id: u16) -> Result<Vec<u16>, ScriptError> {
        let bytes = self
            .host
            .load_messages(bank)
            .ok_or(ScriptError::MessagesMissing { bank })?;
        let parsed = MsgBank::parse(&bytes).map_err(|_| ScriptError::MessagesMissing { bank })?;
        parsed
            .message(usize::from(id))
            .map(<[u16]>::to_vec)
            .ok_or(ScriptError::NoSuchMessage { bank, id })
    }

    /// `ReadMsgDataIntoString` + `SetStringAsPlaceholder` into field
    /// `field` from an external bank (the `Buffer*Name` helpers).
    fn buffer_external(&mut self, bank: u16, id: u16, field: u8) -> Result<(), ScriptError> {
        let units = self.external_message(bank, id)?;
        self.env
            .msgfmt_mut()
            .set_string(usize::from(field), &GameString::from_units(&units));
        Ok(())
    }

    /// `ReadMsgDataIntoString` into buffer 1, `StringExpandPlaceholders`
    /// into buffer 0 (`ovFieldMain_ReadAndExpandMsgDataViaBuffer`).
    fn expand(&mut self, units: &[u16]) -> Result<GameString, ScriptError> {
        let expanded = self.env.message_format().expand_placeholders(units)?;
        self.env
            .set_buffers(GameString::from_units(units), expanded.clone());
        Ok(expanded)
    }

    /// `ov01_021EF4DC` (`src/field/scrcmd_message.c:206`): create the
    /// dialogue window if `unk_8` says there is none
    /// (`ovFieldMain_CreateMessageBox`), expand `units`, and print with
    /// font 1 at the options' speed. The caller installs the
    /// `ov01_021EF348` wait.
    fn show(&mut self, units: &[u16], can_ab_speed_up: bool, flag: u8) -> Result<(), ScriptError> {
        if !self.env.window_open() {
            self.host.dialog_open();
            self.env.set_window_open(true);
            self.env.set_textbox_open(true);
        }
        let text = self.expand(units)?;
        let params = PrintParams {
            font: 1,
            frame_delay: self.query(FieldQuery::TextFrameDelay),
            can_ab_speed_up,
            flag,
            instant: false,
            color: None,
        };
        self.host.print(PrintTarget::Dialog, &text, params);
        Ok(())
    }

    /// The `PAD_KEY_*` newly pressed, as a `DIR_*` (`sub_02041074`'s
    /// priority: up, down, left, right).
    fn pressed_direction(&self) -> Option<u16> {
        let keys = self.host.new_keys();
        if keys.any(key::UP) {
            Some(dir::NORTH)
        } else if keys.any(key::DOWN) {
            Some(dir::SOUTH)
        } else if keys.any(key::LEFT) {
            Some(dir::WEST)
        } else if keys.any(key::RIGHT) {
            Some(dir::EAST)
        } else {
            None
        }
    }
}

/// Runs the handler for `opcode`, whose halfword sat at `offset`.
pub(crate) fn execute(
    opcode: Opcode,
    offset: usize,
    ctx: &mut ScriptContext,
    env: &mut ScriptEnvironment,
    host: &mut dyn ScriptHost,
) -> Result<Flow, ScriptError> {
    let mut c = Cmd {
        ctx,
        env,
        host,
        offset,
    };
    use Flow::{Continue, Yield};
    let flow = match opcode {
        // ---- control flow, registers, comparisons (scrcmd_c.c:153-465) ----
        Opcode::Nop | Opcode::Dummy | Opcode::Dummy486 => Continue,
        Opcode::End => {
            c.ctx.stop();
            Continue
        }
        Opcode::Wait => {
            let frames = c.u16()?;
            let var = c.u16()?;
            c.write(var, frames)?;
            c.ctx.set_data(0, u32::from(var));
            c.native(NativeWait::PauseTimer)
        }
        Opcode::LoadByte => {
            let reg = c.u8()?;
            let value = c.u8()?;
            c.ctx.set_register(reg, u32::from(value), offset)?;
            Continue
        }
        Opcode::LoadWord => {
            let reg = c.u8()?;
            let value = c.u32()?;
            c.ctx.set_register(reg, value, offset)?;
            Continue
        }
        Opcode::CopyLocal => {
            let dest = c.u8()?;
            let src = c.u8()?;
            let value = c.ctx.register(src, offset)?;
            c.ctx.set_register(dest, value, offset)?;
            Continue
        }
        Opcode::CompareLocalToLocal => {
            // `u8 a = ctx->data[..]`: the registers are truncated.
            let ra = c.u8()?;
            let rb = c.u8()?;
            let a = c.ctx.register(ra, offset)? as u8;
            let b = c.ctx.register(rb, offset)? as u8;
            c.ctx.set_comparison(compare(u32::from(a), u32::from(b)));
            Continue
        }
        Opcode::CompareLocalToValue => {
            let ra = c.u8()?;
            let a = c.ctx.register(ra, offset)? as u8;
            let b = c.u8()?;
            c.ctx.set_comparison(compare(u32::from(a), u32::from(b)));
            Continue
        }
        Opcode::CompareVarToValue => {
            let var = c.var_ref()?;
            let a = c.read(var)?;
            let b = c.u16()?;
            c.ctx.set_comparison(compare(u32::from(a), u32::from(b)));
            Continue
        }
        Opcode::CompareVarToVar => {
            let a_var = c.var_ref()?;
            let b_var = c.var_ref()?;
            let a = c.read(a_var)?;
            let b = c.read(b_var)?;
            c.ctx.set_comparison(compare(u32::from(a), u32::from(b)));
            Continue
        }
        Opcode::CallStd => {
            let script = c.u16()?;
            c.env.spawn_std_context(c.host, script)?;
            let mask = c.env.std_wait_mask() | (1 << c.ctx.id);
            c.env.set_std_wait_mask(mask);
            c.native(NativeWait::WaitStd)
        }
        Opcode::RestartCurrentScript => {
            // `*unk ^= 1 << (ctx->id - 1)` — only meaningful in a callee.
            if c.ctx.id > 0 {
                let mask = c.env.std_wait_mask() ^ (1 << (c.ctx.id - 1));
                c.env.set_std_wait_mask(mask);
            }
            Continue
        }
        Opcode::GoTo => {
            let word = c.u32()?;
            let target = c.ctx.relative(word);
            c.ctx.jump(target);
            Continue
        }
        Opcode::ObjectGoTo => {
            let id = c.u8()?;
            let word = c.u32()?;
            if c.env.last_interacted() == Some(u16::from(id)) {
                let target = c.ctx.relative(word);
                c.ctx.jump(target);
            }
            Continue
        }
        Opcode::DirectionGoTo => {
            let direction = c.u8()?;
            let word = c.u32()?;
            if c.env.facing_direction() == u16::from(direction) {
                let target = c.ctx.relative(word);
                c.ctx.jump(target);
            }
            Continue
        }
        Opcode::Call => {
            let word = c.u32()?;
            let target = c.ctx.relative(word);
            c.ctx.call(target);
            Continue
        }
        Opcode::Return => {
            c.ctx.ret();
            Continue
        }
        Opcode::GoToIf => {
            let condition = c.u8()?;
            let word = c.u32()?;
            if c.condition(condition) {
                let target = c.ctx.relative(word);
                c.ctx.jump(target);
            }
            Continue
        }
        Opcode::CallIf => {
            let condition = c.u8()?;
            let word = c.u32()?;
            if c.condition(condition) {
                let target = c.ctx.relative(word);
                c.ctx.call(target);
            }
            Continue
        }

        // ---- flags (scrcmd_c.c:465-543) ----
        Opcode::SetFlag => {
            let flag = c.u16()?;
            ScriptEnvironment::flag_set(c.host, flag);
            Continue
        }
        Opcode::ClearFlag => {
            let flag = c.u16()?;
            ScriptEnvironment::flag_clear(c.host, flag);
            Continue
        }
        Opcode::CheckFlag => {
            let flag = c.u16()?;
            let set = ScriptEnvironment::flag_check(c.host, flag);
            c.ctx.set_comparison(u8::from(set));
            Continue
        }
        Opcode::CheckFlagVar => {
            let flag_var = c.var_ref()?;
            let ret = c.var_ref()?;
            let flag = c.read(flag_var)?;
            let set = ScriptEnvironment::flag_check(c.host, flag);
            c.write(ret, u16::from(set))?;
            Continue
        }
        Opcode::SetFlagVar => {
            let flag_var = c.var_ref()?;
            let flag = c.read(flag_var)?;
            ScriptEnvironment::flag_set(c.host, flag);
            Continue
        }
        Opcode::ClearFlagVar => {
            let flag_var = c.var_ref()?;
            let flag = c.read(flag_var)?;
            ScriptEnvironment::flag_clear(c.host, flag);
            Continue
        }
        Opcode::SetTrainerFlag => {
            let trainer = c.var()?;
            ScriptEnvironment::flag_set(c.host, trainer.wrapping_add(TRAINER_FLAG_BASE));
            Continue
        }
        Opcode::ClearTrainerFlag => {
            let trainer = c.var()?;
            ScriptEnvironment::flag_clear(c.host, trainer.wrapping_add(TRAINER_FLAG_BASE));
            Continue
        }
        Opcode::CheckTrainerFlag => {
            let trainer = c.var()?;
            let set = ScriptEnvironment::flag_check(c.host, trainer.wrapping_add(TRAINER_FLAG_BASE));
            c.ctx.set_comparison(u8::from(set));
            Continue
        }

        // ---- variables (scrcmd_c.c:545-588) ----
        Opcode::AddVar => {
            let dest = c.var_ref()?;
            let addend = c.var()?;
            let value = c.read(dest)?.wrapping_add(addend);
            c.write(dest, value)?;
            Continue
        }
        Opcode::SubVar => {
            let dest = c.var_ref()?;
            let subtrahend = c.var()?;
            let value = c.read(dest)?.wrapping_sub(subtrahend);
            c.write(dest, value)?;
            Continue
        }
        Opcode::SetVar => {
            let dest = c.var_ref()?;
            let value = c.u16()?;
            c.write(dest, value)?;
            Continue
        }
        Opcode::CopyVar => {
            let dest = c.var_ref()?;
            let src = c.var_ref()?;
            let value = c.read(src)?;
            c.write(dest, value)?;
            Continue
        }
        Opcode::SetOrCopyVar => {
            let dest = c.var_ref()?;
            let value = c.var()?;
            c.write(dest, value)?;
            Continue
        }
        Opcode::Random => {
            let ret = c.var_ref()?;
            let modulo = c.var()?;
            let draw = c.host.rng().next_u16();
            // `LCRandom() % modulo` — a zero modulus is a hardware fault
            // the scripts never risk; read it as 0 rather than trap.
            c.write(ret, draw.checked_rem(modulo).unwrap_or(0))?;
            Yield
        }

        // ---- input waits (scrcmd_c.c:609-680) ----
        Opcode::WaitABPress => c.native(NativeWait::WaitAbPress),
        Opcode::WaitButtonOrDelay => {
            let frames = c.var()?;
            c.ctx.set_data(0, u32::from(frames));
            c.native(NativeWait::WaitButtonOrDelay)
        }
        Opcode::WaitButton => c.native(NativeWait::WaitButton),
        Opcode::WaitButtonOrDpad => c.native(NativeWait::WaitButtonOrDpad),

        // ---- the dialogue window (scrcmd_c.c:682-720, scrcmd_message.c) ----
        Opcode::OpenMsg => {
            c.host.dialog_open();
            c.env.set_window_open(true);
            c.env.set_textbox_open(true);
            Continue
        }
        Opcode::CloseMsg => {
            c.host.dialog_close();
            c.env.set_window_open(false);
            c.env.set_textbox_open(false);
            Continue
        }
        Opcode::HoldMsg => {
            c.action(FieldAction::DialogHold);
            c.env.set_window_open(false);
            c.env.set_textbox_open(false);
            Continue
        }
        Opcode::NPCMsg => {
            let id = c.u8()?;
            let units = c.message(u16::from(id))?;
            c.show(&units, true, 0)?;
            c.wait(WaitFor::PrintFinished)
        }
        Opcode::NPCMsgVar => {
            let id = c.var()?;
            let units = c.message(u16::from(id as u8))?;
            c.show(&units, false, 0)?;
            c.wait(WaitFor::PrintFinished)
        }
        Opcode::NonNPCMsgVar => {
            let id = c.var()?;
            let units = c.message(u16::from(id as u8))?;
            c.show(&units, true, 0)?;
            c.wait(WaitFor::PrintFinished)
        }
        Opcode::GenderMsgBox => {
            let male = c.u8()?;
            let female = c.u8()?;
            let id = if c.query(FieldQuery::PlayerGender) != 0 {
                female
            } else {
                male
            };
            let units = c.message(u16::from(id))?;
            c.show(&units, true, 0)?;
            c.wait(WaitFor::PrintFinished)
        }
        Opcode::MsgBoxExtern => {
            let bank = c.var()?;
            let id = c.var()?;
            let units = c.external_message(bank, id)?;
            c.show(&units, true, 0)?;
            c.wait(WaitFor::PrintFinished)
        }
        Opcode::GetStdMsgNaix => {
            let which = c.var()?;
            let ret = c.var_ref()?;
            let bank = STD_MSG_BANKS.get(usize::from(which)).copied().unwrap_or(0);
            c.write(ret, bank)?;
            Continue
        }
        Opcode::YesNo => {
            let var = c.u16()?;
            c.action(FieldAction::YesNoOpen);
            c.ctx.set_data(0, u32::from(var));
            c.native(NativeWait::YesNo)
        }

        // ---- signposts (scrcmd_c.c:784-945) ----
        Opcode::DirectionSignpost => {
            let id = c.u8()?;
            let kind = c.u8()?;
            let map = c.u16()?;
            let _unused_result_var = c.u16()?;
            c.env.set_textbox_open(true);
            c.action(FieldAction::SignpostSet { kind, map });
            c.action(FieldAction::SignpostCommand(MAPSIGNCOMMAND_SHOW));
            c.action(FieldAction::SignpostDoCurrent);
            let units = c.message(u16::from(id))?;
            let text = c.expand(&units)?;
            let params = PrintParams {
                font: 1,
                frame_delay: 0,
                can_ab_speed_up: false,
                flag: 0,
                instant: true,
                color: Some(SIGNPOST_COLOR),
            };
            c.host.print(PrintTarget::Signpost, &text, params);
            Yield
        }
        Opcode::SetSignpostMap => {
            let kind = c.u8()?;
            let map = c.u16()?;
            c.env.set_textbox_open(true);
            c.action(FieldAction::SignpostSet { kind, map });
            c.action(FieldAction::SignpostCommand(MAPSIGNCOMMAND_SHOW));
            Yield
        }
        Opcode::SetSignpostAction => {
            let command = c.u8()?;
            c.action(FieldAction::SignpostCommand(command));
            Yield
        }
        Opcode::WaitSignpostAction => {
            if c.host.poll(WaitFor::SignpostCommandFinished).is_some() {
                Continue
            } else {
                c.wait(WaitFor::SignpostCommandFinished)
            }
        }
        Opcode::TrainerTips => {
            let id = c.u8()?;
            let result_var = c.u16()?;
            let units = c.message(u16::from(id))?;
            let text = c.expand(&units)?;
            let params = PrintParams {
                font: 1,
                frame_delay: c.query(FieldQuery::TextFrameDelay),
                can_ab_speed_up: true,
                flag: 0,
                instant: false,
                color: Some(SIGNPOST_COLOR),
            };
            c.host.print(PrintTarget::Signpost, &text, params);
            c.ctx.set_data(0, u32::from(result_var));
            c.native(NativeWait::TrainerTips)
        }
        Opcode::WaitSignpost => {
            let result_var = c.u16()?;
            c.ctx.set_data(0, u32::from(result_var));
            c.native(NativeWait::WaitSignpost)
        }

        // ---- sound (scrcmd_sound.c) ----
        Opcode::PlayBGM => {
            let seq = c.u16()?;
            c.action(FieldAction::PlayBgm(seq));
            Continue
        }
        Opcode::StopBGM => {
            let _unused = c.u16()?;
            c.action(FieldAction::StopBgm);
            Continue
        }
        Opcode::ResetBGM => {
            c.action(FieldAction::ResetBgm);
            Continue
        }
        Opcode::FadeOutBGM => {
            let seq = c.u16()?;
            let length = c.u16()?;
            c.action(FieldAction::FadeOutBgm { seq, length });
            c.wait(WaitFor::BgmFadeFinished)
        }
        Opcode::FadeInBGM => {
            let length = c.u16()?;
            c.action(FieldAction::FadeInBgm(length));
            c.wait(WaitFor::BgmFadeFinished)
        }
        Opcode::TempBGM => {
            let seq = c.u16()?;
            c.action(FieldAction::TempBgm(seq));
            Continue
        }
        Opcode::PlaySE => {
            let seq = c.var()?;
            c.action(FieldAction::PlaySe(seq));
            Continue
        }
        Opcode::WaitSE => {
            let seq = c.var()?;
            c.ctx.set_data(0, u32::from(seq));
            c.wait(WaitFor::SeFinished(seq))
        }
        Opcode::PlayCry => {
            let form = c.var()?;
            let species = c.var()?;
            c.action(FieldAction::PlayCry { species, form });
            Continue
        }
        Opcode::WaitCry => c.wait(WaitFor::CryFinished),
        Opcode::PlayFanfare => {
            let seq = c.var()?;
            c.action(FieldAction::PlayFanfare(seq));
            Continue
        }
        Opcode::WaitFanfare => c.wait(WaitFor::FanfareFinished),

        // ---- objects and movement (scrcmd_c.c:1134-1560, 2985) ----
        Opcode::ApplyMovement => {
            let object = c.var()?;
            let word = c.u32()?;
            let target = c.ctx.relative(word);
            let steps = decode_movement(c.ctx.bank().bytes(), target)?;
            // A missing object is `GF_ASSERT(person == obj_partner_poke)`
            // then FALSE either way; the host decides what exists.
            c.host.apply_movement(object, &steps);
            Continue
        }
        Opcode::WaitMovement => c.wait(WaitFor::MovementFinished),
        Opcode::LockAll => {
            let last_interacted = c.env.last_interacted();
            let wait = c.action(FieldAction::LockAll { last_interacted }) != 0;
            if wait {
                c.ctx.set_native(NativeWait::Poll(WaitFor::LockSettled));
            }
            Yield
        }
        Opcode::ReleaseAll => {
            c.action(FieldAction::ReleaseAll);
            Yield
        }
        Opcode::Lock => {
            let object = c.u16()?;
            c.action(FieldAction::LockObject(object));
            Continue
        }
        Opcode::Release => {
            let object = c.u16()?;
            c.action(FieldAction::ReleaseObject(object));
            Continue
        }
        Opcode::ShowPerson => {
            let object = c.var()?;
            c.action(FieldAction::ShowObject(object));
            Continue
        }
        Opcode::HidePerson => {
            let object = c.var()?;
            c.action(FieldAction::HideObject(object));
            Continue
        }
        Opcode::FacePlayer => {
            if let Some(object) = c.env.last_interacted() {
                c.action(FieldAction::FacePlayer { object });
            }
            Continue
        }
        Opcode::GetPlayerCoords => {
            let x_var = c.var_ref()?;
            let z_var = c.var_ref()?;
            let (x, z) = c.host.object_position(None).unwrap_or((0, 0));
            c.write(x_var, x)?;
            c.write(z_var, z)?;
            Continue
        }
        Opcode::GetPersonCoords => {
            let object = c.var()?;
            let x_var = c.var_ref()?;
            let z_var = c.var_ref()?;
            let (x, z) = c.host.object_position(Some(object)).unwrap_or((255, 255));
            c.write(x_var, x)?;
            c.write(z_var, z)?;
            Continue
        }
        Opcode::GetPlayerFacing => {
            let ret = c.var_ref()?;
            let facing = c.query(FieldQuery::PlayerFacing) as u16;
            c.write(ret, facing)?;
            Continue
        }
        Opcode::MovePersonFacing => {
            let object = c.var()?;
            let x = c.var()?;
            let y = c.var()?;
            let z = c.var()?;
            let direction = c.var()?;
            c.action(FieldAction::SetObjectPosition {
                object,
                x,
                y,
                z,
                direction,
            });
            Continue
        }

        // ---- items (scrcmd_items.c) ----
        Opcode::GiveItem => {
            let item = c.var()?;
            let quantity = c.var()?;
            let ret = c.var_ref()?;
            let ok = c.action(FieldAction::BagAdd { item, quantity }) as u16;
            c.write(ret, ok)?;
            Continue
        }
        Opcode::TakeItem => {
            let item = c.var()?;
            let quantity = c.var()?;
            let ret = c.var_ref()?;
            let ok = c.action(FieldAction::BagTake { item, quantity }) as u16;
            c.write(ret, ok)?;
            Continue
        }
        Opcode::HasSpaceForItem => {
            let item = c.var()?;
            let quantity = c.var()?;
            let ret = c.var_ref()?;
            let ok = c.query(FieldQuery::BagHasSpace { item, quantity }) as u16;
            c.write(ret, ok)?;
            Continue
        }
        Opcode::HasItem => {
            let item = c.var()?;
            let quantity = c.var()?;
            let ret = c.var_ref()?;
            let ok = c.query(FieldQuery::BagHasItem { item, quantity }) as u16;
            c.write(ret, ok)?;
            Continue
        }
        Opcode::GetItemPocket => {
            let item = c.var()?;
            let ret = c.var_ref()?;
            let pocket = c.query(FieldQuery::ItemPocket(item)) as u16;
            c.write(ret, pocket)?;
            Continue
        }
        Opcode::SetStarterChoice => {
            let choice = c.var()?;
            c.write(VAR_PLAYER_STARTER, choice)?;
            Continue
        }

        // ---- the player, the party, money (scrcmd_c.c, scrcmd_party.c, scrcmd_money.c) ----
        Opcode::GetPlayerGender => {
            let ret = c.var_ref()?;
            let gender = c.query(FieldQuery::PlayerGender) as u16;
            c.write(ret, gender)?;
            Continue
        }
        Opcode::GetFriendSprite => {
            let ret = c.var_ref()?;
            let sprite = if c.query(FieldQuery::PlayerGender) != 0 {
                SPRITE_HERO
            } else {
                SPRITE_HEROINE
            };
            c.write(ret, sprite)?;
            Yield
        }
        Opcode::HealParty => {
            c.action(FieldAction::HealParty);
            Continue
        }
        Opcode::CheckBadge => {
            let badge = c.var()?;
            let ret = c.var_ref()?;
            // GF_ASSERT(badgeIdx < 16).
            let has = c.query(FieldQuery::HasBadge(badge)) as u16;
            c.write(ret, has)?;
            Continue
        }
        Opcode::GetPartyCount => {
            let ret = c.var_ref()?;
            let count = c.query(FieldQuery::PartyCount) as u16;
            c.write(ret, count)?;
            Continue
        }
        Opcode::GetPartyMonSpecies => {
            let slot = c.var()?;
            let ret = c.var_ref()?;
            let species = c
                .host
                .party_mon(slot)
                .filter(|mon| !mon.is_egg)
                .map_or(0, |mon| mon.species);
            c.write(ret, species)?;
            Continue
        }
        Opcode::MonGetFriendship => {
            let ret = c.var_ref()?;
            let slot = c.var()?;
            let friendship = c.host.party_mon(slot).map_or(0, |mon| u16::from(mon.friendship));
            c.write(ret, friendship)?;
            Continue
        }
        Opcode::GetPartyMonForm2 => {
            let slot = c.var()?;
            let ret = c.var_ref()?;
            let form = c.host.party_mon(slot).map_or(0, |mon| u16::from(mon.form));
            c.write(ret, form)?;
            Continue
        }
        Opcode::MonHasRibbon => {
            let ret = c.var_ref()?;
            let slot = c.var()?;
            let ribbon = c.var()?;
            let has = c.query(FieldQuery::MonHasRibbon { slot, ribbon }) as u16;
            c.write(ret, has)?;
            Continue
        }
        Opcode::GiveRibbon => {
            let slot = c.var()?;
            let ribbon = c.var()?;
            c.action(FieldAction::GiveRibbon { slot, ribbon });
            Continue
        }
        Opcode::GetPartyLeadAlive => {
            let ret = c.var_ref()?;
            let slot = c.query(FieldQuery::PartyLeadAlive) as u16;
            c.write(ret, slot)?;
            Continue
        }
        Opcode::HasEnoughMoneyVar => {
            let ret = c.var_ref()?;
            let amount = c.var()?;
            let enough = c.query(FieldQuery::Money) >= u32::from(amount);
            c.write(ret, u16::from(enough))?;
            Continue
        }
        Opcode::Cmd377 => {
            let ret = c.var_ref()?;
            let count = c.query(FieldQuery::MailboxCount) as u16;
            c.write(ret, count)?;
            Continue
        }
        Opcode::Cmd379 => {
            let ret = c.var_ref()?;
            let time = c.query(FieldQuery::TimeOfDay) as u16;
            c.write(ret, time)?;
            Continue
        }
        Opcode::GetWeekday => {
            let ret = c.var_ref()?;
            let week = c.query(FieldQuery::Weekday) as u16;
            c.write(ret, week)?;
            Continue
        }
        Opcode::GetGameVersion => {
            let ret = c.var_ref()?;
            c.write(ret, GAME_VERSION)?;
            Continue
        }
        Opcode::LotoIDSet => {
            // Save_VarsFlags_RollLotoId: two LCRandom draws, then the
            // retail Save_VarsFlags_SetLotoId writes *both* halves to
            // VAR_LOTO_NUMBER_LO (pret's BUGFIX_LOTO_NUMBER_HI is off in
            // the shipped image), so LO ends up holding the second draw.
            let lo = c.host.rng().next_u16();
            let hi = c.host.rng().next_u16();
            c.write(VAR_LOTO_NUMBER_LO, lo)?;
            c.write(VAR_LOTO_NUMBER_LO, hi)?;
            Continue
        }
        Opcode::PhotoAlbumIsFull => {
            let ret = c.var_ref()?;
            let full = c.query(FieldQuery::PhotoCount) >= 36;
            c.write(ret, u16::from(full))?;
            Continue
        }
        Opcode::CheckBankBalance => {
            let ret = c.var_ref()?;
            let amount = c.u32()?;
            let enough = c.query(FieldQuery::BankBalance) >= amount;
            c.write(ret, u16::from(enough))?;
            Continue
        }
        Opcode::BankOrWalletIsFull => {
            let which = c.u16()?;
            let ret = c.var_ref()?;
            let balance = if which == 0 {
                c.query(FieldQuery::BankBalance)
            } else {
                c.query(FieldQuery::Money)
            };
            c.write(ret, u16::from(balance == MAX_MONEY))?;
            Yield
        }
        Opcode::Cmd729 => {
            let ret = c.var_ref()?;
            let active = c.query(FieldQuery::FollowMonActive) as u16;
            c.write(ret, u16::from(active != 0))?;
            Continue
        }
        Opcode::Cmd596 => {
            let ret = c.var_ref()?;
            let state = c.query(FieldQuery::FollowMonState596) as u16;
            c.write(ret, state)?;
            Continue
        }
        Opcode::Cmd600 => {
            if c.query(FieldQuery::FollowMonState600) != 0 {
                Yield
            } else {
                Continue
            }
        }

        // ---- placeholder buffers (scrcmd_strbuf.c, message_format.c) ----
        Opcode::BufferPlayersName => {
            let field = c.u8()?;
            let name = c.host.player_name();
            c.env.msgfmt_mut().set_string(usize::from(field), &name);
            Continue
        }
        Opcode::BufferRivalsName => {
            let field = c.u8()?;
            let name = c.host.rival_name();
            c.env.msgfmt_mut().set_string(usize::from(field), &name);
            Continue
        }
        Opcode::BufferFriendsName => {
            let field = c.u8()?;
            // Male player → Lyra (message 1); female → Ethan (message 0).
            let id = u16::from(c.query(FieldQuery::PlayerGender) == 0);
            c.buffer_external(FRIEND_NAMES_BANK, id, field)?;
            Continue
        }
        Opcode::BufferMonSpeciesName => {
            let field = c.u8()?;
            let slot = c.var()?;
            let species = c.host.party_mon(slot).map_or(0, |mon| mon.species);
            c.buffer_external(SPECIES_NAMES_BANK, species, field)?;
            Continue
        }
        Opcode::BufferPartyMonSpeciesNameIndef => {
            let field = c.u8()?;
            let slot = c.var()?;
            let species = c.host.party_mon(slot).map_or(0, |mon| mon.species);
            c.buffer_external(SPECIES_NAMES_ARTICLE_BANK, species, field)?;
            Continue
        }
        Opcode::BufferPartyMonNick => {
            let field = c.u8()?;
            let slot = c.var()?;
            let nickname = c.host.party_mon(slot).map(|mon| mon.nickname).unwrap_or_default();
            c.env.msgfmt_mut().set_string(usize::from(field), &nickname);
            Continue
        }
        Opcode::BufferItemName => {
            let field = c.u8()?;
            let item = c.var()?;
            c.buffer_external(ITEM_NAMES_BANK, item, field)?;
            Continue
        }
        Opcode::BufferItemNamePlural => {
            let field = c.u8()?;
            let item = c.var()?;
            c.buffer_external(ITEM_NAMES_PLURAL_BANK, item, field)?;
            Continue
        }
        Opcode::BufferPocketName => {
            let field = c.u8()?;
            let pocket = c.var()?;
            c.buffer_external(POCKET_NAMES_BANK, pocket, field)?;
            Continue
        }

        // ---- applications ----
        Opcode::NameRival => {
            let ret = c.var_ref()?;
            c.host.launch(AppRequest::NamingScreen {
                kind: NAME_SCREEN_RIVAL,
                species: 0,
                max_len: PLAYER_NAME_LENGTH,
                party_slot: 0,
                initial: GameString::new(),
            });
            c.native(NativeWait::App {
                result_var: Some(ret),
            })
        }
        Opcode::NicknameInput => {
            let slot = c.var()?;
            if slot == 255 {
                // The Bug Contest's caught Pokémon — not in this port's
                // scope; the C yields when there is none to name.
                return Ok(Yield);
            }
            let mon = c.host.party_mon(slot);
            let ret = c.var_ref()?;
            let (species, initial) = mon
                .map(|mon| (mon.species, mon.nickname))
                .unwrap_or_default();
            c.host.launch(AppRequest::NamingScreen {
                kind: NAME_SCREEN_POKEMON,
                species,
                max_len: POKEMON_NAME_LENGTH,
                party_slot: slot,
                initial,
            });
            c.native(NativeWait::App {
                result_var: Some(ret),
            })
        }
        Opcode::ChooseStarter => {
            c.host.launch(AppRequest::ChooseStarter);
            c.native(NativeWait::App { result_var: None })
        }
        Opcode::CatchingTutorial => {
            c.host.launch(AppRequest::CatchingTutorial);
            c.native(NativeWait::App { result_var: None })
        }
        Opcode::Cmd376 => {
            c.host.launch(AppRequest::Mail);
            c.native(NativeWait::App { result_var: None })
        }
        // These push a child task (`TaskManager_Call`): the C returns
        // TRUE and the script task is simply not run until the child
        // returns, which the host reports as `ChildTask`.
        Opcode::RestoreOverworld => {
            c.action(FieldAction::RestoreOverworld);
            c.wait(WaitFor::ChildTask)
        }
        Opcode::Cmd436 => {
            c.action(FieldAction::LeaveOverworld);
            c.wait(WaitFor::ChildTask)
        }
        Opcode::CameronPhoto => {
            let photo = c.u16()?;
            c.action(FieldAction::TakePhoto(photo));
            c.wait(WaitFor::ChildTask)
        }

        // ---- fades and warps (scrcmd_c.c:2187-2260, 4097) ----
        Opcode::FadeScreen => {
            let duration = c.u16()?;
            let speed = c.u16()?;
            let kind = c.u16()?;
            let color = c.u16()?;
            c.action(FieldAction::FadeScreen {
                duration,
                speed,
                kind,
                color,
            });
            Continue
        }
        Opcode::WaitFade => c.wait(WaitFor::FadeFinished),
        Opcode::Warp => {
            let map = c.var()?;
            let _unused = c.u16()?;
            let x = c.var()?;
            let y = c.var()?;
            let direction = c.var()?;
            c.action(FieldAction::Warp {
                map,
                x,
                y,
                direction,
            });
            c.wait(WaitFor::ChildTask)
        }
        Opcode::Cmd582 => {
            let map = c.var()?;
            let x = c.var()?;
            let y = c.var()?;
            c.action(FieldAction::SetSpecialSpawn { map, x, y });
            Continue
        }

        // ---- the Pokégear, the phone ----
        Opcode::RegisterGearNumber => {
            let number = c.var()? as u8;
            if u16::from(number) < NUM_PHONE_CONTACTS {
                c.action(FieldAction::RegisterPhoneNumber(number));
            }
            Continue
        }
        Opcode::UnsetPhoneCallTrigger => {
            let flag = c.u8()?;
            c.action(FieldAction::ClearPhoneCallTrigger(flag));
            Continue
        }

        // ---- the following Pokémon (scrcmd_c.c:4422-4575) ----
        Opcode::ToggleFollowingPokemonMovement => {
            let mode = c.u16()?;
            if c.query(FieldQuery::FollowMonActive) != 0 {
                c.action(FieldAction::FollowMonPause(mode != 0));
            }
            Continue
        }
        Opcode::WaitFollowingPokemonMovement => {
            if c.query(FieldQuery::FollowMonActive) != 0 {
                c.ctx.set_native(NativeWait::Poll(WaitFor::FollowMonPaused));
            }
            Yield
        }
        Opcode::FollowingPokemonMovement => {
            let movement = c.u16()?;
            if c.query(FieldQuery::FollowMonActive) != 0 {
                c.action(FieldAction::FollowMonMovement(movement));
            }
            Yield
        }
        Opcode::Cmd605 => {
            let a = c.u8()?;
            let b = c.u8()?;
            if c.query(FieldQuery::FollowMonActive) != 0 {
                c.action(FieldAction::FollowMonEffect605 { a, b });
            }
            Continue
        }
        Opcode::Cmd608 => {
            if c.query(FieldQuery::FollowMonActive) != 0 {
                c.action(FieldAction::FollowMonEffect608);
            }
            Continue
        }
        Opcode::Cmd609 => {
            if c.query(FieldQuery::FollowMonActive) != 0 {
                c.action(FieldAction::FollowMonEffect609);
            }
            Yield
        }

        // ---- overlay-1 field effects (scrcmd_c.c:3045-3078) ----
        Opcode::Cmd307 => {
            let bx = c.u16()?;
            let by = c.u16()?;
            let x = c.var()?;
            let y = c.var()?;
            let kind = c.u8()?;
            c.action(FieldAction::Effect307 {
                x: u32::from(x) + 32 * u32::from(bx),
                y: u32::from(y) + 32 * u32::from(by),
                kind,
            });
            Continue
        }
        Opcode::Cmd308 => {
            let arg = c.u8()?;
            c.action(FieldAction::Effect308(arg));
            Yield
        }
        Opcode::Cmd309 => {
            let arg = c.u8()?;
            c.action(FieldAction::Effect309(arg));
            Continue
        }
        Opcode::Cmd310 => {
            let arg = c.u8()?;
            c.action(FieldAction::Effect310(arg));
            Continue
        }
        Opcode::Cmd311 => {
            let arg = c.u8()?;
            c.action(FieldAction::Effect311(arg));
            Continue
        }
        Opcode::PlaceStarterBallsInElmsLab => {
            let n = if ScriptEnvironment::flag_check(c.host, FLAG_GOT_TM51_FROM_FALKNER) {
                0
            } else if ScriptEnvironment::flag_check(c.host, FLAG_MET_PASSERBY_BOY) {
                1
            } else if c.query(FieldQuery::PartyCount) > 0 {
                2
            } else {
                3
            };
            for &(x, z) in &STARTER_BALL_COORDS[..n] {
                c.action(FieldAction::LoadMapProp {
                    prop: STARTER_BALL_PROP,
                    x,
                    z,
                });
            }
            Continue
        }

        // ---- menus, the money box, Mom's savings (scrcmd_c.c:4931-5140, headbutt.c) ----
        Opcode::TouchscreenMenuHide => {
            if c.query(FieldQuery::TouchscreenMenuMode) == 3 {
                Continue
            } else {
                c.action(FieldAction::TouchscreenMenu(3));
                c.wait(WaitFor::TouchscreenMenu(3))
            }
        }
        Opcode::TouchscreenMenuShow => {
            c.action(FieldAction::TouchscreenMenu(0));
            c.wait(WaitFor::TouchscreenMenu(0))
        }
        Opcode::GetMenuChoice => {
            let ret = c.u16()?;
            c.ctx.set_data(0, u32::from(ret));
            c.action(FieldAction::MenuChoiceBegin);
            c.native(NativeWait::MenuChoice)
        }
        Opcode::MenuInit => {
            // sub_02041770: the halfword names the variable the menu
            // gets a pointer to; `data[0]` is *not* written (the C's
            // MenuExec reads whatever an earlier command left there —
            // nothing, in the retail Mom script).
            let x = c.u8()?;
            let y = c.u8()?;
            let cursor = c.u8()?;
            let cancellable = c.u8()?;
            let ret = c.u16()?;
            c.action(FieldAction::MenuInit {
                x,
                y,
                cursor,
                cancellable,
                result_var: ret,
            });
            c.env.set_list_menu_var(Some(ret));
            Yield
        }
        Opcode::MenuItemAdd => {
            let id = c.var()?;
            let position = c.var()?;
            let value = c.var()?;
            let units = c.message(id)?;
            c.action(FieldAction::MenuAddItem {
                text: GameString::from_units(&units),
                position,
                value,
            });
            Continue
        }
        Opcode::MenuExec => {
            c.action(FieldAction::MenuExec);
            c.native(NativeWait::MenuExec)
        }
        Opcode::BankTransaction => {
            let mode = c.u16()?;
            let ret = c.u16()?;
            c.action(FieldAction::BankTransaction { mode });
            c.ctx.set_data(0, u32::from(ret));
            c.native(NativeWait::BankTransaction)
        }
        Opcode::Cmd795 => {
            let x = c.var()? as u8;
            let y = c.var()? as u8;
            c.action(FieldAction::MoneyBoxShow { x, y });
            Continue
        }
        Opcode::Cmd796 => {
            c.action(FieldAction::MoneyBoxHide);
            Continue
        }

        // ---- scene control ----
        Opcode::Cmd061 => {
            // sub_0204031C: arm scrctx_end_cb (outside the Mystery Zone).
            c.env.arm_end_callback();
            Continue
        }

        other => {
            return Err(ScriptError::Unimplemented {
                opcode: other,
                offset,
            });
        }
    };
    Ok(flow)
}

/// Runs the NATIVE-mode predicate once; `true` when it holds and the
/// context should return to bytecode next frame.
pub(crate) fn run_native(
    wait: &NativeWait,
    ctx: &mut ScriptContext,
    env: &mut ScriptEnvironment,
    host: &mut dyn ScriptHost,
) -> Result<bool, ScriptError> {
    let offset = ctx.pc().unwrap_or(0);
    let mut c = Cmd {
        ctx,
        env,
        host,
        offset,
    };
    let done = match wait {
        NativeWait::PauseTimer => {
            let var = c.ctx.data(0) as u16;
            let left = c.read(var)?.wrapping_sub(1);
            c.write(var, left)?;
            left == 0
        }
        NativeWait::WaitStd => c.env.std_wait_mask() & (1 << c.ctx.id) == 0,
        NativeWait::WaitButton => {
            if c.host.new_keys().any(AB) {
                true
            } else if let Some(direction) = c.pressed_direction() {
                c.action(FieldAction::SetPlayerFacing(direction));
                true
            } else {
                false
            }
        }
        NativeWait::WaitButtonOrDpad => c.host.new_keys().any(AB | DPAD),
        NativeWait::WaitButtonOrDelay => {
            if c.host.new_keys().any(AB) {
                true
            } else {
                let left = c.ctx.data(0).wrapping_sub(1);
                c.ctx.set_data(0, left);
                left == 0
            }
        }
        NativeWait::WaitAbPress => c.host.new_keys().any(AB),
        NativeWait::YesNo => match c.host.poll(WaitFor::YesNo) {
            Some(selection) => {
                let var = c.ctx.data(0) as u16;
                c.write(var, u16::from(selection != 0))?;
                true
            }
            None => false,
        },
        NativeWait::TrainerTips => {
            let var = c.ctx.data(0) as u16;
            if c.host.poll(WaitFor::PrintFinished).is_some() {
                c.write(var, 2)?;
                true
            } else if let Some(direction) = c.pressed_direction() {
                c.action(FieldAction::RemoveTextPrinter);
                c.action(FieldAction::SetPlayerFacing(direction));
                c.write(var, 0)?;
                c.env.set_textbox_open(false);
                true
            } else {
                false
            }
        }
        NativeWait::WaitSignpost => {
            let var = c.ctx.data(0) as u16;
            if c.host.new_keys().any(AB) {
                c.write(var, 0)?;
                c.env.set_textbox_open(false);
                true
            } else if let Some(direction) = c.pressed_direction() {
                c.action(FieldAction::SetPlayerFacing(direction));
                c.write(var, 0)?;
                c.env.set_textbox_open(false);
                true
            } else {
                false
            }
        }
        NativeWait::App { result_var } => match c.host.poll(WaitFor::App) {
            Some(result) => {
                if let Some(var) = result_var {
                    c.write(*var, result)?;
                }
                true
            }
            None => false,
        },
        NativeWait::MenuChoice => match c.host.poll(WaitFor::MenuChoice) {
            Some(choice) => {
                c.ctx.set_data(1, u32::from(choice));
                let var = c.ctx.data(0) as u16;
                c.write(var, choice)?;
                true
            }
            None => false,
        },
        NativeWait::MenuExec => match c.host.poll(WaitFor::MenuExec) {
            Some(result) => {
                // The choice lands in the variable MenuInit handed the
                // menu; the C also hands `GetVarPointer(data[0])` to the
                // touch-menu task, so a resolvable `data[0]` gets it too.
                if let Some(var) = c.env.list_menu_var() {
                    c.write(var, result)?;
                }
                let stale = c.ctx.data(0) as u16;
                if is_saved_var(stale) || is_special_var(stale) {
                    c.write(stale, result)?;
                }
                c.env.set_list_menu_var(None);
                true
            }
            None => false,
        },
        NativeWait::BankTransaction => match c.host.poll(WaitFor::BankTransaction) {
            Some(result) => {
                let var = c.ctx.data(0) as u16;
                c.write(var, result)?;
                true
            }
            None => false,
        },
        NativeWait::Poll(wait) => c.host.poll(*wait).is_some(),
    };
    Ok(done)
}

/// The opcodes [`execute`] implements, ascending — for the inventory
/// tests and the docs.
#[must_use]
pub fn implemented_opcodes() -> Vec<Opcode> {
    let mut out = Vec::new();
    for op in 0..super::commands::OPCODE_COUNT as u16 {
        let opcode = Opcode::from_u16(op).expect("in range");
        if is_implemented(opcode) {
            out.push(opcode);
        }
    }
    out
}

/// Whether [`execute`] has a handler for `opcode`.
#[must_use]
pub fn is_implemented(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Nop
            | Opcode::Dummy
            | Opcode::Dummy486
            | Opcode::End
            | Opcode::Wait
            | Opcode::LoadByte
            | Opcode::LoadWord
            | Opcode::CopyLocal
            | Opcode::CompareLocalToLocal
            | Opcode::CompareLocalToValue
            | Opcode::CompareVarToValue
            | Opcode::CompareVarToVar
            | Opcode::CallStd
            | Opcode::RestartCurrentScript
            | Opcode::GoTo
            | Opcode::ObjectGoTo
            | Opcode::DirectionGoTo
            | Opcode::Call
            | Opcode::Return
            | Opcode::GoToIf
            | Opcode::CallIf
            | Opcode::SetFlag
            | Opcode::ClearFlag
            | Opcode::CheckFlag
            | Opcode::CheckFlagVar
            | Opcode::SetFlagVar
            | Opcode::ClearFlagVar
            | Opcode::SetTrainerFlag
            | Opcode::ClearTrainerFlag
            | Opcode::CheckTrainerFlag
            | Opcode::AddVar
            | Opcode::SubVar
            | Opcode::SetVar
            | Opcode::CopyVar
            | Opcode::SetOrCopyVar
            | Opcode::Random
            | Opcode::WaitABPress
            | Opcode::WaitButtonOrDelay
            | Opcode::WaitButton
            | Opcode::WaitButtonOrDpad
            | Opcode::OpenMsg
            | Opcode::CloseMsg
            | Opcode::HoldMsg
            | Opcode::NPCMsg
            | Opcode::NPCMsgVar
            | Opcode::NonNPCMsgVar
            | Opcode::GenderMsgBox
            | Opcode::MsgBoxExtern
            | Opcode::GetStdMsgNaix
            | Opcode::YesNo
            | Opcode::DirectionSignpost
            | Opcode::SetSignpostMap
            | Opcode::SetSignpostAction
            | Opcode::WaitSignpostAction
            | Opcode::TrainerTips
            | Opcode::WaitSignpost
            | Opcode::PlayBGM
            | Opcode::StopBGM
            | Opcode::ResetBGM
            | Opcode::FadeOutBGM
            | Opcode::FadeInBGM
            | Opcode::TempBGM
            | Opcode::PlaySE
            | Opcode::WaitSE
            | Opcode::PlayCry
            | Opcode::WaitCry
            | Opcode::PlayFanfare
            | Opcode::WaitFanfare
            | Opcode::ApplyMovement
            | Opcode::WaitMovement
            | Opcode::LockAll
            | Opcode::ReleaseAll
            | Opcode::Lock
            | Opcode::Release
            | Opcode::ShowPerson
            | Opcode::HidePerson
            | Opcode::FacePlayer
            | Opcode::GetPlayerCoords
            | Opcode::GetPersonCoords
            | Opcode::GetPlayerFacing
            | Opcode::MovePersonFacing
            | Opcode::GiveItem
            | Opcode::TakeItem
            | Opcode::HasSpaceForItem
            | Opcode::HasItem
            | Opcode::GetItemPocket
            | Opcode::SetStarterChoice
            | Opcode::GetPlayerGender
            | Opcode::GetFriendSprite
            | Opcode::HealParty
            | Opcode::CheckBadge
            | Opcode::GetPartyCount
            | Opcode::GetPartyMonSpecies
            | Opcode::MonGetFriendship
            | Opcode::GetPartyMonForm2
            | Opcode::MonHasRibbon
            | Opcode::GiveRibbon
            | Opcode::GetPartyLeadAlive
            | Opcode::HasEnoughMoneyVar
            | Opcode::Cmd377
            | Opcode::Cmd379
            | Opcode::GetWeekday
            | Opcode::GetGameVersion
            | Opcode::LotoIDSet
            | Opcode::PhotoAlbumIsFull
            | Opcode::CheckBankBalance
            | Opcode::BankOrWalletIsFull
            | Opcode::Cmd729
            | Opcode::Cmd596
            | Opcode::Cmd600
            | Opcode::BufferPlayersName
            | Opcode::BufferRivalsName
            | Opcode::BufferFriendsName
            | Opcode::BufferMonSpeciesName
            | Opcode::BufferPartyMonSpeciesNameIndef
            | Opcode::BufferPartyMonNick
            | Opcode::BufferItemName
            | Opcode::BufferItemNamePlural
            | Opcode::BufferPocketName
            | Opcode::NameRival
            | Opcode::NicknameInput
            | Opcode::ChooseStarter
            | Opcode::CatchingTutorial
            | Opcode::Cmd376
            | Opcode::RestoreOverworld
            | Opcode::Cmd436
            | Opcode::CameronPhoto
            | Opcode::FadeScreen
            | Opcode::WaitFade
            | Opcode::Warp
            | Opcode::Cmd582
            | Opcode::RegisterGearNumber
            | Opcode::UnsetPhoneCallTrigger
            | Opcode::ToggleFollowingPokemonMovement
            | Opcode::WaitFollowingPokemonMovement
            | Opcode::FollowingPokemonMovement
            | Opcode::Cmd605
            | Opcode::Cmd608
            | Opcode::Cmd609
            | Opcode::Cmd307
            | Opcode::Cmd308
            | Opcode::Cmd309
            | Opcode::Cmd310
            | Opcode::Cmd311
            | Opcode::PlaceStarterBallsInElmsLab
            | Opcode::TouchscreenMenuHide
            | Opcode::TouchscreenMenuShow
            | Opcode::GetMenuChoice
            | Opcode::MenuInit
            | Opcode::MenuItemAdd
            | Opcode::MenuExec
            | Opcode::BankTransaction
            | Opcode::Cmd795
            | Opcode::Cmd796
            | Opcode::Cmd061
    )
}
