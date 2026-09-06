//! Bit-level decoding helpers: conditions, the barrel shifter, and
//! immediate extraction.
//!
//! The CPU loop (`exec`) matches instruction encodings inline and
//! reaches for these helpers whenever the same bit arithmetic shows up
//! in more than one encoding — keeping the fiddly parts in one place,
//! unit-tested per encoding class instead of smeared across the
//! interpreter.

/// The four NZCV flags, as the condition evaluator and shifter see
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    /// N — bit 31 of the last data-processing result.
    pub n: bool,
    /// Z — the last result was zero.
    pub z: bool,
    /// C — the last shift carry out (or add's carry, sub's borrow-not).
    pub c: bool,
    /// V — the last add/sub signed overflow.
    pub v: bool,
}

impl Flags {
    /// Packs the flags into a CPSR-shaped nibble (NZCV, bits 3..=0).
    #[must_use]
    pub fn pack(self) -> u32 {
        (u32::from(self.n) << 3)
            | (u32::from(self.z) << 2)
            | (u32::from(self.c) << 1)
            | u32::from(self.v)
    }

    /// Unpacks a CPSR-shaped nibble.
    #[must_use]
    pub fn unpack(bits: u32) -> Self {
        Self {
            n: bits & 0b1000 != 0,
            z: bits & 0b0100 != 0,
            c: bits & 0b0010 != 0,
            v: bits & 0b0001 != 0,
        }
    }
}

/// Whether an ARM condition field (bits 31..28) holds for `flags`.
#[must_use]
pub fn cond_holds(cond: u8, flags: Flags) -> bool {
    let Flags { n, z, c, v } = flags;
    let base = match cond >> 1 {
        0b000 => z,            // EQ / NE
        0b001 => c,            // CS / CC
        0b010 => n,            // MI / PL
        0b011 => v,            // VS / VC
        0b100 => c && !z,      // HI / LS
        0b101 => n == v,       // GE / LT
        0b110 => !z && n == v, // GT / LE
        _ => true,             // AL (and NV, treated as AL on ARMv5)
    };
    // The odd condition codes are their partner's negation; 1111 is
    // the one odd code that is not.
    base ^ (cond & 1 == 1 && cond != 0b1111)
}

/// A barrel-shifter operation kind, decoded from an ARM data-processing
/// operand or a Thumb format-1/4 shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    /// `LSL` — logical shift left.
    Lsl,
    /// `LSR` — logical shift right.
    Lsr,
    /// `ASR` — arithmetic shift right.
    Asr,
    /// `ROR` — rotate right (`RRX` when the register-specified amount
    /// is 0).
    Ror,
}

impl Shift {
    /// Decodes a 2-bit shift kind (data-processing bits 6..5, Thumb
    /// format-1 bits 12..11).
    #[must_use]
    pub fn from_bits(bits: u32) -> Self {
        match bits {
            0b00 => Self::Lsl,
            0b01 => Self::Lsr,
            0b10 => Self::Asr,
            _ => Self::Ror,
        }
    }

    /// Applies the shift to `value`, returning the shifted result and
    /// the carry-out bit.
    ///
    /// Follows ARMv5 semantics exactly:
    ///
    /// * `LSL` by 0 carries `carry_in` out; by more than 31 gives
    ///   0 with C = bit 32's survivor or 0.
    /// * `LSR`/`ASR` by 0 (a register amount of zero) means by 32;
    ///   by more than 31 saturates (0, or the sign fill for `ASR`).
    /// * `ROR` by 0 means `RRX` (rotate through the carry bit); by
    ///   32+ wraps the amount mod 32.
    #[must_use]
    pub fn apply(self, value: u32, amount: u32, carry_in: bool) -> (u32, bool) {
        match self {
            Self::Lsl => match amount {
                0 => (value, carry_in),
                1..=31 => (value << amount, value & 1 << (32 - amount) != 0),
                32 => (0, value & 1 != 0),
                _ => (0, false),
            },
            Self::Lsr => match amount {
                0 => (0, value >> 31 != 0), // LSR #0 == LSR #32
                1..=31 => (value >> amount, value & 1 << (amount - 1) != 0),
                _ => (0, value >> 31 != 0),
            },
            Self::Asr => {
                let fill = u32::from(value >> 31 != 0) * u32::MAX;
                match amount {
                    0 => (fill, value >> 31 != 0),
                    1..=31 => {
                        let out = ((value as i32) >> amount) as u32;
                        (out, value & 1 << (amount - 1) != 0)
                    }
                    _ => (fill, value >> 31 != 0),
                }
            }
            Self::Ror => match amount {
                0 => {
                    // RRX: rotate right through carry.
                    let carry_bit = u32::from(carry_in) << 31;
                    (carry_bit | value >> 1, value & 1 != 0)
                }
                a => {
                    let a = a % 32;
                    if a == 0 {
                        (value, value >> 31 != 0)
                    } else {
                        (value.rotate_right(a), value & 1 << (a - 1) != 0)
                    }
                }
            },
        }
    }
}

/// Rotates a 12-bit ARM immediate right by its leading 4-bit field
/// (the `ror #2n` trick, bits 11..8).
#[must_use]
pub fn arm_imm12(word: u32) -> u32 {
    let imm = word & 0xFF;
    let rot = (word >> 8 & 0xF) * 2;
    imm.rotate_right(rot)
}

/// Sign-extends a 24-bit branch offset (bits 23..0) to `i32`-shaped
/// bits.
#[must_use]
pub fn sign_extend24(word: u32) -> u32 {
    let off = word & 0x00FF_FFFF;
    if off & 0x0080_0000 != 0 {
        off | 0xFF00_0000
    } else {
        off
    }
}

/// Sign-extends an 11-bit Thumb offset to `i32`-shaped bits.
#[must_use]
pub fn sign_extend11(word: u32) -> u32 {
    let off = word & 0x7FF;
    if off & 0x400 != 0 {
        off | 0xFFFF_F800
    } else {
        off
    }
}

/// Sign-extends a Thumb conditional-branch 8-bit offset.
#[must_use]
pub fn sign_extend8(word: u32) -> u32 {
    let off = word & 0xFF;
    if off & 0x80 != 0 {
        off | 0xFFFF_FF00
    } else {
        off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_follow_the_arm_table() {
        let eq = Flags {
            z: true,
            ..Flags::default()
        };
        let ne = Flags::default();
        let cs = Flags {
            c: true,
            ..Flags::default()
        };
        let mi = Flags {
            n: true,
            ..Flags::default()
        };
        let vs = Flags {
            v: true,
            ..Flags::default()
        };
        let hi = Flags {
            c: true,
            ..Flags::default()
        };
        let ge = Flags {
            n: true,
            v: true,
            ..Flags::default()
        };
        let gt = Flags {
            n: true,
            v: true,
            ..Flags::default()
        };
        let al = Flags::default();

        assert!(cond_holds(0b0000, eq)); // EQ
        assert!(cond_holds(0b0001, ne)); // NE
        assert!(cond_holds(0b0010, cs)); // CS
        assert!(cond_holds(0b0011, Flags::default())); // CC
        assert!(cond_holds(0b0100, mi)); // MI
        assert!(cond_holds(0b0101, Flags::default())); // PL
        assert!(cond_holds(0b0110, vs)); // VS
        assert!(cond_holds(0b0111, Flags::default())); // VC
        assert!(cond_holds(0b1000, hi)); // HI
        assert!(!cond_holds(0b1001, hi)); // LS = !C || Z
        assert!(cond_holds(0b1010, ge)); // GE = N == V
        assert!(cond_holds(
            0b1011,
            Flags {
                n: true,
                ..Flags::default()
            }
        )); // LT = N != V
        assert!(cond_holds(0b1100, gt)); // GT = !Z && N == V
        assert!(!cond_holds(0b1101, Flags::default())); // LE = Z || N != V
        assert!(cond_holds(
            0b1101,
            Flags {
                z: true,
                ..Flags::default()
            }
        )); // LE with Z set
        assert!(cond_holds(0b1110, al)); // AL
        assert!(cond_holds(0b1111, al)); // NV treated as always (v5)
    }

    #[test]
    fn shifter_matches_reference_values() {
        // LSL carry-outs.
        assert_eq!(Shift::Lsl.apply(1, 0, false), (1, false));
        assert_eq!(Shift::Lsl.apply(0x8000_0000, 1, false), (0, true));
        assert_eq!(Shift::Lsl.apply(1, 32, false), (0, true));
        assert_eq!(Shift::Lsl.apply(1, 33, false), (0, false));
        // LSR: amount 0 is 32; 33+ saturates to 0.
        assert_eq!(Shift::Lsr.apply(0x8000_0000, 0, true), (0, true));
        assert_eq!(Shift::Lsr.apply(4, 1, true), (2, false));
        assert_eq!(Shift::Lsr.apply(4, 33, true), (0, false));
        // ASR sign-fills.
        assert_eq!(Shift::Asr.apply(0x8000_0000, 4, true), (0xF800_0000, false));
        assert_eq!(Shift::Asr.apply(0x8000_0000, 33, true), (0xFFFF_FFFF, true));
        assert_eq!(Shift::Asr.apply(0x8000_0000, 0, false), (0xFFFF_FFFF, true));
        // ROR wraps; amount 0 is RRX.
        assert_eq!(Shift::Ror.apply(0x2, 1, true), (1, false));
        assert_eq!(Shift::Ror.apply(1, 4, false), (0x1000_0000, false));
        assert_eq!(Shift::Ror.apply(2, 32, false), (2, false));
        assert_eq!(Shift::Ror.apply(2, 0, true), (0x8000_0001, false));
        assert_eq!(Shift::Ror.apply(1, 0, true), (0x8000_0000, true));
        assert_eq!(Shift::Ror.apply(1, 0, false), (0, true));
    }

    #[test]
    fn immediate_rotation_decodes() {
        // The rotate nibble is the word's bits 11..8: 0xFF with rot 2
        // rotates right by 4.
        assert_eq!(arm_imm12(0x0000_00FF), 0xFF);
        assert_eq!(arm_imm12(0x0000_02FF), 0xF000_000F); // 0xFF ror 4
        assert_eq!(arm_imm12(0x0000_0412), 0x1200_0000); // 0x12 ror 8
    }

    #[test]
    fn sign_extensions_fill_high_bits() {
        assert_eq!(sign_extend24(0x00FF_FFFF), 0xFFFF_FFFF);
        assert_eq!(sign_extend24(0x0000_0001), 1);
        assert_eq!(sign_extend11(0x7FF), 0xFFFF_FFFF);
        assert_eq!(sign_extend11(0x1), 1);
        assert_eq!(sign_extend8(0x80), 0xFFFF_FF80);
        assert_eq!(sign_extend8(0x7F), 0x7F);
    }

    #[test]
    fn flags_pack_round_trip() {
        for bits in 0..16u32 {
            assert_eq!(Flags::unpack(bits).pack(), bits);
        }
    }
}
