//! The script/event VM — HeartGold's field bytecode interpreter
//! (`PLAN.md` Phase 5), ported from pret `src/script.c` (the
//! interpreter, 148 lines, all C), `src/script_manager.c` (the driver
//! task, bank loading, variable resolution) and the `ScrCmd_*` handlers
//! in `src/scrcmd_*.c` / `src/field/scrcmd_*.c`.
//!
//! The engine is host-agnostic: it owns only what pret's
//! `ScriptContext` and `ScriptEnvironment` own (program counter, call
//! stack, registers, the three contexts, special variables, the
//! message-format buffers) and reaches everything else — the save's
//! flags and variables, map objects, the dialogue box, sound, fades,
//! warps, input, the RNG — through the [`host::ScriptHost`] trait a
//! field scene implements. Ticks are pure functions of the host's
//! answers: no clock, no ambient randomness.
//!
//! ```text
//! bank.rs      ScriptBank — NARC a/0/1/2 member: u32 offset table + FD13 + bytecode;
//!              the std-script → bank mapping (sScriptBankMapping)
//! header.rs    the per-map init-script header (ON_TRANSITION/ON_RESUME/ON_LOAD
//!              ids, the ON_FRAME_TABLE condition table)
//! commands.rs  the 853-entry opcode table, operand layouts, decoder, disassembler
//! context.rs   ScriptContext — one running script (pret include/script.h:164)
//! env.rs       ScriptEnvironment — up to three contexts, special vars, buffers,
//!              the Task_RunScripts frame driver
//! host.rs      ScriptHost — what the VM asks of the field; RecordingHost, a mock
//! exec.rs      the command handlers (the early-game subset) and native waits
//! ```
//!
//! Execution model (`RunScriptCommand`): a context in BYTECODE mode
//! runs commands back to back until one *yields* (returns `TRUE` in
//! C); a context in NATIVE mode polls one wait predicate per frame and,
//! once it holds, resumes bytecode on the *next* frame. `End` stops the
//! context; when the environment's last context stops, the script task
//! ends. Unimplemented commands surface as
//! [`ScriptError::Unimplemented`] — never a panic — so a field scene can
//! log and recover.

pub mod bank;
pub mod commands;
pub mod context;
pub mod env;
mod exec;
pub mod header;
pub mod host;

use std::fmt;

pub use bank::{
    MapBanks, ResolvedScript, STD_BANK_MAPPING, ScriptBank, narc_member, resolve_script,
};
pub use commands::{
    Disassembly, Instruction, MovementCommand, Opcode, decode_at, disassemble, disassemble_entries,
};
pub use context::{Mode, ScriptContext};
pub use env::{FrameStatus, ScriptEnvironment};
pub use exec::{implemented_opcodes, is_implemented};
pub use header::InitScriptHeader;
pub use host::{RecordingHost, ScriptHost};

use crate::text::TextError;

/// Everything that can go wrong decoding or running a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    /// An opcode at or beyond the command table (`RunScriptCommand`'s
    /// `GF_ASSERT(FALSE)`; the context stops).
    UnknownOpcode {
        /// The raw halfword.
        opcode: u16,
        /// Where in the bank it sits.
        offset: usize,
    },
    /// An opcode or operand read past the end of the bank.
    Truncated {
        /// Where the instruction started.
        offset: usize,
    },
    /// A command the VM does not implement yet (it is in the table;
    /// the handler is not ported). The context is left stopped.
    Unimplemented {
        /// The command.
        opcode: Opcode,
        /// Where in the bank it sits.
        offset: usize,
    },
    /// A variable id that resolves to nothing (`GetVarPointer` returns
    /// `NULL` below `VAR_BASE`, and the original asserts on ids past the
    /// saved and special ranges).
    BadVar {
        /// The offending id.
        var: u16,
        /// The instruction that used it.
        offset: usize,
    },
    /// A script register index outside `data[4]`.
    BadRegister {
        /// The offending index.
        register: u8,
        /// The instruction that used it.
        offset: usize,
    },
    /// The bank's entry table is malformed, or a script index is past it.
    BadBank {
        /// What was wrong.
        what: &'static str,
    },
    /// `ScriptRunByIndex` past the entry table.
    NoSuchScript {
        /// The requested index.
        index: u16,
        /// How many scripts the bank has.
        count: usize,
    },
    /// The host had no such script bank.
    BankMissing {
        /// The `a/0/1/2` member id.
        bank: u16,
    },
    /// The host had no such message bank, or it did not parse.
    MessagesMissing {
        /// The `a/0/2/7` member id.
        bank: u16,
    },
    /// A message id beyond its bank.
    NoSuchMessage {
        /// The `a/0/2/7` member id.
        bank: u16,
        /// The message id.
        id: u16,
    },
    /// `CallStd` with all three context slots busy (the original would
    /// write past `scriptContexts[]`).
    TooManyContexts,
    /// A map-load script ran past the iteration budget without ending.
    Runaway {
        /// The budget that was exceeded.
        steps: usize,
    },
    /// Placeholder expansion failed.
    Text(TextError),
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOpcode { opcode, offset } => {
                write!(f, "unknown opcode {opcode:#06x} at {offset:#x}")
            }
            Self::Truncated { offset } => write!(f, "instruction at {offset:#x} is truncated"),
            Self::Unimplemented { opcode, offset } => write!(
                f,
                "unimplemented command {} (opcode {}) at {offset:#x}",
                opcode.name(),
                opcode.code()
            ),
            Self::BadVar { var, offset } => {
                write!(f, "variable {var:#06x} does not resolve (at {offset:#x})")
            }
            Self::BadRegister { register, offset } => {
                write!(f, "script register {register} out of range (at {offset:#x})")
            }
            Self::BadBank { what } => write!(f, "malformed script bank: {what}"),
            Self::NoSuchScript { index, count } => {
                write!(f, "script index {index} past the bank's {count} entries")
            }
            Self::BankMissing { bank } => write!(f, "script bank {bank} not available"),
            Self::MessagesMissing { bank } => write!(f, "message bank {bank} not available"),
            Self::NoSuchMessage { bank, id } => {
                write!(f, "message bank {bank} has no message {id}")
            }
            Self::TooManyContexts => write!(f, "all three script contexts are busy"),
            Self::Runaway { steps } => {
                write!(f, "map-load script did not end within {steps} steps")
            }
            Self::Text(err) => write!(f, "message expansion: {err}"),
        }
    }
}

impl std::error::Error for ScriptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Text(err) => Some(err),
            _ => None,
        }
    }
}

impl From<TextError> for ScriptError {
    fn from(err: TextError) -> Self {
        Self::Text(err)
    }
}
