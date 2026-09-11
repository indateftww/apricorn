//! `ScriptEnvironment` — the state one script *task* owns (pret
//! `include/script.h:46`, `src/script_manager.c`): up to three
//! contexts (a scene script and the std scripts it `CallStd`s), the
//! fourteen special variables `0x8000..=0x800D`, the message-format
//! placeholders and the two 1024-unit string buffers, the object the
//! player interacted with, and the driver `Task_RunScripts`.
//!
//! `SetupScriptEngine` (`:161`) → [`ScriptEnvironment::setup`];
//! `Task_RunScripts` (`:97`) → [`ScriptEnvironment::run_frame`];
//! `CreateScriptContext` + `SetUpScriptContextForMap` (`:175-192`) →
//! [`ScriptEnvironment::create_context`]; `GetVarPointer` /
//! `FieldSystem_VarGet` / `FieldSystem_VarSet` (`:354-380`) →
//! [`ScriptEnvironment::var_get`] / [`ScriptEnvironment::var_set`];
//! `StartMapLoadScript` (`:580`) → [`ScriptEnvironment::run_map_load_script`].

use crate::save::vars_flags::{
    NUM_SPECIAL_VARS, SPECIAL_VAR_BASE, VAR_BASE, VAR_SPECIAL_LAST_TALKED, is_saved_var,
    is_special_var, is_temp_flag,
};
use crate::text::format::MessageFormat;
use crate::text::string::GameString;

use super::ScriptError;
use super::bank::{ScriptBank, resolve_script};
use super::context::ScriptContext;
use super::host::ScriptHost;

/// `NELEMS(env->scriptContexts)`.
pub const NUM_CONTEXTS: usize = 3;

/// What [`ScriptEnvironment::run_frame`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStatus {
    /// At least one context is still alive.
    Running,
    /// Every context has stopped; the task is over (`Task_RunScripts`
    /// returns `TRUE`, or runs `scrctx_end_cb` first when
    /// `callback` — `ScrCmd_061` had armed it).
    Finished {
        /// Whether the end callback (`sub_0203BD64`) was armed.
        callback: bool,
    },
}

/// One script task's environment.
#[derive(Debug)]
pub struct ScriptEnvironment {
    contexts: [Option<ScriptContext>; NUM_CONTEXTS],
    /// `activeScriptContextCount`.
    active_count: u8,
    /// `activeScriptNumber` — the script this task was started for.
    active_script: u16,
    /// `lastInteracted` — the object id the player talked to.
    last_interacted: Option<u16>,
    /// `facingDirection` — the player's facing when the task started.
    facing_direction: u16,
    /// `unk_7` — the `CallStd` wait mask: bit `id` is set while context
    /// `id` waits for the std script it called.
    std_wait_mask: u8,
    /// `fieldSystem->textbox_open` — a text box (dialogue or signpost)
    /// is showing; the field's input handling looks at it.
    textbox_open: bool,
    /// `unk_8` — the dialogue *window* exists (`OpenMsg`/`NPCMsg`
    /// create it, `CloseMsg`/`HoldMsg` remove it); signposts never
    /// touch it.
    window_open: bool,
    /// `msgfmt` — `MessageFormat_New_Custom(8, 64)`.
    msgfmt: MessageFormat,
    /// `stringBuffer0` — the expanded text of the last message.
    string_buffer0: GameString,
    /// `stringBuffer1` — the raw text of the last message.
    string_buffer1: GameString,
    /// `specialVars[14]`.
    special_vars: [u16; NUM_SPECIAL_VARS],
    /// `scrctx_end_cb` armed.
    end_callback: bool,
    /// `state` — 0 before the first frame creates context 0.
    started: bool,
}

impl Default for ScriptEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptEnvironment {
    /// `ScriptEnvironment_New` — zeroed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            contexts: [None, None, None],
            active_count: 0,
            active_script: 0,
            last_interacted: None,
            facing_direction: 0,
            std_wait_mask: 0,
            textbox_open: false,
            window_open: false,
            msgfmt: MessageFormat::new(8),
            string_buffer0: GameString::new(),
            string_buffer1: GameString::new(),
            special_vars: [0; NUM_SPECIAL_VARS],
            end_callback: false,
            started: false,
        }
    }

    /// `SetupScriptEngine` (`src/script_manager.c:161`): records the
    /// script to run, the player's facing and the object interacted
    /// with (whose id also lands in `VAR_SPECIAL_LAST_TALKED`).
    ///
    /// The hidden-item parameter fill for ids `8000..8800`
    /// (`GetHiddenItemParams`, from `sHiddenItemParam`) is not ported:
    /// those scripts are outside the early-game subset.
    pub fn setup(&mut self, script: u16, last_interacted: Option<u16>, facing_direction: u16) {
        self.facing_direction = facing_direction;
        self.last_interacted = last_interacted;
        self.active_script = script;
        if let Some(id) = last_interacted {
            self.special_vars[usize::from(VAR_SPECIAL_LAST_TALKED - SPECIAL_VAR_BASE)] = id;
        }
    }

    /// The script this task runs (`activeScriptNumber`).
    #[must_use]
    pub fn active_script(&self) -> u16 {
        self.active_script
    }

    /// How many contexts are alive.
    #[must_use]
    pub fn active_count(&self) -> u8 {
        self.active_count
    }

    /// The context in slot `slot`, if alive.
    #[must_use]
    pub fn context(&self, slot: usize) -> Option<&ScriptContext> {
        self.contexts.get(slot).and_then(Option::as_ref)
    }

    /// The object the player interacted with.
    #[must_use]
    pub fn last_interacted(&self) -> Option<u16> {
        self.last_interacted
    }

    /// The player's facing when the task started.
    #[must_use]
    pub fn facing_direction(&self) -> u16 {
        self.facing_direction
    }

    /// Whether a text box is showing (`fieldSystem->textbox_open`).
    #[must_use]
    pub fn textbox_open(&self) -> bool {
        self.textbox_open
    }

    /// Whether the dialogue window exists (`unk_8`).
    #[must_use]
    pub fn window_open(&self) -> bool {
        self.window_open
    }

    /// The message-format placeholders (`msgfmt`).
    #[must_use]
    pub fn message_format(&self) -> &MessageFormat {
        &self.msgfmt
    }

    /// The last expanded message (`stringBuffer0`).
    #[must_use]
    pub fn last_message(&self) -> &GameString {
        &self.string_buffer0
    }

    /// Special variable `0x8000 + index`.
    #[must_use]
    pub fn special_var(&self, index: usize) -> Option<u16> {
        self.special_vars.get(index).copied()
    }

    /// Whether the end callback is armed.
    #[must_use]
    pub fn end_callback_armed(&self) -> bool {
        self.end_callback
    }

    /// `FieldSystem_VarGet`: a literal below `VAR_BASE` reads as
    /// itself, a saved variable from the host's block, a special
    /// variable from here; `None` for the ranges the original asserts on.
    pub fn var_get(&self, host: &mut dyn ScriptHost, var: u16) -> Option<u16> {
        if var < VAR_BASE {
            Some(var)
        } else if is_saved_var(var) {
            host.vars_flags().var(var)
        } else if is_special_var(var) {
            Some(self.special_vars[usize::from(var - SPECIAL_VAR_BASE)])
        } else {
            None
        }
    }

    /// `FieldSystem_VarSet`: `false` where `GetVarPointer` is `NULL`.
    pub fn var_set(&mut self, host: &mut dyn ScriptHost, var: u16, value: u16) -> bool {
        if is_saved_var(var) {
            host.vars_flags().set_var(var, value)
        } else if is_special_var(var) {
            self.special_vars[usize::from(var - SPECIAL_VAR_BASE)] = value;
            true
        } else {
            false
        }
    }

    /// `FieldSystem_FlagCheck` — saved or temporary by range.
    pub fn flag_check(host: &mut dyn ScriptHost, flag: u16) -> bool {
        if is_temp_flag(flag) {
            host.temp_flags().flag(flag)
        } else {
            host.vars_flags().flag(flag)
        }
    }

    /// `FieldSystem_FlagSet`.
    pub fn flag_set(host: &mut dyn ScriptHost, flag: u16) {
        if is_temp_flag(flag) {
            host.temp_flags().set_flag(flag);
        } else {
            host.vars_flags().set_flag(flag);
        }
    }

    /// `FieldSystem_FlagClear`.
    pub fn flag_clear(host: &mut dyn ScriptHost, flag: u16) {
        if is_temp_flag(flag) {
            host.temp_flags().clear_flag(flag);
        } else {
            host.vars_flags().clear_flag(flag);
        }
    }

    /// `CreateScriptContext` + `SetUpScriptContextForMap`: resolves
    /// `script` to its banks ([`resolve_script`]), loads them through
    /// the host, and positions the context at the script's entry.
    ///
    /// # Errors
    /// [`ScriptError::BankMissing`] when the host has no such script
    /// bank, the bank parse errors, [`ScriptError::NoSuchScript`] past
    /// its table. A missing message bank is not an error until a
    /// message is read (the original loads messages lazily).
    pub fn create_context(
        &self,
        host: &mut dyn ScriptHost,
        script: u16,
    ) -> Result<ScriptContext, ScriptError> {
        let resolved = resolve_script(script, host.current_map_banks());
        let bytes = host
            .load_scripts(resolved.script_bank)
            .ok_or(ScriptError::BankMissing {
                bank: resolved.script_bank,
            })?;
        let bank = ScriptBank::parse(bytes)?;
        let messages = host
            .load_messages(resolved.msg_bank)
            .and_then(|bytes| ScriptContext::decode_messages(&bytes));
        let mut ctx = ScriptContext::new(bank, messages, resolved.script_bank, resolved.msg_bank);
        ctx.setup_bytecode(0);
        ctx.run_by_index(resolved.index)?;
        Ok(ctx)
    }

    /// `Task_RunScripts` (`src/script_manager.c:97`): the first call
    /// creates context 0 for the script [`Self::setup`] named; every
    /// call then steps each live context once, in slot order (a
    /// context a `CallStd` creates in a later slot runs in the same
    /// frame), destroying the ones that stopped.
    ///
    /// # Errors
    /// Propagates the first context error; that context is stopped and
    /// destroyed, the others keep their state for inspection.
    pub fn run_frame(&mut self, host: &mut dyn ScriptHost) -> Result<FrameStatus, ScriptError> {
        if !self.started {
            let ctx = self.create_context(host, self.active_script)?;
            self.contexts[0] = Some(ctx);
            self.active_count = 1;
            self.msgfmt = MessageFormat::new(8);
            self.started = true;
        }
        for slot in 0..NUM_CONTEXTS {
            let Some(mut ctx) = self.contexts[slot].take() else {
                continue;
            };
            match ctx.step(self, host) {
                Ok(true) => self.contexts[slot] = Some(ctx),
                Ok(false) => {
                    // DestroyScriptContext; GF_ASSERT(count != 0).
                    self.active_count = self.active_count.saturating_sub(1);
                }
                Err(err) => {
                    self.active_count = self.active_count.saturating_sub(1);
                    return Err(err);
                }
            }
        }
        if self.active_count == 0 {
            Ok(FrameStatus::Finished {
                callback: self.end_callback,
            })
        } else {
            Ok(FrameStatus::Running)
        }
    }

    /// `StartMapLoadScript` (`src/script_manager.c:580`): runs `script`
    /// synchronously — `while (RunScriptCommand(ctx) == TRUE) {}` — in
    /// a fresh context 0, no frames in between (a native wait spins
    /// within the call, so init scripts never wait). Returns how many
    /// `RunScriptCommand` calls it took.
    ///
    /// # Errors
    /// As [`Self::create_context`] and the commands; [`ScriptError::Runaway`]
    /// past `max_steps` calls.
    pub fn run_map_load_script(
        &mut self,
        host: &mut dyn ScriptHost,
        script: u16,
        max_steps: usize,
    ) -> Result<usize, ScriptError> {
        self.active_script = script;
        let mut ctx = self.create_context(host, script)?;
        self.active_count = 1;
        self.started = true;
        let mut steps = 0;
        loop {
            steps += 1;
            match ctx.step(self, host) {
                Ok(true) => {}
                Ok(false) => break,
                Err(err) => {
                    self.active_count = 0;
                    return Err(err);
                }
            }
            if steps >= max_steps {
                self.active_count = 0;
                return Err(ScriptError::Runaway { steps });
            }
        }
        self.active_count = 0;
        Ok(steps)
    }

    // ---- the executor's view of the environment ----

    pub(crate) fn msgfmt_mut(&mut self) -> &mut MessageFormat {
        &mut self.msgfmt
    }

    pub(crate) fn set_buffers(&mut self, raw: GameString, expanded: GameString) {
        self.string_buffer1 = raw;
        self.string_buffer0 = expanded;
    }

    pub(crate) fn set_textbox_open(&mut self, open: bool) {
        self.textbox_open = open;
    }

    pub(crate) fn set_window_open(&mut self, open: bool) {
        self.window_open = open;
    }

    pub(crate) fn std_wait_mask(&self) -> u8 {
        self.std_wait_mask
    }

    pub(crate) fn set_std_wait_mask(&mut self, mask: u8) {
        self.std_wait_mask = mask;
    }

    pub(crate) fn arm_end_callback(&mut self) {
        self.end_callback = true;
    }

    /// `CallStd`'s context creation: the new context takes slot
    /// `activeScriptContextCount` and that id.
    pub(crate) fn spawn_std_context(
        &mut self,
        host: &mut dyn ScriptHost,
        script: u16,
    ) -> Result<u8, ScriptError> {
        let slot = usize::from(self.active_count);
        if slot >= NUM_CONTEXTS || self.contexts[slot].is_some() {
            return Err(ScriptError::TooManyContexts);
        }
        let mut ctx = self.create_context(host, script)?;
        ctx.id = self.active_count;
        self.contexts[slot] = Some(ctx);
        self.active_count += 1;
        Ok(slot as u8)
    }
}
