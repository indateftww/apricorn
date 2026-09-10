//! The CPU: registers, flags, and the ARMv5TE decode-execute loop.
//!
//! Decode is inline in the two big matches (`exec_arm`, `exec_thumb`)
//! rather than a separate instruction IR — the leaves arm-runner calls
//! exercise a bounded encoding set, and an IR would double the code
//! for nothing. Every encoding the retail leaves use decodes; every
//! encoding they *don't* fails loudly as
//! [`RunnerError::Unsupported`](super::RunnerError::Unsupported).
//! The genuinely fiddly bits (conditions, the barrel shifter, sign
//! extension) live in [`super::decode`] with their own unit tests.
//!
//! PC convention: `regs[15]` always holds the address of the
//! instruction being executed; operand reads of r15 add the mode's
//! read-ahead (8 in ARM, 4 in Thumb) at the use site.

use super::RunnerError;
use super::decode::{
    Flags, Shift, arm_imm12, cond_holds, sign_extend8, sign_extend11, sign_extend24,
};
use super::mem::Memory;

/// The register file + NZCV + the one memory the machine has.
#[derive(Debug, Clone)]
pub struct Cpu {
    regs: [u32; 16],
    flags: Flags,
    thumb: bool,
    /// The sticky saturation (Q) flag the v5TE DSP ops set.
    q: bool,
    mem: Memory,
    /// Instructions executed since the last reset (diagnostics).
    steps: u32,
}

impl Cpu {
    /// A CPU over `mem` with r13 at the scratch stack top and
    /// everything else zero.
    #[must_use]
    pub fn new(mem: Memory) -> Self {
        let mut regs = [0u32; 16];
        regs[13] = super::mem::SCRATCH_STACK_TOP;
        Self {
            regs,
            flags: Flags::default(),
            thumb: false,
            q: false,
            mem,
            steps: 0,
        }
    }

    /// The current register file (r15 = instruction address).
    #[must_use]
    pub fn regs(&self) -> &[u32; 16] {
        &self.regs
    }

    /// The NZCV flags.
    #[must_use]
    pub fn flags(&self) -> Flags {
        self.flags
    }

    /// Whether the CPU is in Thumb state.
    #[must_use]
    pub fn is_thumb(&self) -> bool {
        self.thumb
    }

    /// Instructions executed since `new`.
    #[must_use]
    pub fn steps(&self) -> u32 {
        self.steps
    }

    /// The interpreter's memory.
    #[must_use]
    pub fn mem(&self) -> &Memory {
        &self.mem
    }

    /// The interpreter's memory, mutable (for seeding the globals a
    /// call probe needs).
    pub fn mem_mut(&mut self) -> &mut Memory {
        &mut self.mem
    }

    /// Sets a register directly (harness setup; r13, or seeding an
    /// argument register the call convention would set anyway).
    pub fn set_reg(&mut self, r: usize, value: u32) {
        self.regs[r] = value;
    }

    /// Enters ARM or Thumb state at `entry` (bit 0 decides).
    #[must_use]
    pub fn enter(entry: u32) -> Self {
        let mut cpu = Self::new(Memory::new());
        cpu.jump(entry);
        cpu
    }

    /// Jumps to `target`, switching mode per bit 0 (BX semantics).
    pub fn jump(&mut self, target: u32) {
        self.thumb = target & 1 != 0;
        self.regs[15] = target & !1;
    }

    /// Executes one instruction (a BL pair in Thumb counts as one).
    ///
    /// # Errors
    /// Returns a [`RunnerError`] for unmapped or unaligned memory,
    /// unsupported encodings, and `SWI`.
    pub fn step(&mut self) -> Result<(), RunnerError> {
        let pc = self.regs[15];
        let hw = self.mem.read16(pc)?;
        if self.thumb {
            self.regs[15] = pc.wrapping_add(2);
            // A BL prefix needs its suffix halfword to mean anything.
            if hw >> 11 == 0b11110 {
                let hw2 = self.mem.read16(pc.wrapping_add(2))?;
                self.regs[15] = pc.wrapping_add(4);
                self.exec_thumb_bl(u32::from(hw), u32::from(hw2), pc)?;
            } else {
                self.exec_thumb(hw, pc)?;
            }
        } else {
            // Little-endian word fetch: the low halfword is at pc.
            let hi = u32::from(self.mem.read16(pc.wrapping_add(2))?);
            let word = hi << 16 | u32::from(hw);
            self.regs[15] = pc.wrapping_add(4);
            self.exec_arm(word, pc)?;
        }
        self.steps = self.steps.wrapping_add(1);
        Ok(())
    }

    /// An operand read of register `r` at instruction `pc` (r15 reads
    /// the mode's read-ahead value).
    fn reg_operand(&self, r: usize, pc: u32) -> u32 {
        if r == 15 {
            pc.wrapping_add(if self.thumb { 4 } else { 8 })
        } else {
            self.regs[r]
        }
    }

    /// Writes r15 with v5 interworking (bit 0 selects Thumb) — the
    /// `BX`/`BLX`/`pop {pc}`/`ldm pc` semantics.
    fn bx(&mut self, target: u32) {
        self.thumb = target & 1 != 0;
        self.regs[15] = target & !1;
    }

    /// Sets N and Z from `result`.
    fn set_nz(&mut self, result: u32) {
        self.flags.n = result >> 31 != 0;
        self.flags.z = result == 0;
    }

    /// A data-processing result into `rd`. Flags are *not* touched
    /// here — the caller applies them only for the S-bit forms.
    fn write_result(&mut self, rd: usize, result: u32) {
        if rd == 15 {
            // ARMv5: writing r15 by data processing does not
            // interwork (only BX/BLX/LDM do).
            self.regs[15] = result & !1;
        } else {
            self.regs[rd] = result;
        }
    }

    /// ADD-with-carry-in semantics: result, carry, overflow.
    fn add_with_carry(a: u32, b: u32, carry_in: bool) -> (u32, bool, bool) {
        let c = u32::from(carry_in);
        let (r1, o1) = a.overflowing_add(b);
        let (r2, o2) = r1.overflowing_add(c);
        let result = r2;
        let carry = o1 || o2;
        let overflow = ((a ^ result) & (b ^ result)) >> 31 != 0;
        (result, carry, overflow)
    }

    // ===== ARM =========================================================

    /// Executes the ARM instruction `word` at address `pc`.
    fn exec_arm(&mut self, word: u32, pc: u32) -> Result<(), RunnerError> {
        let cond = (word >> 28) as u8;
        let bits27_25 = word >> 25 & 7;

        // Branch-with-link-as-1111-condition is BLX-imm and always
        // executes; SWI too. Everything else honors its condition.
        let is_blx_imm = cond == 0b1111 && bits27_25 == 0b101;
        let is_swi = cond == 0b1111 && word >> 24 == 0b1111_1111 >> 4;
        if !is_blx_imm && !is_swi && !cond_holds(cond, self.flags) {
            return Ok(());
        }

        match bits27_25 {
            0b000 => self.exec_arm_misc(word, pc),
            0b001 => self.exec_arm_data_proc(word, pc, arm_imm12(word), None),
            0b010 => self.exec_arm_single(word, pc, word & 0xFFF),
            0b011 => {
                if word & 0x10 != 0 {
                    return Err(RunnerError::Unsupported {
                        addr: pc,
                        enc: word,
                    });
                }
                let rm = word & 0xF;
                let shift = Shift::from_bits(word >> 5 & 3);
                let amount = word >> 7 & 0x1F;
                self.exec_arm_single(
                    word,
                    pc,
                    Shift::apply(shift, self.regs[rm as usize], amount, self.flags.c).0,
                )
            }
            0b100 => self.exec_arm_block(word, pc),
            0b101 => {
                let offset = sign_extend24(word) << 2;
                let target = pc.wrapping_add(8).wrapping_add(offset);
                if word & 0x0100_0000 != 0 {
                    // BL
                    self.regs[14] = pc.wrapping_add(4);
                }
                if is_blx_imm {
                    // BLX imm: H doubles the offset and forces Thumb.
                    let h = (word >> 24 & 1) << 1;
                    self.bx(pc.wrapping_add(8).wrapping_add(offset).wrapping_add(h));
                } else {
                    self.regs[15] = target;
                }
                Ok(())
            }
            0b110 | 0b111 => self.exec_arm_coproc_swi(word, pc),
            _ => unreachable!("three bits cannot exceed 0b111"),
        }
    }

    /// The bits 27-25 = 000 space: multiply, halfword/signed
    /// transfers, `BX`/`BLX`, `CLZ`, the DSP saturating ops, and
    /// register-shifted data processing.
    fn exec_arm_misc(&mut self, word: u32, pc: u32) -> Result<(), RunnerError> {
        // Register transfers and special ops, by full mask.
        match word & 0x0FFF_FFF0 {
            0x0100_0010 | 0x012F_FF10 => {
                // BX (the second arm is the no-conditional alias).
                let rm = word & 0xF;
                let target = self.reg_operand(rm as usize, pc);
                self.bx(target);
                return Ok(());
            }
            0x012F_FF30 => {
                // BLX (register) — link to the next instruction first.
                let rm = word & 0xF;
                let target = self.reg_operand(rm as usize, pc);
                self.regs[14] = pc.wrapping_add(4) | 1;
                self.bx(target);
                return Ok(());
            }
            _ => {}
        }
        if word & 0x0FFF_0FF0 == 0x016F_0F10 {
            // CLZ
            let rd = word >> 12 & 0xF;
            let rm = word & 0xF;
            let value = self.reg_operand(rm as usize, pc);
            self.regs[rd as usize] = value.leading_zeros();
            return Ok(());
        }
        // The DSP saturating family.
        let dsp = word & 0x0FF0_0FF0;
        if (0x0100_0050..=0x0100_00B0).contains(&dsp)
            && matches!(dsp, 0x0100_0050 | 0x0100_0070 | 0x0100_0090 | 0x0100_00B0)
        {
            let rn = word >> 16 & 0xF;
            let rd = word >> 12 & 0xF;
            let rm = word & 0xF;
            let a = self.reg_operand(rn as usize, pc);
            let b = self.reg_operand(rm as usize, pc);
            let doubled = match dsp {
                0x0100_0050 => self.qadd(a, b),
                0x0100_0070 => self.qsub(a, b),
                0x0100_0090 => {
                    let b2 = self.qdoulbe_sat(b);
                    self.qadd(a, b2)
                }
                _ => {
                    let b2 = self.qdoulbe_sat(b);
                    self.qsub(a, b2)
                }
            };
            self.regs[rd as usize] = doubled;
            return Ok(());
        }
        // MRS/MSR: the leaves never run privileged code — loud error.
        if word & 0x0FBF_0FFF == 0x010F_0000 || word & 0x0FBF_F000 == 0x0129_F000 {
            return Err(RunnerError::Unsupported {
                addr: pc,
                enc: word,
            });
        }
        // Multiply family (bits 27-23 zero, bits 7-4 = 1001).
        if (word >> 4) & 0x0F == 0b1001 {
            if word & 0x0F00_0000 == 0 {
                return self.exec_arm_multiply(word, pc);
            }
            // SWP and friends: not implemented, loudly.
            return Err(RunnerError::Unsupported {
                addr: pc,
                enc: word,
            });
        }
        // Halfword/signed transfers: bit 7 = 1, bit 4 = 1
        // (bits 7-4 in 1011/1101/1111); bit 22 selects the immediate
        // (0x10-immediate) vs register offset form.
        if word >> 4 & 0b1001 == 0b1001 && word & 0x90 == 0x90 {
            return self.exec_arm_halfword(word, pc);
        }
        // Data processing: bit 4 = 1 means a register-specified
        // shift amount.
        let rm = word & 0xF;
        let shift = Shift::from_bits(word >> 5 & 3);
        let amount = if word & 0x10 != 0 {
            self.regs[(word >> 8 & 0xF) as usize]
        } else {
            word >> 7 & 0x1F
        };
        let value = self.reg_operand(rm as usize, pc);
        let (shifted, carry) = Shift::apply(shift, value, amount, self.flags.c);
        self.exec_arm_data_proc(word, pc, shifted, Some(carry))
    }

    /// ARM multiply family: MUL/MLA/UMULL/UMLAL/SMULL/SMLAL.
    fn exec_arm_multiply(&mut self, word: u32, pc: u32) -> Result<(), RunnerError> {
        let hi = (word >> 16 & 15) as usize;
        let lo = (word >> 12 & 15) as usize;
        let a = self.reg_operand((word & 15) as usize, pc);
        let b = self.reg_operand((word >> 8 & 15) as usize, pc);
        let accumulate = word & (1 << 21) != 0;
        let set_flags = word & (1 << 20) != 0;
        if word & (1 << 23) == 0 {
            let mut result = a.wrapping_mul(b);
            if accumulate {
                result = result.wrapping_add(self.reg_operand(lo, pc));
            }
            self.regs[hi] = result;
            if set_flags {
                self.set_nz(result);
            }
        } else {
            let mut result = if word & (1 << 22) != 0 {
                (a as i32 as i64).wrapping_mul(b as i32 as i64) as u64
            } else {
                (a as u64) * (b as u64)
            };
            if accumulate {
                let addend = (self.regs[hi] as u64) << 32 | self.regs[lo] as u64;
                result = result.wrapping_add(addend);
            }
            self.regs[lo] = result as u32;
            self.regs[hi] = (result >> 32) as u32;
            if set_flags {
                self.flags.n = result >> 63 != 0;
                self.flags.z = result == 0;
            }
        }
        Ok(())
    }

    /// Signed 32-bit saturating add (sets Q on saturation).
    fn qadd(&mut self, a: u32, b: u32) -> u32 {
        let (r, overflowed) = (a as i32).overflowing_add(b as i32);
        if overflowed {
            // Both operands share the overflowing direction's sign.
            self.q = true;
            if (a as i32) >= 0 {
                0x7FFF_FFFF
            } else {
                0x8000_0000
            }
        } else {
            r as u32
        }
    }

    /// Signed 32-bit saturating subtract (sets Q on saturation).
    fn qsub(&mut self, a: u32, b: u32) -> u32 {
        let (r, overflowed) = (a as i32).overflowing_sub(b as i32);
        if overflowed {
            self.q = true;
            if (a as i32) >= 0 {
                0x7FFF_FFFF
            } else {
                0x8000_0000
            }
        } else {
            r as u32
        }
    }

    /// QDADD/QDSUB's "saturating double" (sets Q on saturation).
    fn qdoulbe_sat(&mut self, b: u32) -> u32 {
        self.qadd(b, b)
    }

    /// ARM halfword/signed transfers.
    fn exec_arm_halfword(&mut self, word: u32, pc: u32) -> Result<(), RunnerError> {
        let p = word & 0x0100_0000 != 0;
        let up = word & 0x0080_0000 != 0;
        let wb = word & 0x0020_0000 != 0;
        let load = word & 0x0010_0000 != 0;
        let sh = word >> 5 & 3;
        let imm_form = word & 0x0040_0000 != 0;
        let rn = word >> 16 & 0xF;
        let rd = word >> 12 & 0xF;
        let offset = if imm_form {
            (word >> 4 & 0xF0) | word & 0xF
        } else {
            let rm = word & 0xF;
            self.reg_operand(rm as usize, pc)
        };
        let base = self.reg_operand(rn as usize, pc);
        let effective = if up {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let addr = if p { effective } else { base };

        let value = if load {
            match sh {
                0b01 => u32::from(self.mem.read16(addr)?),
                0b10 => self.mem.read8(addr)? as i8 as i32 as u32, // LDRSB
                _ => self.mem.read16(addr)? as i16 as i32 as u32,  // LDRSH
            }
        } else {
            let v = self.reg_operand(rd as usize, pc);
            self.mem.write16(addr, v as u16)?;
            if !p || wb {
                self.writeback(rn as usize, effective);
            }
            return Ok(());
        };
        self.regs[rd as usize] = value;
        if !p || wb {
            self.writeback(rn as usize, effective);
        }
        Ok(())
    }

    /// Single data transfer (LDR/STR/byte, imm or reg offset already
    /// resolved by the caller).
    fn exec_arm_single(&mut self, word: u32, pc: u32, offset: u32) -> Result<(), RunnerError> {
        let p = word & 0x0100_0000 != 0;
        let up = word & 0x0080_0000 != 0;
        let byte = word & 0x0040_0000 != 0;
        let wb = word & 0x0020_0000 != 0;
        let load = word & 0x0010_0000 != 0;
        let rn = word >> 16 & 0xF;
        let rd = word >> 12 & 0xF;
        let base = self.reg_operand(rn as usize, pc);
        let effective = if up {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let addr = if p { effective } else { base };

        if load {
            let value = if byte {
                u32::from(self.mem.read8(addr)?)
            } else {
                self.mem.read32(addr)?
            };
            self.regs[rd as usize] = value;
        } else {
            let value = self.reg_operand(rd as usize, pc);
            if byte {
                self.mem.write8(addr, value as u8)?;
            } else {
                self.mem.write32(addr, value)?;
            }
        }
        if !p || wb {
            self.writeback(rn as usize, effective);
        }
        Ok(())
    }

    /// Single-transfer writeback: the base register becomes the
    /// post-transfer effective address (callers already applied the
    /// pre/post-index rules to decide whether to call this).
    fn writeback(&mut self, rn: usize, effective: u32) {
        self.regs[rn] = effective;
    }

    /// ARM LDM/STM.
    fn exec_arm_block(&mut self, word: u32, pc: u32) -> Result<(), RunnerError> {
        let p = word & 0x0100_0000 != 0;
        let up = word & 0x0080_0000 != 0;
        let sbit = word & 0x0040_0000 != 0;
        let wb = word & 0x0020_0000 != 0;
        let load = word & 0x0010_0000 != 0;
        let rn = word >> 16 & 0xF;
        let list = word & 0xFFFF;
        if list == 0 {
            return Err(RunnerError::Unsupported {
                addr: pc,
                enc: word,
            });
        }
        if sbit {
            // SPSR transfers: the leaves never execute privileged code.
            return Err(RunnerError::Unsupported {
                addr: pc,
                enc: word,
            });
        }
        let base = self.reg_operand(rn as usize, pc);
        let count = list.count_ones();
        let start = if up {
            if p { base.wrapping_add(4) } else { base }
        } else if p {
            base.wrapping_sub(count * 4)
        } else {
            base.wrapping_sub(count * 4).wrapping_add(4)
        };
        let mut addr = start;
        for r in 0..16 {
            if list & 1 << r == 0 {
                continue;
            }
            if load {
                let v = self.mem.read32(addr)?;
                if r == 15 {
                    // ARMv5: LDM-to-PC interworks per bit 0.
                    self.bx(v);
                } else {
                    self.regs[r] = v;
                }
            } else {
                let v = if r == 15 {
                    pc.wrapping_add(12)
                } else {
                    self.regs[r]
                };
                self.mem.write32(addr, v)?;
            }
            addr = addr.wrapping_add(4);
        }
        if wb && !(load && list & 1 << rn != 0) {
            // Writeback lands on the last transferred address + 4
            // (ascending) or the first one (descending):
            // STMDB starts at the lowest address, STMDA one above it.
            let end = if up {
                start.wrapping_add(count * 4)
            } else if p {
                start
            } else {
                start.wrapping_sub(4)
            };
            self.regs[rn as usize] = end;
        }
        Ok(())
    }

    /// The coprocessor/SWI space (bits 27-25 = 11x): CP15 MCR/MRC as
    /// benign no-ops, `SWI` as a loud error.
    fn exec_arm_coproc_swi(&mut self, word: u32, pc: u32) -> Result<(), RunnerError> {
        let top4 = word >> 24 & 0xF;
        match top4 {
            0b1110 => {
                // Coprocessor data ops: MRC reads 0, MCR/CDP do
                // nothing. The leaves never touch CP15 either way.
                if word & 0x0010_0000 != 0 && word & 0x10 != 0 {
                    // MRC: Rd gets an implementation-defined zero.
                    let rd = word >> 12 & 0xF;
                    if rd != 15 {
                        self.regs[rd as usize] = 0;
                    }
                }
                Ok(())
            }
            0b1111 => Err(RunnerError::Swi {
                addr: pc,
                num: word & 0x00FF_FFFF,
            }),
            _ => Err(RunnerError::Unsupported {
                addr: pc,
                enc: word,
            }),
        }
    }

    /// Shared ARM data-processing core (operand 2 already computed;
    /// `shift_carry` the immediate/shift carry-out).
    fn exec_arm_data_proc(
        &mut self,
        word: u32,
        pc: u32,
        operand: u32,
        shift_carry: Option<bool>,
    ) -> Result<(), RunnerError> {
        let opcode = word >> 21 & 0xF;
        let set_flags = word & 0x0010_0000 != 0;
        let rn = word >> 16 & 0xF;
        let rd = word >> 12 & 0xF;
        let a = self.reg_operand(rn as usize, pc);
        let flags = self.flags;

        let (result, write, carry, overflow) = match opcode {
            0x0 => logic(a & operand, true, shift_carry),   // AND
            0x1 => logic(a ^ operand, true, shift_carry),   // EOR
            0x2 => arith(sub(a, operand, false)),           // SUB
            0x3 => arith(sub(operand, a, false)),           // RSB
            0x4 => arith(add(a, operand, false)),           // ADD
            0x5 => arith(add(a, operand, flags.c)),         // ADC
            0x6 => arith(sub(a, operand, !flags.c)),        // SBC
            0x7 => arith(sub(operand, a, !flags.c)),        // RSC
            0x8 => logic(a & operand, false, shift_carry),  // TST
            0x9 => logic(a ^ operand, false, shift_carry),  // TEQ
            0xA => cmp_only(arith(sub(a, operand, false))), // CMP
            0xB => cmp_only(arith(add(a, operand, false))), // CMN
            0xC => logic(a | operand, true, shift_carry),   // ORR
            0xD => logic(operand, true, shift_carry),       // MOV
            0xE => logic(a & !operand, true, shift_carry),  // BIC
            0xF => logic(!operand, true, shift_carry),      // MVN
            _ => unreachable!("four bits cannot exceed 0xF"),
        };

        if set_flags {
            if rd == 15 {
                // S-with-rd-15 is the SPSR form: out of scope.
                return Err(RunnerError::Unsupported {
                    addr: pc,
                    enc: word,
                });
            }
            self.set_nz(result);
            if let Some(c) = carry {
                self.flags.c = c;
            }
            if let Some(v) = overflow {
                self.flags.v = v;
            }
        }
        if write {
            self.write_result(rd as usize, result);
        }
        Ok(())
    }

    // ===== Thumb =======================================================

    /// Executes the Thumb halfword `hw` at address `pc`.
    fn exec_thumb(&mut self, hw: u16, pc: u32) -> Result<(), RunnerError> {
        let hw = hw as u32;
        match hw >> 11 {
            0b00000 => self.thumb_shift(Shift::Lsl, hw, pc), // LSL imm5
            0b00001 => self.thumb_shift(Shift::Lsr, hw, pc), // LSR imm5
            0b00010 => self.thumb_shift(Shift::Asr, hw, pc), // ASR imm5
            0b00011 => {
                // add/sub: bit 10 = immediate, bit 9 = subtract.
                let sub_op = hw >> 9 & 1 == 1;
                let op = if hw >> 10 & 1 == 1 {
                    hw >> 6 & 7
                } else {
                    self.regs[(hw >> 6 & 7) as usize]
                };
                let rn = self.regs[(hw >> 3 & 7) as usize];
                let rd = (hw & 7) as usize;
                let (result, _, carry, overflow) = if sub_op {
                    sub(rn, op, false)
                } else {
                    add(rn, op, false)
                };
                self.regs[rd] = result;
                self.set_nz(result);
                self.flags.c = carry;
                self.flags.v = overflow;
                Ok(())
            }
            0b00100 => {
                // MOV imm8 (flags).
                let rd = (hw >> 8 & 7) as usize;
                let value = hw & 0xFF;
                self.regs[rd] = value;
                self.set_nz(value);
                Ok(())
            }
            0b00101 => {
                // CMP imm8.
                let rn = self.regs[(hw >> 8 & 7) as usize];
                let (result, _, carry, overflow) = sub(rn, hw & 0xFF, false);
                self.set_nz(result);
                self.flags.c = carry;
                self.flags.v = overflow;
                Ok(())
            }
            0b00110 => {
                // ADD imm8.
                let rd = (hw >> 8 & 7) as usize;
                let rn = self.regs[rd];
                let (result, _, carry, overflow) = add(rn, hw & 0xFF, false);
                self.regs[rd] = result;
                self.set_nz(result);
                self.flags.c = carry;
                self.flags.v = overflow;
                Ok(())
            }
            0b00111 => {
                // SUB imm8.
                let rd = (hw >> 8 & 7) as usize;
                let rn = self.regs[rd];
                let (result, _, carry, overflow) = sub(rn, hw & 0xFF, false);
                self.regs[rd] = result;
                self.set_nz(result);
                self.flags.c = carry;
                self.flags.v = overflow;
                Ok(())
            }
            0b01000 => self.thumb_alu_or_high(hw, pc),
            0b01001 => {
                // LDR literal: base = align(pc + 4, 4) + imm8*4.
                let rd = (hw >> 8 & 7) as usize;
                let base = (pc + 4) & !3;
                let addr = base.wrapping_add((hw & 0xFF) << 2);
                self.regs[rd] = self.mem.read32(addr)?;
                Ok(())
            }
            0b01010 | 0b01011 => {
                // Load/store with register offset (bits 9-6 select).
                let rd = (hw & 7) as usize;
                let rn = self.regs[(hw >> 3 & 7) as usize];
                let addr = rn.wrapping_add(self.regs[(hw >> 6 & 7) as usize]);
                match hw >> 9 & 7 {
                    0 => self.mem.write32(addr, self.regs[rd])?, // STR
                    1 => self.mem.write16(addr, self.regs[rd] as u16)?, // STRH
                    2 => self.mem.write8(addr, self.regs[rd] as u8)?, // STRB
                    3 => self.regs[rd] = self.mem.read8(addr)? as i8 as i32 as u32, // LDRSB
                    4 => self.regs[rd] = self.mem.read32(addr)?, // LDR
                    5 => self.regs[rd] = u32::from(self.mem.read16(addr)?), // LDRH
                    6 => self.regs[rd] = u32::from(self.mem.read8(addr)?), // LDRB
                    _ => self.regs[rd] = self.mem.read16(addr)? as i16 as i32 as u32, // LDRSH
                }
                Ok(())
            }
            0b01100 | 0b01101 => {
                // LDR/STR imm5 (word), offset scaled by 4.
                let load = hw >> 11 & 1 == 1;
                let offset = (hw >> 6 & 0x1F) << 2;
                self.thumb_load_store_imm(
                    hw,
                    load,
                    offset,
                    |mem, addr, v| mem.write32(addr, v),
                    |mem, addr| mem.read32(addr),
                )
            }
            0b01110 | 0b01111 => {
                // LDRB/STRB imm5.
                let load = hw >> 11 & 1 == 1;
                self.thumb_load_store_imm(
                    hw,
                    load,
                    hw >> 6 & 0x1F,
                    |mem, addr, v| mem.write8(addr, v as u8),
                    |mem, addr| mem.read8(addr).map(u32::from),
                )
            }
            0b10000 | 0b10001 => {
                // STRH/LDRH imm5, offset scaled by 2.
                let load = hw >> 11 & 1 == 1;
                self.thumb_load_store_imm(
                    hw,
                    load,
                    (hw >> 6 & 0x1F) << 1,
                    |mem, addr, v| mem.write16(addr, v as u16),
                    |mem, addr| mem.read16(addr).map(u32::from),
                )
            }
            0b10010 | 0b10011 => {
                // STR/LDR [sp, #imm8*4].
                let load = hw >> 11 & 1 == 1;
                let rd = (hw >> 8 & 7) as usize;
                let addr = self.regs[13].wrapping_add((hw & 0xFF) << 2);
                if load {
                    self.regs[rd] = self.mem.read32(addr)?;
                } else {
                    self.mem.write32(addr, self.regs[rd])?;
                }
                Ok(())
            }
            0b10100 | 0b10101 => {
                // ADD rd, pc|sp, #imm8*4 (bit 11 selects).
                let rd = (hw >> 8 & 7) as usize;
                let base = if hw >> 11 & 1 == 1 {
                    self.regs[13]
                } else {
                    (pc + 4) & !3
                };
                self.regs[rd] = base.wrapping_add((hw & 0xFF) << 2);
                Ok(())
            }
            0b10110 | 0b10111 => self.thumb_misc16(hw, pc),
            0b11000 | 0b11001 => self.thumb_block(hw, pc),
            0b11100 => {
                // B (unconditional).
                let offset = sign_extend11(hw) << 1;
                self.regs[15] = pc.wrapping_add(4).wrapping_add(offset);
                Ok(())
            }
            0b11101 => Err(RunnerError::Unsupported { addr: pc, enc: hw }), // BL suffix alone / undefined
            0b11110 => Err(RunnerError::Unsupported { addr: pc, enc: hw }), // BLX suffix without prefix
            0b11111 => Err(RunnerError::Unsupported { addr: pc, enc: hw }), // undefined without prefix
            _ => self.thumb_cond_branch_or_svc(hw, pc),
        }
    }

    /// Thumb format 1 shifts (imm5, flags set).
    fn thumb_shift(&mut self, shift: Shift, hw: u32, pc: u32) -> Result<(), RunnerError> {
        let _ = pc;
        let rd = (hw & 7) as usize;
        let value = self.regs[(hw >> 3 & 7) as usize];
        let amount = hw >> 6 & 0x1F;
        let (result, carry) = Shift::apply(shift, value, amount, self.flags.c);
        self.regs[rd] = result;
        self.set_nz(result);
        self.flags.c = carry;
        Ok(())
    }

    /// Format 4 ALU ops and the high-register operations.
    fn thumb_alu_or_high(&mut self, hw: u32, pc: u32) -> Result<(), RunnerError> {
        match hw >> 8 {
            0x44 => {
                // ADD (high registers): DN bit 7 picks the destination.
                let d = if hw >> 7 & 1 == 1 {
                    8 + (hw & 7) as usize
                } else {
                    (hw & 7) as usize
                };
                let rn = hw >> 3 & 0xF;
                let b = if rn == 15 {
                    self.reg_operand(15, pc)
                } else {
                    self.regs[rn as usize]
                };
                self.regs[d] = self.regs[d].wrapping_add(b);
                Ok(())
            }
            0x45 => {
                // CMP (high registers): sets flags.
                let rn = hw >> 3 & 0xF;
                let a = if rn == 15 {
                    self.reg_operand(15, pc)
                } else {
                    self.regs[rn as usize]
                };
                let b = self.regs[(hw & 7) as usize];
                let (result, _, carry, overflow) = sub(a, b, false);
                self.set_nz(result);
                self.flags.c = carry;
                self.flags.v = overflow;
                Ok(())
            }
            0x46 => {
                // MOV (high registers).
                let d = if hw >> 7 & 1 == 1 {
                    8 + (hw & 7) as usize
                } else {
                    (hw & 7) as usize
                };
                let rn = hw >> 3 & 0xF;
                let b = if rn == 15 {
                    self.reg_operand(15, pc)
                } else {
                    self.regs[rn as usize]
                };
                if d == 15 {
                    self.bx(b); // MOV pc reg does interwork in v5 Thumb
                } else {
                    self.regs[d] = b;
                }
                Ok(())
            }
            0x47 => {
                // BX/BLX (register): bit 7 selects BLX.
                let rn = hw >> 3 & 0xF;
                let target = if rn == 15 {
                    self.reg_operand(15, pc)
                } else {
                    self.regs[rn as usize]
                };
                if hw & 0x80 != 0 {
                    self.regs[14] = pc.wrapping_add(2) | 1;
                }
                self.bx(target);
                Ok(())
            }
            _ => self.thumb_format4_alu(hw),
        }
    }

    /// The sixteen format-4 ALU operations.
    fn thumb_format4_alu(&mut self, hw: u32) -> Result<(), RunnerError> {
        let rd = (hw & 7) as usize;
        let rm = self.regs[(hw >> 3 & 7) as usize];
        let flags = self.flags;
        match hw >> 6 & 0xF {
            0x0 => self.regs[rd] &= rm, // AND
            0x1 => self.regs[rd] ^= rm, // EOR
            0x2 | 0x3 | 0x4 | 0x7 => {
                // LSL/LSR/ASR/ROR by register.
                let shift = Shift::from_bits(hw >> 8 & 3);
                let (result, carry) = Shift::apply(shift, self.regs[rd], rm, flags.c);
                self.regs[rd] = result;
                self.set_nz(result);
                self.flags.c = carry;
            }
            0x5 => {
                let (r, _, c, v) = add(self.regs[rd], rm, flags.c); // ADC
                self.regs[rd] = r;
                self.set_nz(r);
                self.flags.c = c;
                self.flags.v = v;
            }
            0x6 => {
                let (r, _, c, v) = sub(self.regs[rd], rm, !flags.c); // SBC
                self.regs[rd] = r;
                self.set_nz(r);
                self.flags.c = c;
                self.flags.v = v;
            }
            0x8 => {
                let r = self.regs[rd] & rm; // TST
                self.set_nz(r);
            }
            0x9 => {
                let (r, _, c, v) = sub(0, rm, false); // NEG
                self.regs[rd] = r;
                self.set_nz(r);
                self.flags.c = c;
                self.flags.v = v;
            }
            0xA => {
                let (r, _, c, v) = sub(self.regs[rd], rm, false); // CMP
                self.set_nz(r);
                self.flags.c = c;
                self.flags.v = v;
            }
            0xB => {
                let (r, _, c, v) = add(self.regs[rd], rm, false); // CMN
                self.set_nz(r);
                self.flags.c = c;
                self.flags.v = v;
            }
            0xC => {
                self.regs[rd] |= rm; // ORR
                self.set_nz(self.regs[rd]);
            }
            0xD => {
                self.regs[rd] = self.regs[rd].wrapping_mul(rm); // MUL
                self.set_nz(self.regs[rd]);
            }
            0xE => {
                self.regs[rd] &= !rm; // BIC
                self.set_nz(self.regs[rd]);
            }
            _ => {
                self.regs[rd] = !rm; // MVN
                self.set_nz(self.regs[rd]);
            }
        }
        Ok(())
    }

    /// LDR/STR with immediate offset, shared over the word/byte/half
    /// widths (the closures pick the access).
    fn thumb_load_store_imm(
        &mut self,
        hw: u32,
        load: bool,
        offset: u32,
        store: impl Fn(&mut Memory, u32, u32) -> Result<(), RunnerError>,
        load_fn: impl Fn(&Memory, u32) -> Result<u32, RunnerError>,
    ) -> Result<(), RunnerError> {
        let rd = (hw & 7) as usize;
        let addr = self.regs[(hw >> 3 & 7) as usize].wrapping_add(offset);
        if load {
            self.regs[rd] = load_fn(&self.mem, addr)?;
        } else {
            store(&mut self.mem, addr, self.regs[rd])?;
        }
        Ok(())
    }

    /// The 0xBxxx space: ADD/SUB sp, PUSH/POP (bit 8 selects lr / pc
    /// in the list), and (unsupported) v6 control ops.
    fn thumb_misc16(&mut self, hw: u32, pc: u32) -> Result<(), RunnerError> {
        match hw & 0xFE00 {
            0xB000 => {
                // ADD (bit 7 clear) / SUB (bit 7 set) sp, #imm8*4.
                let delta = (hw & 0x7F) << 2;
                self.regs[13] = if hw & 0x80 != 0 {
                    self.regs[13].wrapping_sub(delta)
                } else {
                    self.regs[13].wrapping_add(delta)
                };
                Ok(())
            }
            0xB400 => {
                // PUSH {list, lr?}. The register copy keeps the reads
                // ahead of the sp writeback, like the hardware's
                // start-address computation.
                let saved = self.regs;
                let list = hw & 0x1FF;
                let count = list.count_ones() * 4;
                let mut addr = self.regs[13].wrapping_sub(count);
                self.regs[13] = addr;
                for r in 0..9 {
                    if list & 1 << r != 0 {
                        let v = if r == 8 { saved[14] } else { saved[r] };
                        self.mem.write32(addr, v)?;
                        addr = addr.wrapping_add(4);
                    }
                }
                Ok(())
            }
            0xBC00 => {
                // POP {list, pc?} — pc interworks per bit 0.
                let list = hw & 0x1FF;
                let mut addr = self.regs[13];
                for r in 0..9 {
                    if list & 1 << r != 0 {
                        let v = self.mem.read32(addr)?;
                        addr = addr.wrapping_add(4);
                        if r == 8 {
                            self.bx(v);
                        } else {
                            self.regs[r] = v;
                        }
                    }
                }
                self.regs[13] = addr;
                Ok(())
            }
            _ => Err(RunnerError::Unsupported { addr: pc, enc: hw }),
        }
    }

    /// STM/LDM (bits 11 = load).
    fn thumb_block(&mut self, hw: u32, _pc: u32) -> Result<(), RunnerError> {
        let load = hw >> 11 & 1 == 1;
        let rn = (hw >> 8 & 7) as usize;
        let list = hw & 0xFF;
        if list == 0 {
            // Empty list forms are deprecated "load/store pc-relative";
            // reject loudly.
            return Err(RunnerError::Unsupported {
                addr: self.regs[15],
                enc: hw,
            });
        }
        let mut addr = self.regs[rn];
        for r in 0..8 {
            if list & 1 << r != 0 {
                if load {
                    self.regs[r] = self.mem.read32(addr)?;
                } else {
                    let v = self.regs[r];
                    self.mem.write32(addr, v)?;
                }
                addr = addr.wrapping_add(4);
            }
        }
        self.regs[rn] = addr;
        Ok(())
    }

    /// Conditional B (0xD0xx-0xDEFF) and SVC (0xDFxx).
    fn thumb_cond_branch_or_svc(&mut self, hw: u32, pc: u32) -> Result<(), RunnerError> {
        if hw >> 8 == 0b1101_1111 {
            return Err(RunnerError::Swi {
                addr: pc,
                num: hw & 0xFF,
            });
        }
        let cond = hw >> 8 & 0xF;
        if cond_holds(cond as u8, self.flags) {
            let offset = sign_extend8(hw) << 1;
            self.regs[15] = pc.wrapping_add(4).wrapping_add(offset);
        }
        Ok(())
    }

    /// The BL/BLX prefix-suffix pair (prefix hw1, suffix hw2, both at
    /// addresses starting at `pc`).
    fn exec_thumb_bl(&mut self, hw1: u32, hw2: u32, pc: u32) -> Result<(), RunnerError> {
        let off_hi = sign_extend11(hw1) << 12;
        let target = pc
            .wrapping_add(4)
            .wrapping_add(off_hi)
            .wrapping_add((hw2 & 0x7FF) << 1);
        // Return address: the halfword after the pair (pc + 4, since
        // pc is the prefix address), kept in Thumb by setting bit 0.
        self.regs[14] = pc.wrapping_add(4) | 1;
        if hw2 >> 11 == 0b11111 {
            // BL suffix: stay Thumb (pc stays even; thumb bit says so).
            self.regs[15] = target;
            self.thumb = true;
        } else if hw2 >> 11 == 0b11101 {
            // BLX suffix: target word-aligned, enter ARM.
            self.regs[15] = target & !3;
            self.thumb = false;
        } else {
            return Err(RunnerError::Unsupported {
                addr: pc.wrapping_add(2),
                enc: hw2,
            });
        }
        Ok(())
    }
}

/// ADD result shape: (result, write, carry, overflow).
type Arith = (u32, bool, bool, bool);

fn add(a: u32, b: u32, carry_in: bool) -> Arith {
    let (r, c, v) = Cpu::add_with_carry(a, b, carry_in);
    (r, true, c, v)
}

fn sub(a: u32, b: u32, borrow_in: bool) -> Arith {
    // SUB: a - b - !carry_in. carry-out = !borrow.
    let (r, c, v) = Cpu::add_with_carry(a, !b, !borrow_in);
    (r, true, c, v)
}

/// The data-processing match's result shape: flag updates as
/// `Option`s (`None` = "leave the flag alone").
type Proc = (u32, bool, Option<bool>, Option<bool>);

/// A logical op: carry from the shifter, V untouched.
fn logic(result: u32, write: bool, carry: Option<bool>) -> Proc {
    (result, write, carry, None)
}

/// An arithmetic op: carry and V both update.
fn arith(r: Arith) -> Proc {
    let (result, write, c, v) = r;
    (result, write, Some(c), Some(v))
}

/// CMP/CMN: arithmetic without the writeback.
fn cmp_only(p: Proc) -> Proc {
    (p.0, false, p.2, p.3)
}
