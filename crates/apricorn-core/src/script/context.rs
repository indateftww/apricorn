//! `ScriptContext` — one running script (pret `include/script.h:164`
//! and `src/script.c`).
//!
//! ```text
//! struct ScriptContext {
//!     u8 stackDepth; u8 mode; u8 comparisonResult; u8 id;
//!     ScrCmdFunc native_ptr;       // the NATIVE-mode wait predicate
//!     const u8 *script_ptr;        // program counter (NULL = stopped)
//!     const u8 *stack[20];         // Call/Return
//!     const ScrCmdFunc *cmdTable; u32 cmd_count;
//!     u32 data[4];                 // scratch registers
//!     TaskManager *taskman; MsgData *msgdata; u8 *mapScripts; FieldSystem *fieldSystem;
//! };
//! ```
//!
//! Here the program counter and stack are offsets into the owned
//! [`ScriptBank`], the native pointer is a [`NativeWait`] the executor
//! interprets, and the message bank is decoded once into units.

use crate::formats::MsgBank;

use super::ScriptError;
use super::bank::ScriptBank;
use super::commands::Opcode;
use super::env::ScriptEnvironment;
use super::exec::{self, Flow};
use super::host::{ScriptHost, WaitFor};

/// `NELEMS(ctx->stack)` — the call-stack depth.
pub const STACK_DEPTH: usize = 20;
/// `NELEMS(ctx->data)` — the scratch registers.
pub const NUM_REGISTERS: usize = 4;

/// `SCRIPT_MODE_*` (`include/script.h:22-24`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// `SCRIPT_MODE_STOPPED` — not running; `step` returns `false`.
    Stopped,
    /// `SCRIPT_MODE_BYTECODE` — executing commands until one yields.
    Bytecode,
    /// `SCRIPT_MODE_NATIVE` — polling a wait predicate once per frame.
    Native,
}

/// `SCRIPT_COMPARISON_RESULT_*` (`include/script.h:26-28`) — the
/// value `Compare*` commands leave in `comparisonResult`, which
/// `GoToIf`/`CallIf` index the condition table with.
pub mod comparison {
    /// `a < b`.
    pub const LESS: u8 = 0;
    /// `a == b`.
    pub const EQUAL: u8 = 1;
    /// `a > b`.
    pub const GREATER: u8 = 2;
}

/// `Compare` (`src/scrcmd_c.c:256`).
#[must_use]
pub(crate) fn compare(a: u32, b: u32) -> u8 {
    if a < b {
        comparison::LESS
    } else if a == b {
        comparison::EQUAL
    } else {
        comparison::GREATER
    }
}

/// `sConditionTable[6][3]` (`src/scrcmd_c.c:143`): rows are the
/// `GoToIf` conditions `lt eq gt le ge ne` (`include/constants/scrcmd.h`),
/// columns the comparison result `< = >`.
pub(crate) const CONDITION_TABLE: [[u8; 3]; 6] = [
    [1, 0, 0], // lt
    [0, 1, 0], // eq
    [0, 0, 1], // gt
    [1, 1, 0], // le
    [0, 1, 1], // ge
    [1, 0, 1], // ne
];

/// The NATIVE-mode wait a command installed (`SetupNativeScript`): the
/// predicate `RunScriptCommand` polls once per frame until it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeWait {
    /// `RunPauseTimer` — decrement the variable in `data[0]` until zero.
    PauseTimer,
    /// `ScrNative_WaitStd` — until the std context this one called
    /// clears our bit of the environment's wait mask.
    WaitStd,
    /// `sub_02041074` — A/B, or a d-pad press (which also turns the
    /// player).
    WaitButton,
    /// `sub_020410F0` — A/B or any d-pad press.
    WaitButtonOrDpad,
    /// `sub_02041040` — A/B, or `data[0]` frames elapsed.
    WaitButtonOrDelay,
    /// `sub_02041000` — A/B.
    WaitAbPress,
    /// `sub_020416E4` — the yes/no menu's choice into the variable in
    /// `data[0]` (0 yes, 1 no).
    YesNo,
    /// `NativeScript_WaitTrainerTips` — the signpost print finishing
    /// (result 2) or a d-pad press cancelling it (result 0).
    TrainerTips,
    /// `NativeScript_WaitSignpost` — A/B or a d-pad press dismissing a
    /// signpost (result 0).
    WaitSignpost,
    /// An application launched by [`ScriptHost::launch`]; its result
    /// goes to `result_var` when one was named.
    App {
        /// The variable to receive the application's result.
        result_var: Option<u16>,
    },
    /// `sub_020477C0` — the lower-screen menu's choice into `data[1]`
    /// and the variable in `data[0]`.
    MenuChoice,
    /// `sub_020478D0` — the scripted list menu's result into the
    /// variable `MenuInit` named (and `data[0]`'s, when that resolves).
    MenuExec,
    /// `sub_020479D4` — the bank-transaction result into the variable
    /// in `data[0]`.
    BankTransaction,
    /// A plain host predicate.
    Poll(WaitFor),
}

/// One running script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptContext {
    /// Which environment slot this context runs in (`ctx->id`) —
    /// context 0 for a scene script, `activeScriptContextCount` at
    /// creation for a `CallStd` callee.
    pub id: u8,
    mode: Mode,
    comparison: u8,
    stack: [usize; STACK_DEPTH],
    stack_depth: u8,
    pc: Option<usize>,
    data: [u32; NUM_REGISTERS],
    native: Option<NativeWait>,
    bank: ScriptBank,
    messages: Option<Vec<Vec<u16>>>,
    script_bank: u16,
    msg_bank: u16,
}

impl ScriptContext {
    /// `InitScriptContext`: stopped, registers and stack cleared, over
    /// `bank` (loaded from `a/0/1/2` member `script_bank`) and the
    /// decoded messages of `a/0/2/7` member `msg_bank` (lazily loaded
    /// in the original; `None` here means the host had none, which
    /// only matters when a message is read).
    #[must_use]
    pub fn new(
        bank: ScriptBank,
        messages: Option<Vec<Vec<u16>>>,
        script_bank: u16,
        msg_bank: u16,
    ) -> Self {
        Self {
            id: 0,
            mode: Mode::Stopped,
            comparison: 0,
            stack: [0; STACK_DEPTH],
            stack_depth: 0,
            pc: None,
            data: [0; NUM_REGISTERS],
            native: None,
            bank,
            messages,
            script_bank,
            msg_bank,
        }
    }

    /// Decodes a raw `a/0/2/7` member into message units for [`Self::new`].
    #[must_use]
    pub fn decode_messages(bytes: &[u8]) -> Option<Vec<Vec<u16>>> {
        let bank = MsgBank::parse(bytes).ok()?;
        Some(bank.messages().map(<[u16]>::to_vec).collect())
    }

    /// `SetupBytecodeScript` — start executing at `offset`.
    pub fn setup_bytecode(&mut self, offset: usize) {
        self.pc = Some(offset);
        self.mode = Mode::Bytecode;
    }

    /// `ScriptRunByIndex` — jump to entry `index` of the bank.
    ///
    /// # Errors
    /// [`ScriptError::NoSuchScript`] past the entry table.
    pub fn run_by_index(&mut self, index: u16) -> Result<(), ScriptError> {
        let offset = self
            .bank
            .script_offset(usize::from(index))
            .ok_or(ScriptError::NoSuchScript {
                index,
                count: self.bank.script_count(),
            })?;
        self.setup_bytecode(offset);
        Ok(())
    }

    /// `StopScript`.
    pub fn stop(&mut self) {
        self.mode = Mode::Stopped;
        self.pc = None;
    }

    /// The current mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The program counter — the next byte to read — while running.
    #[must_use]
    pub fn pc(&self) -> Option<usize> {
        self.pc
    }

    /// `comparisonResult`.
    #[must_use]
    pub fn comparison(&self) -> u8 {
        self.comparison
    }

    /// The scratch registers `data[4]`.
    #[must_use]
    pub fn registers(&self) -> &[u32; NUM_REGISTERS] {
        &self.data
    }

    /// The call stack, innermost last.
    #[must_use]
    pub fn stack(&self) -> &[usize] {
        &self.stack[..usize::from(self.stack_depth)]
    }

    /// The installed native wait, in NATIVE mode.
    #[must_use]
    pub fn native(&self) -> Option<&NativeWait> {
        self.native.as_ref()
    }

    /// The bank this context runs.
    #[must_use]
    pub fn bank(&self) -> &ScriptBank {
        &self.bank
    }

    /// The `a/0/1/2` member id of the bank.
    #[must_use]
    pub fn script_bank(&self) -> u16 {
        self.script_bank
    }

    /// The `a/0/2/7` member id of the messages.
    #[must_use]
    pub fn msg_bank(&self) -> u16 {
        self.msg_bank
    }

    /// Message `id` of the context's bank (`ctx->msgdata`).
    ///
    /// # Errors
    /// [`ScriptError::MessagesMissing`] when the host supplied no bank,
    /// [`ScriptError::NoSuchMessage`] past its end.
    pub fn message(&self, id: u16) -> Result<&[u16], ScriptError> {
        let messages = self
            .messages
            .as_ref()
            .ok_or(ScriptError::MessagesMissing {
                bank: self.msg_bank,
            })?;
        messages
            .get(usize::from(id))
            .map(Vec::as_slice)
            .ok_or(ScriptError::NoSuchMessage {
                bank: self.msg_bank,
                id,
            })
    }

    /// `RunScriptCommand` (`src/script.c:45`): one frame of this
    /// context. In NATIVE mode polls the wait and, once it holds,
    /// returns to BYTECODE for the *next* frame; in BYTECODE mode runs
    /// commands until one yields. `Ok(true)` while the context lives,
    /// `Ok(false)` once it has stopped (the caller destroys it).
    ///
    /// # Errors
    /// Any [`ScriptError`] a command raises — an unknown or
    /// unimplemented opcode, a truncated operand, an unresolvable
    /// variable. The context is stopped first.
    pub fn step(
        &mut self,
        env: &mut ScriptEnvironment,
        host: &mut dyn ScriptHost,
    ) -> Result<bool, ScriptError> {
        match self.mode {
            Mode::Stopped => return Ok(false),
            Mode::Native => {
                if let Some(wait) = self.native.clone() {
                    let done = match exec::run_native(&wait, self, env, host) {
                        Ok(done) => done,
                        Err(err) => {
                            self.stop();
                            return Err(err);
                        }
                    };
                    if done {
                        self.mode = Mode::Bytecode;
                        self.native = None;
                    }
                    return Ok(true);
                }
                self.mode = Mode::Bytecode;
            }
            Mode::Bytecode => {}
        }
        loop {
            let Some(pc) = self.pc else {
                self.mode = Mode::Stopped;
                return Ok(false);
            };
            let raw = match self.read_u16() {
                Ok(raw) => raw,
                Err(err) => {
                    self.stop();
                    return Err(err);
                }
            };
            let Some(opcode) = Opcode::from_u16(raw) else {
                // GF_ASSERT(FALSE); mode = STOPPED; return FALSE.
                self.stop();
                return Err(ScriptError::UnknownOpcode {
                    opcode: raw,
                    offset: pc,
                });
            };
            match exec::execute(opcode, pc, self, env, host) {
                Ok(Flow::Continue) => {}
                Ok(Flow::Yield) => break,
                Err(err) => {
                    self.stop();
                    return Err(err);
                }
            }
        }
        Ok(true)
    }

    // ---- the executor's primitives (ScriptRead*, ScriptPush/Pop/...) ----

    /// `SetupNativeScript`.
    pub(crate) fn set_native(&mut self, wait: NativeWait) {
        self.mode = Mode::Native;
        self.native = Some(wait);
    }

    /// `ctx->comparisonResult = ...`.
    pub(crate) fn set_comparison(&mut self, result: u8) {
        self.comparison = result;
    }

    /// `ctx->data[reg]`, bounds-checked.
    pub(crate) fn register(&self, reg: u8, offset: usize) -> Result<u32, ScriptError> {
        self.data
            .get(usize::from(reg))
            .copied()
            .ok_or(ScriptError::BadRegister {
                register: reg,
                offset,
            })
    }

    /// `ctx->data[reg] = value`, bounds-checked.
    pub(crate) fn set_register(&mut self, reg: u8, value: u32, offset: usize) -> Result<(), ScriptError> {
        match self.data.get_mut(usize::from(reg)) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(ScriptError::BadRegister {
                register: reg,
                offset,
            }),
        }
    }

    /// `ctx->data[i]` without a check (the natives use fixed indices).
    pub(crate) fn data(&self, i: usize) -> u32 {
        self.data[i]
    }

    /// `ctx->data[i] = value` without a check.
    pub(crate) fn set_data(&mut self, i: usize, value: u32) {
        self.data[i] = value;
    }

    fn read(&mut self, size: usize) -> Result<u32, ScriptError> {
        let pc = self.pc.ok_or(ScriptError::Truncated { offset: 0 })?;
        let bytes = self
            .bank
            .bytes()
            .get(pc..pc + size)
            .ok_or(ScriptError::Truncated { offset: pc })?;
        let value = bytes
            .iter()
            .rev()
            .fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
        self.pc = Some(pc + size);
        Ok(value)
    }

    /// `ScriptReadByte`.
    pub(crate) fn read_u8(&mut self) -> Result<u8, ScriptError> {
        self.read(1).map(|v| v as u8)
    }

    /// `ScriptReadHalfword`.
    pub(crate) fn read_u16(&mut self) -> Result<u16, ScriptError> {
        self.read(2).map(|v| v as u16)
    }

    /// `ScriptReadWord`.
    pub(crate) fn read_u32(&mut self) -> Result<u32, ScriptError> {
        self.read(4)
    }

    /// The address a just-read relative word names: `script_ptr +
    /// offset`, with pointer wraparound.
    pub(crate) fn relative(&self, word: u32) -> usize {
        self.pc
            .unwrap_or(0)
            .wrapping_add(word as i32 as isize as usize)
    }

    /// `ScriptPush` — `true` when the stack is full (the original
    /// refuses the push and returns `TRUE`).
    pub(crate) fn push(&mut self, offset: usize) -> bool {
        if usize::from(self.stack_depth) + 1 >= STACK_DEPTH {
            return true;
        }
        self.stack[usize::from(self.stack_depth)] = offset;
        self.stack_depth += 1;
        false
    }

    /// `ScriptPop` — `None` on an empty stack.
    pub(crate) fn pop(&mut self) -> Option<usize> {
        if self.stack_depth == 0 {
            return None;
        }
        self.stack_depth -= 1;
        Some(self.stack[usize::from(self.stack_depth)])
    }

    /// `ScriptJump`.
    pub(crate) fn jump(&mut self, offset: usize) {
        self.pc = Some(offset);
    }

    /// `ScriptCall` — pushes the current pc (silently dropped when the
    /// stack is full, as the original does) and jumps.
    pub(crate) fn call(&mut self, offset: usize) {
        if let Some(pc) = self.pc {
            self.push(pc);
        }
        self.pc = Some(offset);
    }

    /// `ScriptReturn` — pops into the pc; an empty stack leaves it
    /// `NULL`, which stops the script on the next command fetch.
    pub(crate) fn ret(&mut self) {
        self.pc = self.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(code: &[u8]) -> ScriptContext {
        let mut member = vec![0, 0, 0, 0, 0x13, 0xFD];
        member.extend_from_slice(code);
        // Entry 0 → after the word (4) + 2 = the code start.
        member[0] = 2;
        let bank = ScriptBank::parse(member).unwrap();
        let mut ctx = ScriptContext::new(bank, None, 0, 0);
        ctx.run_by_index(0).unwrap();
        ctx
    }

    #[test]
    fn reads_advance_the_pc_little_endian() {
        let mut ctx = context(&[0x34, 0x12, 0x78, 0x56, 0xAA, 0xBB, 0xCC, 0xDD, 9]);
        assert_eq!(ctx.pc(), Some(6));
        assert_eq!(ctx.read_u16().unwrap(), 0x1234);
        assert_eq!(ctx.read_u8().unwrap(), 0x78);
        assert_eq!(ctx.read_u8().unwrap(), 0x56);
        assert_eq!(ctx.read_u32().unwrap(), 0xDDCC_BBAA);
        assert_eq!(ctx.pc(), Some(14));
        assert_eq!(ctx.read_u8().unwrap(), 9);
        assert!(matches!(
            ctx.read_u16(),
            Err(ScriptError::Truncated { offset: 15 })
        ));
        assert!(matches!(ctx.run_by_index(1), Err(ScriptError::NoSuchScript { index: 1, count: 1 })));
    }

    #[test]
    fn stack_holds_nineteen_frames_like_the_original() {
        let mut ctx = context(&[2, 0]);
        // ScriptPush refuses when depth + 1 >= 20: the 20th push fails.
        for i in 0..19 {
            assert!(!ctx.push(i), "push {i}");
        }
        assert!(ctx.push(99));
        assert_eq!(ctx.stack().len(), 19);
        assert_eq!(ctx.pop(), Some(18));
        ctx.call(40);
        assert_eq!(ctx.pc(), Some(40));
        ctx.ret();
        // Returned to the pc before the call (the code start, 6).
        assert_eq!(ctx.pc(), Some(6));
        for _ in 0..18 {
            ctx.pop();
        }
        assert_eq!(ctx.pop(), None);
        ctx.ret();
        assert_eq!(ctx.pc(), None, "a return on an empty stack nulls the pc");
    }

    #[test]
    fn relative_targets_wrap_backwards() {
        let mut ctx = context(&[0; 8]);
        ctx.read_u32().unwrap();
        assert_eq!(ctx.pc(), Some(10));
        assert_eq!(ctx.relative(0xFFFF_FFFC), 6);
        assert_eq!(ctx.relative(4), 14);
    }

    #[test]
    fn comparison_table_matches_scrcmd_h() {
        assert_eq!(compare(1, 2), comparison::LESS);
        assert_eq!(compare(2, 2), comparison::EQUAL);
        assert_eq!(compare(3, 2), comparison::GREATER);
        // ne is true for < and >; ge for = and >.
        assert_eq!(CONDITION_TABLE[5], [1, 0, 1]);
        assert_eq!(CONDITION_TABLE[4], [0, 1, 1]);
    }
}
