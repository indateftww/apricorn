//! The leaf-call convention: seed registers, run to the sentinel.
//!
//! arm-runner never runs a program — it calls *one function*: the
//! caller loads the ARM9 image, points a [`Cpu`] at a pinned function
//! address, and [`call`] sets up the AAPCS-shaped entry state
//! (r0–r3 arguments, r13 on the scratch stack, lr at the sentinel)
//! and steps until the function returns to the sentinel.

use super::RunnerError;
use super::exec::Cpu;
use super::mem::SCRATCH_STACK_TOP;

/// The fake return address every leaf call comes back to. It maps to
/// nothing on purpose: the run loop stops *before* fetching there.
pub const SENTINEL: u32 = 0xFFFF_0000;

/// Default step budget: generous for the fattest pinned leaf (the MT
/// twist loop) and small enough that a runaway dies in milliseconds.
pub const DEFAULT_STEP_BUDGET: u32 = 4 * 1024 * 1024;

/// What a leaf call left behind: the return registers and how many
/// instructions it took (diagnostics, and a cheap extra equality signal
/// for differential probes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallResult {
    /// r0 at return (the C return value).
    pub r0: u32,
    /// r1 at return.
    pub r1: u32,
    /// r2 at return.
    pub r2: u32,
    /// r3 at return.
    pub r3: u32,
    /// Instructions executed.
    pub steps: u32,
}

impl Cpu {
    /// Prepares the AAPCS-shaped entry state: `args` into r0–r3
    /// (shorter arrays leave the rest zero), r13 on the scratch
    /// stack, lr at the sentinel, pc at `entry` (bit 0 picks Thumb).
    ///
    /// r12, the callee-work register, is zeroed too — a leaf that
    /// *reads* it before writing is a bug worth failing on loudly.
    pub fn prepare_call(&mut self, entry: u32, args: &[u32]) {
        self.set_reg(12, 0);
        for (r, arg) in args.iter().take(4).enumerate() {
            self.set_reg(r, *arg);
        }
        self.set_reg(13, SCRATCH_STACK_TOP);
        self.set_reg(14, SENTINEL);
        self.jump(entry);
    }

    /// Steps until the PC reaches [`SENTINEL`] (or the budget dies).
    ///
    /// # Errors
    /// Returns [`RunnerError::Runaway`] if `budget` instructions
    /// execute without returning, and passes through any fault from
    /// [`Cpu::step`].
    pub fn run(&mut self, budget: u32) -> Result<CallResult, RunnerError> {
        for _ in 0..budget {
            if self.regs()[15] == SENTINEL {
                let [r0, r1, r2, r3, ..] = *self.regs();
                return Ok(CallResult {
                    r0,
                    r1,
                    r2,
                    r3,
                    steps: self.steps(),
                });
            }
            self.step()?;
        }
        Err(RunnerError::Runaway { steps: budget })
    }

    /// [`Cpu::run`] with the default budget.
    ///
    /// # Errors
    /// Same as [`Cpu::run`].
    pub fn run_default(&mut self) -> Result<CallResult, RunnerError> {
        self.run(DEFAULT_STEP_BUDGET)
    }
}

#[cfg(test)]
mod tests {
    use super::super::mem::Memory;
    use super::*;

    /// Assembles a tiny Thumb function: `movs r0, #imm; bx lr`.
    fn thumb_leaf(mem: &mut Memory, imm: u16) -> u32 {
        let entry = 0x0200_0000 | 1; // Thumb
        let base = entry & !1;
        mem.write16(base, 0x2000 | imm).expect("movs");
        mem.write16(base + 2, 0x4770).expect("bx lr");
        entry
    }

    #[test]
    fn leaf_returns_to_sentinel_with_result() {
        let mut mem = Memory::new();
        let entry = thumb_leaf(&mut mem, 42);
        let mut cpu = Cpu::new(mem);
        cpu.prepare_call(entry, &[7, 8]);
        let result = cpu.run_default().expect("leaf returns");
        assert_eq!(result.r0, 42);
        assert_eq!(result.steps, 2);
        // Arguments landed; lr untouched by the convention setup.
        assert_eq!(cpu.regs()[1], 8);
    }

    #[test]
    fn arm_leaf_returns_to_sentinel() {
        // `mov r0, #0x42; bx lr` in ARM (0x42 is 8-bit encodable).
        let mut mem = Memory::new();
        let base = 0x0200_0000u32;
        mem.write32(base, 0xE3A0_0042).expect("mov");
        mem.write32(base + 4, 0xE12F_FF1E).expect("bx lr");
        let mut cpu = Cpu::new(mem);
        cpu.prepare_call(base, &[]);
        let result = cpu.run_default().expect("leaf returns");
        assert_eq!(result.r0, 0x42);
    }

    #[test]
    fn runaway_call_hits_the_budget() {
        // `b .` — an infinite loop.
        let mut mem = Memory::new();
        mem.write32(0x0200_0000, 0xEAFF_FFFE).expect("b .");
        let mut cpu = Cpu::new(mem);
        cpu.prepare_call(0x0200_0000, &[]);
        assert_eq!(cpu.run(64), Err(RunnerError::Runaway { steps: 64 }));
    }

    #[test]
    fn nested_calls_stack_through() {
        // Outer (ARM) at 0x02000000 — the compiler-idiomatic frame:
        //   push {lr}          ; e52de004
        //   bl  inner          ; eb000002 -> 0x02000014
        //   pop {pc}           ; e8bd8000 (interworking: back to sentinel)
        // Inner (ARM) at 0x02000014:
        //   mov r0, #9         ; e3a00009
        //   bx  lr             ; e12fff1e
        let mut mem = Memory::new();
        mem.write32(0x0200_0000, 0xE52D_E004).expect("push {lr}");
        mem.write32(0x0200_0004, 0xEB00_0002).expect("bl inner");
        mem.write32(0x0200_0008, 0xE8BD_8000).expect("pop {pc}");
        mem.write32(0x0200_0014, 0xE3A0_0009).expect("mov r0, #9");
        mem.write32(0x0200_0018, 0xE12F_FF1E).expect("bx lr");
        let mut cpu = Cpu::new(mem);
        cpu.prepare_call(0x0200_0000, &[]);
        let result = cpu.run_default().expect("returns through both");
        assert_eq!(result.r0, 9);
        assert_eq!(cpu.regs()[15], SENTINEL);
        // The frame was balanced: sp back at the scratch top.
        assert_eq!(cpu.regs()[13], super::super::mem::SCRATCH_STACK_TOP);
    }
}
