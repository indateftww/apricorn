//! arm-runner — a test-only ARMv5TE interpreter for the ARM9 side.
//!
//! The per-function oracle where pret has no C: it loads the *original*
//! ARM9 image into flat RAM and calls original functions with
//! controlled inputs, capturing the registers and memory they leave
//! behind. The oracle (patched melonDS) proves the whole machine; this
//! proves individual leaves, and later serves as the differential
//! counterpart to the oracle's `call` mode (same seeded memory in,
//! same hashes out).
//!
//! **Never shipped in the game** — the crate doc in `lib.rs` says so,
//! and nothing under `arm/` is referenced outside the harness.
//!
//! Scope and honesty:
//!
//! * **Implemented:** the ARMv5TE core — full data-processing with the
//!   barrel shifter, all single-register load/store addressing modes
//!   (`LDR`/`STR`/`LDRB`/`STRB`/`LDRH`/`STRH`/`LDRSB`/`LDRSH`,
//!   pre/post-indexing, writeback), `LDM`/`STM`, `MUL`/`MLA`/
//!   `UMULL`/`UMLAL`/`SMULL`/`SMLAL`, `CLZ`, the v5TE DSP saturating
//!   family (`QADD`/`QSUB`/`QDADD`/`QDSUB`), `B`/`BL`/`BX`/`BLX` in
//!   both instruction sets, and `MCR`/`MRC` (CP15) as benign no-ops —
//!   the math_util leaves never touch them but the SDK does.
//! * **Out of scope, loudly:** interrupts, timers, DMA, caches, the
//!   ARM7, IPC. `SWI` and any unimplemented encoding are errors, not
//!   silently-skipped instructions. arm-runner runs leaf functions to
//!   a sentinel `lr`; it is not the game loop.
//! * **Correctness strategy:** per-encoding unit tests plus known-value
//!   tests from pret's C semantics (`LCRandom`, Mersenne Twister,
//!   CRC-16/CCITT), then differential probes against the oracle. There
//!   is no formal proof — the oracle is the proof.
//!
//! The encrypted secure area (below the 0x02000800 entry point) is
//! never executed; arm-runner loads the decompressed image as data and
//! only enters at pinned function addresses, all far above it.

pub mod call;
pub mod decode;
pub mod exec;
pub mod mem;
pub mod retail;

pub use call::{CallResult, SENTINEL};
pub use exec::Cpu;
pub use mem::Memory;

use std::fmt;

/// A fatal arm-runner error: unmapped memory, an unsupported encoding,
/// or a runaway call. Every one of these is a bug in the harness or a
/// pin drift — never a condition the real hardware would survive
/// quietly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerError {
    /// A read outside every mapped region.
    UnmappedRead {
        /// The faulting address.
        addr: u32,
    },
    /// A write outside every mapped region.
    UnmappedWrite {
        /// The faulting address.
        addr: u32,
    },
    /// A misaligned access (unaligned `LDR`/`LDRH`-width reads).
    Alignment {
        /// The faulting address.
        addr: u32,
        /// The required alignment in bytes (2 or 4).
        width: u32,
    },
    /// An instruction the interpreter does not implement.
    Unsupported {
        /// Where the instruction sits.
        addr: u32,
        /// The raw encoding (a halfword for Thumb).
        enc: u32,
    },
    /// `SWI` executed — arm-runner runs leaf functions, not syscalls.
    Swi {
        /// Where the instruction sits.
        addr: u32,
        /// The syscall number.
        num: u32,
    },
    /// The call exceeded its step budget — an infinite loop, or a
    /// function that never returned to the sentinel `lr`.
    Runaway {
        /// The step budget that was exhausted.
        steps: u32,
    },
}

impl fmt::Display for RunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunnerError::UnmappedRead { addr } => write!(f, "unmapped read at {addr:#010x}"),
            RunnerError::UnmappedWrite { addr } => write!(f, "unmapped write at {addr:#010x}"),
            RunnerError::Alignment { addr, width } => {
                write!(f, "unaligned {width}-byte access at {addr:#010x}")
            }
            RunnerError::Unsupported { addr, enc } => {
                write!(f, "unsupported encoding {enc:#x} at {addr:#010x}")
            }
            RunnerError::Swi { addr, num } => write!(f, "SWI {num} at {addr:#010x}"),
            RunnerError::Runaway { steps } => {
                write!(f, "call exceeded its step budget of {steps} instructions")
            }
        }
    }
}

impl std::error::Error for RunnerError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_display_with_addresses() {
        assert_eq!(
            RunnerError::UnmappedRead { addr: 0x0800_0000 }.to_string(),
            "unmapped read at 0x08000000"
        );
        assert_eq!(
            RunnerError::Swi {
                addr: 0x0200_0800,
                num: 4
            }
            .to_string(),
            "SWI 4 at 0x02000800"
        );
    }
}
