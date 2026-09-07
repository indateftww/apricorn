//! The game's `String` type — a port of pret's `pm_string.c` — plus the
//! msgenc-side trainer-name packing.
//!
//! [`GameString`] is the game's growable u16 code-unit buffer
//! (`String_New`/`String_AddChar`/...). The game's fixed heap capacity
//! (`maxsize`, asserted with `GF_ASSERT`) is not modeled: a `Vec` grows
//! where the game would assert. EOS is never stored; it is implicit at
//! the end of the units, exactly as in the C struct's invariant
//! `data[size] == EOS`.
//!
//! Behavioral parity stays in unit space; [`GameString::to_text`] is a
//! lossy debug rendering through the charmap.
//!
//! Trainer names ("TRNAME") are stored in messages as 9-bit chars
//! packed into 15-bit units behind a `0xF100` marker
//! (`String_Cat_HandleTrainerName` unpacks them;
//! `MessagesEncoder.cpp` in pret's msgenc packs them). The packed
//! stream's tail bits are saturated with ones so the terminator
//! (`EOS_TRNAME`, `0x1FF`) is always found — a name whose bit length
//! lands exactly on a 15-bit boundary (5, 10, ... chars) emits *no*
//! terminator unit, and the unpacker then reads the all-ones message
//! EOS unit. Retail TRNAME blocks always end their message, which is
//! what makes that legal; [`unpack_trainer_name`] errors loudly
//! instead of reading past its slice.

use crate::formats::EOS;
use crate::text::ctrl::{CHAR_LF, EOS_TRNAME, TRNAMECODE, TRNAME_MASK};
use crate::text::TextError;
use crate::text::render_lossy_units;

/// `string_util.h`'s `PrintingMode` (C values 0/1/2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintingMode {
    /// `PRINTING_MODE_LEFT_ALIGN` — leading zeros emit nothing.
    LeftAlign,
    /// `PRINTING_MODE_RIGHT_ALIGN` — leading positions become spaces.
    RightAlign,
    /// `PRINTING_MODE_LEADING_ZEROS` — leading zeros are written.
    LeadingZeros,
}

/// `String16_FormatInteger`'s `whichCharset`: 0 selects the JP glyph
/// digits, anything else the EN ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    /// The JP digit/hyphen/space glyphs (`sCharset_JP`).
    Jp,
    /// The EN digit/hyphen/space glyphs (`sCharset_EN`).
    En,
}

// charcode.h constants used here.
const CHAR_JP_SPACE: u16 = 1;
const CHAR_JP_0: u16 = 162;
const CHAR_JP_HYPHEN: u16 = 241;
const CHAR_JP_QUESTION_MARK: u16 = 226;
const CHAR_0: u16 = 289;
const CHAR_A: u16 = 299;
const CHAR_HYPHEN: u16 = 446;
// pret's charcode.h spellings (CHAR_a / CHAR_z), kept verbatim.
#[allow(non_upper_case_globals)]
const CHAR_a: u16 = 325;
#[allow(non_upper_case_globals)]
const CHAR_z: u16 = 350;
const CHAR_NARROW_SPACE: u16 = 482;

/// pret's `String`: the game's u16 code-unit text buffer.
///
/// Equality is unit equality; EOS is never part of the stored units.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameString {
    units: Vec<u16>,
}

impl GameString {
    /// `String_New` — an empty string.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `CopyU16ArrayToString` — copies `units` up to (not including)
    /// the first EOS.
    #[must_use]
    pub fn from_units(units: &[u16]) -> Self {
        Self {
            units: units.iter().copied().take_while(|&u| u != EOS).collect(),
        }
    }

    /// The stored code units (`String_cstr`'s data, without its EOS).
    #[must_use]
    pub fn units(&self) -> &[u16] {
        &self.units
    }

    /// `String_GetLength` — the number of stored units.
    #[must_use]
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether the string has no units.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// `String_SetEmpty`.
    pub fn set_empty(&mut self) {
        self.units.clear();
    }

    /// `String_AddChar` — appends one code unit.
    pub fn push_char(&mut self, unit: u16) {
        self.units.push(unit);
    }

    /// `String_Cat` — appends all of `src`'s units.
    pub fn cat(&mut self, src: &Self) {
        self.units.extend_from_slice(&src.units);
    }

    /// `String_IsTrainerName` — whether the string starts with the
    /// `0xF100` marker (and is thus a packed trainer name).
    #[must_use]
    pub fn is_trainer_name(&self) -> bool {
        !self.units.is_empty() && self.units[0] == TRNAMECODE
    }

    /// `String_Cat_HandleTrainerName` — appends `src`, unpacking it in
    /// place if it is a packed trainer name.
    ///
    /// The C reads the source's implicit EOS (`data[size]`); the walk
    /// models that by appending one EOS unit for the stream.
    ///
    /// # Errors
    /// [`TextError::TrnameOverrun`] if a packed stream runs past its
    /// end without the `0x1FF` terminator (retail streams always
    /// terminate — their tail bits are saturated with ones).
    pub fn cat_handle_trainer_name(&mut self, src: &Self) -> Result<(), TextError> {
        if src.is_trainer_name() {
            let mut units = src.units.clone();
            units.push(EOS);
            let (chars, _) = unpack_stream(&units, 1)?;
            self.units.extend_from_slice(&chars);
            Ok(())
        } else {
            self.cat(src);
            Ok(())
        }
    }

    /// `String_UpperCharN` — uppercases unit `n` if it is `a..z`.
    pub fn upper_char_n(&mut self, n: usize) {
        if let Some(slot) = self.units.get_mut(n) {
            let unit = *slot;
            if (CHAR_a..=CHAR_z).contains(&unit) {
                *slot = unit - CHAR_a + CHAR_A;
            }
        }
    }

    /// `String_CountLines` — 1 plus the number of LF units (an empty
    /// string is one line, as in the game).
    #[must_use]
    pub fn count_lines(&self) -> usize {
        1 + self.units.iter().filter(|&&u| u == CHAR_LF).count()
    }

    /// `String_GetLineN` — line `n` (0-based) without its LF.
    ///
    /// A line index past the end yields an empty string, exactly as
    /// the C walk running off `src->size` does.
    #[must_use]
    pub fn line_n(&self, n: u32) -> Self {
        let mut start = 0;
        if n != 0 {
            start = self.units.len();
            let mut left = n;
            for (i, &u) in self.units.iter().enumerate() {
                if u == CHAR_LF {
                    left -= 1;
                    if left == 0 {
                        start = i + 1;
                        break;
                    }
                }
            }
        }
        let mut dest = Self::new();
        for &u in &self.units[start..] {
            if u == CHAR_LF {
                break;
            }
            dest.push_char(u);
        }
        dest
    }

    /// Lossy debug rendering through the charmap (`'\n'` for LF, the
    /// unpacked name in brackets for a TRNAME string, control codes by
    /// pret command name). Not the pret-text serialization.
    #[must_use]
    pub fn to_text(&self) -> String {
        render_lossy_units(&self.units)
    }
}

/// `String16_FormatInteger`.
///
/// `ndigits` is the field width in digits, 1–10 (the game's power-table
/// size; the game `GF_ASSERT`s the destination fits — panicking here
/// mirrors that).
///
/// The C computes `num / dividend` in *u32* arithmetic (the int operand
/// converts) and `num *= -1` overflows for `i32::MIN`; both are
/// mirrored exactly via wrapping u32 math, so every input — including
/// `i32::MIN` — matches the original's two's-complement behavior.
///
/// A digit ≥ 10 (only reachable when `ndigits` is narrower than the
/// number) renders as the JP question mark, as in the game. Leading
/// positions pad with `CHAR_NARROW_SPACE` (EN) / `CHAR_JP_SPACE` (JP).
///
/// # Panics
/// Panics if `ndigits` is 0 or above 10 (the game asserts).
#[must_use]
pub fn format_integer(
    num: i32,
    ndigits: u32,
    mode: PrintingMode,
    charset: Charset,
) -> GameString {
    const POWERS: [u32; 10] = [
        1,
        10,
        100,
        1_000,
        10_000,
        100_000,
        1_000_000,
        10_000_000,
        100_000_000,
        1_000_000_000,
    ];
    assert!(
        (1..=10).contains(&ndigits),
        "String16_FormatInteger: ndigits {ndigits} outside the game's 1..=10"
    );

    let (digit0, hyphen, space) = match charset {
        Charset::Jp => (CHAR_JP_0, CHAR_JP_HYPHEN, CHAR_JP_SPACE),
        Charset::En => (CHAR_0, CHAR_HYPHEN, CHAR_NARROW_SPACE),
    };

    let negative = num < 0;
    let mut num_u = num as u32;
    let mut units = Vec::new();
    if negative {
        num_u = num_u.wrapping_neg();
        units.push(hyphen);
    }

    // The C mutates its by-value strConvMode: once a significant digit
    // is found, every following position is written.
    let mut mode = mode;
    let mut dividend = POWERS[(ndigits - 1) as usize];
    while dividend != 0 {
        let digit = (num_u / dividend) as u16;
        num_u = num_u.wrapping_sub(dividend.wrapping_mul(u32::from(digit)));
        let value = if digit < 10 {
            digit0 + digit
        } else {
            CHAR_JP_QUESTION_MARK
        };
        if mode == PrintingMode::LeadingZeros {
            units.push(value);
        } else if digit != 0 || dividend == 1 {
            mode = PrintingMode::LeadingZeros;
            units.push(value);
        } else if mode == PrintingMode::RightAlign {
            units.push(space);
        }
        dividend /= 10;
    }

    GameString { units }
}

/// Unpacks one packed trainer-name block: `units[0]` must be the
/// `0xF100` marker and the slice must be EOS-terminated (as every MAT
/// message is).
///
/// Returns the unpacked character units and the units consumed *after
/// the marker* — the walker's index once the terminator is found. That
/// is the last unit the stream read (it may legally read the message's
/// trailing EOS unit, all of whose bits are ones), or one past it when
/// the terminator read ends exactly on a unit boundary: the C advances
/// its index before testing for the terminator. Either way the next
/// position in the walking message is `marker + consumed + 1`.
///
/// # Errors
/// [`TextError::Invalid`] if `units[0]` is not the marker;
/// [`TextError::TrnameOverrun`] if the stream runs past its input
/// without the `0x1FF` terminator (retail data always terminates — the
/// packer saturates the tail bits with ones).
pub fn unpack_trainer_name(units: &[u16]) -> Result<(Vec<u16>, usize), TextError> {
    if units.first() != Some(&TRNAMECODE) {
        return Err(TextError::Invalid {
            what: "trainer name must start with the 0xF100 marker",
        });
    }
    unpack_stream(units, 1)
}

/// Packs trainer-name characters into 15-bit units — the inverse of
/// [`unpack_trainer_name`], and exactly how the retail banks were
/// written (pret msgenc's `MessagesEncoder::EncodeMessage`).
///
/// The marker and the message EOS are the caller's; a name whose bit
/// length lands exactly on a 15-bit boundary (5, 10, ... chars) emits
/// no terminator unit, leaving the unpacker to terminate on the
/// message EOS.
///
/// # Errors
/// [`TextError::Invalid`] if any character exceeds the 9-bit
/// `TRNAME_MASK`.
pub fn pack_trainer_name(chars: &[u16]) -> Result<Vec<u16>, TextError> {
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bit: u32 = 0;
    for &code in chars {
        if code & !TRNAME_MASK != 0 {
            return Err(TextError::Invalid {
                what: "trainer-name char exceeds the 9-bit TRNAME_MASK",
            });
        }
        buf |= u32::from(code) << bit;
        bit += 9;
        if bit >= 15 {
            bit -= 15;
            out.push((buf & 0x7FFF) as u16);
            buf >>= 15;
        }
    }
    if bit > 1 {
        buf |= 0xFFFF << bit;
        out.push((buf & 0x7FFF) as u16);
    }
    Ok(out)
}

/// `String_Cat_HandleTrainerName`'s inner loop: walks the packed 9-bit
/// stream starting at `units[from]`. The slice must end with the EOS
/// the C's `data[size]` invariant guarantees; reading past it is an
/// overrun.
///
/// Returns the unpacked chars and the last index the stream touched.
fn unpack_stream(units: &[u16], from: usize) -> Result<(Vec<u16>, usize), TextError> {
    let read =
        |idx: usize| -> Result<u16, TextError> { units.get(idx).copied().ok_or(TextError::TrnameOverrun) };
    let mut chars = Vec::new();
    let mut idx = from;
    let mut bit: u32 = 0;
    loop {
        let mut cur = read(idx)? >> bit & TRNAME_MASK;
        bit += 9;
        if bit >= 15 {
            idx += 1;
            bit -= 15;
            if bit != 0 {
                cur |= read(idx)? << (9 - bit) & TRNAME_MASK;
            }
        }
        if cur == EOS_TRNAME {
            break;
        }
        chars.push(cur);
    }
    Ok((chars, idx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::charmap::char_of;
    use crate::text::ctrl::{EXT_CTRL_CODE_BEGIN, parse_ext_ctrl};

    /// EN letters/digits/space as code units (`charcode.h`: `CHAR_A`=299,
    /// `CHAR_a`=325, `CHAR_0`=289, the space glyph is 478).
    fn en(s: &str) -> Vec<u16> {
        /// The EN space glyph (0x1DE).
        const CHAR_SPACE: u16 = 478;
        s.chars()
            .map(|c| match c {
                'a'..='z' => CHAR_a + (c as u16 - u16::from(b'a')),
                'A'..='Z' => CHAR_A + (c as u16 - u16::from(b'A')),
                '0'..='9' => CHAR_0 + (c as u16 - u16::from(b'0')),
                ' ' => CHAR_SPACE,
                _ => panic!("test helper: unexpected char {c:?}"),
            })
            .collect()
    }

    #[test]
    fn string_ops_mirror_pm_string() {
        let mut s = GameString::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        s.push_char(CHAR_a);
        s.push_char(CHAR_a + 1);
        assert_eq!(s.units(), &en("ab")[..]);

        // from_units stops at EOS.
        let t = GameString::from_units(&[CHAR_A, EOS, CHAR_a]);
        assert_eq!(t.units(), &en("A")[..]);

        let mut u = GameString::new();
        u.cat(&s);
        u.cat(&t);
        assert_eq!(u.to_text(), "abA");

        u.set_empty();
        assert!(u.is_empty());

        // Upper: only a..z, only at the requested position — 'a' and
        // 'c' flip, 'B' and 'D' are untouched.
        let mut mixed = GameString::from_units(&en("aBcD"));
        mixed.upper_char_n(0);
        mixed.upper_char_n(2);
        assert_eq!(mixed.to_text(), "ABCD");
        mixed.upper_char_n(9); // out of range: no-op, like the C
        assert_eq!(mixed.to_text(), "ABCD");

        // Lines: 1 + LF count; line_n splits on LF.
        let mut lines = GameString::new();
        for &c in &en("ab") {
            lines.push_char(c);
        }
        lines.push_char(CHAR_LF);
        for &c in &en("cd") {
            lines.push_char(c);
        }
        assert_eq!(lines.count_lines(), 2);
        assert_eq!(lines.line_n(0).to_text(), "ab");
        assert_eq!(lines.line_n(1).to_text(), "cd");
        assert_eq!(lines.line_n(2).to_text(), "");
        assert_eq!(GameString::new().count_lines(), 1);
    }

    #[test]
    fn format_integer_matches_the_c_modes() {
        use Charset::En;

        // LeftAlign: bare number.
        let s = format_integer(123, 3, PrintingMode::LeftAlign, En);
        assert_eq!(s.units(), &en("123")[..]);

        // RightAlign: narrow spaces before a narrower number.
        let s = format_integer(123, 5, PrintingMode::RightAlign, En);
        assert_eq!(
            s.units(),
            &[CHAR_NARROW_SPACE, CHAR_NARROW_SPACE, CHAR_0 + 1, CHAR_0 + 2, CHAR_0 + 3][..],
            "two leading narrow spaces, then 123"
        );

        // LeadingZeros.
        let s = format_integer(123, 5, PrintingMode::LeadingZeros, En);
        assert_eq!(s.units(), &en("00123")[..]);

        // Negative: hyphen glyph first, digits unmodified.
        let s = format_integer(-42, 3, PrintingMode::LeftAlign, En);
        assert_eq!(s.units(), &[CHAR_HYPHEN, CHAR_0 + 4, CHAR_0 + 2][..]);

        // A digit ≥ 10 (ndigits narrower than the number) is the JP
        // question mark: 42 with width 1 divides 42/1 = 42.
        let s = format_integer(42, 1, PrintingMode::LeftAlign, En);
        assert_eq!(s.units(), &[CHAR_JP_QUESTION_MARK][..]);

        // Zero: only the final dividend (1) emits.
        let s = format_integer(0, 3, PrintingMode::LeftAlign, En);
        assert_eq!(s.units(), &en("0")[..]);

        // JP charset digits are the JP glyphs (162..171).
        let s = format_integer(7, 2, PrintingMode::RightAlign, Charset::Jp);
        assert_eq!(s.units(), &[CHAR_JP_SPACE, CHAR_JP_0 + 7][..]);

        // The i32::MIN mirror: wrapping u32 math, hyphen + the C's
        // (u16-truncated) digits. 2147483648 with width 10: the first
        // dividend yields digit 2, the rest follow.
        let s = format_integer(i32::MIN, 10, PrintingMode::LeadingZeros, En);
        assert_eq!(s.to_text(), "-2147483648");

        // Out-of-range width panics (the game asserts).
        let result = std::panic::catch_unwind(|| format_integer(1, 0, PrintingMode::LeftAlign, En));
        assert!(result.is_err());
        let result =
            std::panic::catch_unwind(|| format_integer(1, 11, PrintingMode::LeftAlign, En));
        assert!(result.is_err());
    }

    #[test]
    fn trname_pack_unpack_round_trips_both_terminator_styles() {
        // Retail bank 246, "Don": the packed stream reads the message
        // EOS unit for its terminator bits (saturated tail).
        let don = en("Don");
        let packed = pack_trainer_name(&don).expect("in-range chars");
        assert_eq!(packed, [0x272E, 0x7A95], "matches the retail units");
        // 3 chars = 27 bits = one flushed unit + a 12-bit tail, which
        // the packer saturates into the second unit.
        let mut message = vec![TRNAMECODE];
        message.extend_from_slice(&packed);
        message.push(EOS);
        let (chars, consumed) =
            unpack_trainer_name(&message).expect("retail-shaped block unpacks");
        assert_eq!(chars, don);
        // The terminator straddle reads the EOS unit (index 3), so the
        // stream's last touched index is the EOS itself — the block runs
        // to the very end of its message.
        assert_eq!(consumed, 3, "stream touched the EOS unit");
        assert_eq!(message.len(), consumed + 1, "block ends the message");

        // 5 chars = 45 bits = exactly 3 units: no terminator unit is
        // emitted; the unpacker reads the message EOS.
        let five = en("Abcde");
        let packed = pack_trainer_name(&five).expect("in-range chars");
        assert_eq!(packed.len(), 3);
        let mut message = vec![TRNAMECODE];
        message.extend_from_slice(&packed);
        message.push(EOS);
        let (chars, consumed) =
            unpack_trainer_name(&message).expect("boundary-length name unpacks");
        assert_eq!(chars, five);
        // 45 bits = 3 exact units; the terminator read starts on the EOS
        // unit at bit offset 0, so the stream's last touched index is 4
        // — the EOS unit itself.
        assert_eq!(consumed, 4, "stream read the EOS unit as terminator");

        // The marker is required, and oversize chars are rejected.
        assert_eq!(
            unpack_trainer_name(&en("Don")).unwrap_err(),
            TextError::Invalid {
                what: "trainer name must start with the 0xF100 marker"
            }
        );
        assert!(pack_trainer_name(&[0x200]).is_err());

        // A stream that runs past its input without terminating errors.
        assert_eq!(
            unpack_trainer_name(&[TRNAMECODE, 0x0000]).unwrap_err(),
            TextError::TrnameOverrun
        );

        // pack(unpack(x)) == x over the boundary-length cases.
        for len in 1..=12 {
            let name: Vec<u16> = (0..len).map(|i| CHAR_a + u16::try_from(i % 26).unwrap()).collect();
            let packed = pack_trainer_name(&name).expect("in-range chars");
            let mut message = vec![TRNAMECODE];
            message.extend_from_slice(&packed);
            message.push(EOS);
            let (chars, _) = unpack_trainer_name(&message).expect("round-trip case unpacks");
            assert_eq!(chars, name, "{len}-char name round-trips");
        }
    }

    #[test]
    fn cat_handle_trainer_name_unpacks_in_place() {
        // A bank-729-style field string is the marker + packed chunks.
        let don = en("Don");
        let mut field = GameString::new();
        field.push_char(TRNAMECODE);
        for &u in &pack_trainer_name(&don).expect("in-range chars") {
            field.push_char(u);
        }
        assert!(field.is_trainer_name());

        let mut dest = GameString::new();
        for &u in &en("Hi ") {
            dest.push_char(u);
        }
        dest.cat_handle_trainer_name(&field).expect("unpacks");
        assert_eq!(dest.to_text(), "Hi Don");

        // Plain strings cat as-is.
        let mut dest2 = GameString::new();
        dest2.cat_handle_trainer_name(&GameString::from_units(&en("XY"))).expect("cats");
        assert_eq!(dest2.to_text(), "XY");
    }

    #[test]
    fn lossy_rendering_names_control_codes() {
        // 0xFFFE blocks render by pret command name (or raw), LF as
        // '\n'; a malformed tail renders U+FFFD and stops.
        let mut s = GameString::new();
        for &u in &en("A") {
            s.push_char(u);
        }
        s.push_char(0xFFFE);
        s.push_char(0xFF00);
        s.push_char(1);
        s.push_char(0x0003); // {COLOR 3}
        s.push_char(CHAR_LF);
        s.push_char(CHAR_a); // 'a' (0x145; 0x12B is 'A')
        assert_eq!(s.to_text(), "A{COLOR 3}\na");

        let truncated = GameString::from_units(&[0xFFFE, 0xFF00, 1]);
        assert!(truncated.to_text().ends_with('\u{fffd}'));
        assert_eq!(parse_ext_ctrl(&[0xFFFE, 0xFF00, 1]).unwrap_err(), {
            let _ = EXT_CTRL_CODE_BEGIN;
            TextError::CtrlOverrun {
                code: 0xFF00,
                size: 1
            }
        });
        assert_eq!(char_of(0x12B), Some('A'), "0x12B is 'A', not 'a'");
    }
}