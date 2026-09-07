//! The game's variable formatting — a port of pret's `message_format.c`.
//!
//! A [`MessageFormat`] is `count` placeholder fields plus the expansion
//! walk (`StringExpandPlaceholders`): strvar control-code blocks in a
//! message are replaced by their bound field strings (through
//! [`GameString::cat_handle_trainer_name`], so a packed trainer-name
//! field unpacks in place); every other control-code block is copied
//! through verbatim; plain units pass through.
//!
//! The game's `Buffer*` helpers (`BufferSpeciesName`,
//! `BufferItemName`, ...) are all `ReadMsgDataIntoString` into a field
//! — [`MessageFormat::set_message`] is their mechanical core, and the
//! specialized wrappers arrive with the game-state machine once the
//! bank registry exists. Retail banks address fields up to index 18,
//! so the field count is dynamic (the game's default constructor uses
//! 8).

use crate::formats::{EOS, MsgBank};
use crate::text::ctrl::{EXT_CTRL_CODE_BEGIN, is_strvar_code, parse_ext_ctrl};
use crate::text::string::{Charset, GameString, PrintingMode, format_integer};
use crate::text::TextError;

/// pret's `MessageFormat`: `count` string fields bound to placeholder
/// positions, expanded into a [`GameString`].
#[derive(Debug, Clone, Default)]
pub struct MessageFormat {
    fields: Vec<GameString>,
}

impl MessageFormat {
    /// `MessageFormat_New_Custom(nstr, ...)` — the game's default
    /// `MessageFormat_New` is `new(8)`. Retail banks reference field
    /// indices up to 18.
    ///
    /// # Panics
    /// Panics if `count` is 0 (the game `GF_ASSERT`s `nstr != 0`).
    #[must_use]
    pub fn new(count: usize) -> Self {
        assert!(count != 0, "MessageFormat_New_Custom: nstr != 0");
        Self {
            fields: vec![GameString::new(); count],
        }
    }

    /// The number of placeholder fields (`count`).
    #[must_use]
    pub fn count(&self) -> usize {
        self.fields.len()
    }

    /// `SetStringAsPlaceholder` — binds `string` to field `fieldno`.
    ///
    /// # Panics
    /// Panics if `fieldno` is out of range (the game `GF_ASSERT`s).
    pub fn set_string(&mut self, fieldno: usize, string: &GameString) {
        assert!(
            fieldno < self.fields.len(),
            "SetStringAsPlaceholder: field {fieldno} out of range"
        );
        self.fields[fieldno] = string.clone();
    }

    /// `ReadMsgDataIntoString` + `SetStringAsPlaceholder` — binds bank
    /// message `id` to field `fieldno`. This is the body of every
    /// `Buffer*Name` helper in `message_format.c`.
    ///
    /// # Panics
    /// Panics if `fieldno` is out of range or the bank has no message
    /// `id` (the game `GF_ASSERT`s both).
    pub fn set_message(&mut self, fieldno: usize, bank: &MsgBank<'_>, id: usize) {
        let Some(units) = bank.message(id) else {
            panic!("ReadMsgDataIntoString: bank has no message {id}");
        };
        self.set_string(fieldno, &GameString::from_units(units));
    }

    /// `BufferIntegerAsString` — formats `num` and binds it to field
    /// `fieldno` ([`format_integer`]).
    ///
    /// # Panics
    /// Panics on the same conditions as [`format_integer`] plus an
    /// out-of-range `fieldno`.
    pub fn buffer_integer(
        &mut self,
        fieldno: usize,
        num: i32,
        ndigits: u32,
        mode: PrintingMode,
        charset: Charset,
    ) {
        self.set_string(fieldno, &format_integer(num, ndigits, mode, charset));
    }

    /// The string bound to field `fieldno`, if any.
    #[must_use]
    pub fn field(&self, fieldno: usize) -> Option<&GameString> {
        self.fields.get(fieldno)
    }

    /// `MessageFormat_ResetBuffers` — empties every field.
    pub fn reset(&mut self) {
        for field in &mut self.fields {
            field.set_empty();
        }
    }

    /// `MessageFormat_UpperFirstChar` — uppercases the first unit of
    /// field `fieldno`.
    ///
    /// # Panics
    /// Panics if `fieldno` is out of range.
    pub fn upper_first_char(&mut self, fieldno: usize) {
        assert!(
            fieldno < self.fields.len(),
            "MessageFormat_UpperFirstChar: field {fieldno} out of range"
        );
        self.fields[fieldno].upper_char_n(0);
    }

    /// `StringExpandPlaceholders` — expands `src` (walked to its EOS
    /// or the slice end) into a new string, substituting strvar blocks
    /// with their bound fields.
    ///
    /// The strvar's buffer index is `field[0]` — the code's low byte
    /// is a tooling tag the game never reads (bank 0, message 10 uses
    /// one code with two different field values).
    ///
    /// # Errors
    /// [`TextError::NoSuchField`] if a strvar's `field[0]` is beyond
    /// this format's fields (the game `GF_ASSERT`s there instead);
    /// [`TextError::Invalid`] if a strvar block carries no field
    /// units; the parse errors for any malformed control-code block.
    pub fn expand_placeholders(&self, src: &[u16]) -> Result<GameString, TextError> {
        let mut dest = GameString::new();
        let mut i = 0;
        while i < src.len() && src[i] != EOS {
            if src[i] == EXT_CTRL_CODE_BEGIN {
                let (ctrl, len) = parse_ext_ctrl(&src[i..])?;
                if is_strvar_code(ctrl.code) {
                    let Some(&fieldno) = ctrl.fields.first() else {
                        return Err(TextError::Invalid {
                            what: "strvar block has no field units",
                        });
                    };
                    let Some(field) = self.fields.get(usize::from(fieldno)) else {
                        return Err(TextError::NoSuchField { fieldno });
                    };
                    dest.cat_handle_trainer_name(field)?;
                } else {
                    for &unit in &src[i..i + len] {
                        dest.push_char(unit);
                    }
                }
                i += len;
            } else {
                dest.push_char(src[i]);
                i += 1;
            }
        }
        Ok(dest)
    }

    /// `ReadMsgData_ExpandPlaceholders` — expands bank message `id`.
    ///
    /// # Panics
    /// Panics if the bank has no message `id` (the game asserts).
    ///
    /// # Errors
    /// As [`MessageFormat::expand_placeholders`].
    pub fn expand_message(
        &self,
        bank: &MsgBank<'_>,
        id: usize,
    ) -> Result<GameString, TextError> {
        let Some(units) = bank.message(id) else {
            panic!("ReadMsgData_ExpandPlaceholders: bank has no message {id}");
        };
        self.expand_placeholders(units)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EN letters/digits/space as code units (`charcode.h`: `CHAR_A`=299,
    /// `CHAR_a`=325, `CHAR_0`=289, space glyph 478, `CHAR_EXCL`=427).
    fn en(s: &str) -> Vec<u16> {
        s.chars()
            .map(|c| match c {
                'a'..='z' => 325 + (c as u16 - u16::from(b'a')),
                'A'..='Z' => 299 + (c as u16 - u16::from(b'A')),
                '0'..='9' => 289 + (c as u16 - u16::from(b'0')),
                ' ' => 478,
                '!' => 427,
                _ => panic!("test helper: unexpected char {c:?}"),
            })
            .collect()
    }

    /// The exact retail shape of bank 3, message 3 (Pound's usage
    /// line): `<strvar field 0> used\nPound!` — note there is no space
    /// between "used" and the linefeed.
    fn strvar_message() -> Vec<u16> {
        let mut units = vec![
            0xFFFE, 0x0101, 2, // strvar (STRVAR_1 class) reading field[0] = 0
            0x0000, 0x0000,
        ];
        units.extend_from_slice(&en(" used"));
        units.push(crate::text::ctrl::CHAR_LF);
        units.extend_from_slice(&en("Pound!"));
        units.push(EOS);
        units
    }

    #[test]
    fn expands_strvars_and_passes_ctrl_through() {
        let mut fmt = MessageFormat::new(8);
        let name = GameString::from_units(&en("PIKA"));
        fmt.set_string(0, &name);

        let expanded = fmt
            .expand_placeholders(&strvar_message())
            .expect("well-formed message");
        assert_eq!(expanded.to_text(), "PIKA used\nPound!");

        // reset + a rebound field.
        fmt.reset();
        assert!(fmt.field(0).expect("field 0").is_empty());
        fmt.buffer_integer(0, 5, 1, PrintingMode::LeadingZeros, Charset::En);
        assert_eq!(fmt.field(0).expect("field 0").to_text(), "5");
        let expanded = fmt
            .expand_placeholders(&strvar_message())
            .expect("well-formed message");
        assert_eq!(expanded.to_text(), "5 used\nPound!");

        // UpperFirstChar uppercases the field, not the message.
        fmt.upper_first_char(0);
        assert_eq!(fmt.field(0).expect("field 0").to_text(), "5");
    }

    #[test]
    fn strvar_field_index_comes_from_field0_not_the_low_byte() {
        // One strvar code with two different field[0] values (the
        // retail bank 0 message 10 shape): the low byte is a tag.
        let units = [
            0xFFFE, 0x0133, 2, 4, 0, // code 0x0133, field[0] = 4
            0x12B, EOS, // 'A' (0x12B = 299 = CHAR_A)
        ];
        let mut fmt = MessageFormat::new(8);
        let mut field4 = GameString::new();
        field4.push_char(299); // 'A'
        fmt.set_string(4, &field4);
        let expanded = fmt.expand_placeholders(&units).expect("field 4 bound");
        assert_eq!(expanded.to_text(), "AA");
    }

    #[test]
    fn out_of_range_fields_error_like_the_game_asserts() {
        let fmt = MessageFormat::new(8);
        let units = [0xFFFE, 0x0101, 2, 8, 0, EOS]; // field[0] = 8
        assert_eq!(
            fmt.expand_placeholders(&units).unwrap_err(),
            TextError::NoSuchField { fieldno: 8 }
        );
        let units = [0xFFFE, 0x0101, 0, EOS]; // size 0: no field units
        assert_eq!(
            fmt.expand_placeholders(&units).unwrap_err(),
            TextError::Invalid {
                what: "strvar block has no field units"
            }
        );
    }

    #[test]
    fn non_strvar_ctrl_blocks_copy_through_verbatim() {
        let units = [
            0xFFFE, 0xFF00, 1, 0x0003, // {COLOR 3}
            0x12B, // 'A'
            EOS,
        ];
        let fmt = MessageFormat::new(8);
        let expanded = fmt.expand_placeholders(&units).expect("passes through");
        // The whole block is in the output, units intact.
        assert_eq!(expanded.units(), &units[..units.len() - 1]);
    }

    #[test]
    fn packed_trainer_name_fields_unpack_into_expansions() {
        // A field bound straight from a bank-729-style message: marker
        // + packed chunks. Expansion unpacks it in place.
        let mut packed = GameString::new();
        packed.push_char(crate::text::ctrl::TRNAMECODE);
        for &u in &crate::text::string::pack_trainer_name(&[302, 339, 338]).expect("9-bit") {
            packed.push_char(u);
        }

        let mut fmt = MessageFormat::new(8);
        fmt.set_string(0, &packed);
        // 0x1AB = 427 = CHAR_EXCL.
        let units = [0xFFFE, 0x0101, 2, 0, 0, 0x1AB, EOS]; // <strvar 0>!
        let expanded = fmt.expand_placeholders(&units).expect("unpacks");
        assert_eq!(expanded.to_text(), "Don!");
    }

    #[test]
    fn constructor_and_accessors() {
        assert_eq!(MessageFormat::new(1).count(), 1);
        assert_eq!(MessageFormat::new(19).count(), 19);
        assert!(MessageFormat::new(8).field(7).is_some());
        assert!(MessageFormat::new(8).field(8).is_none());
        assert!(std::panic::catch_unwind(|| MessageFormat::new(0)).is_err());
        let mut fmt = MessageFormat::new(8);
        let mut fmt = std::panic::AssertUnwindSafe(&mut fmt);
        assert!(
            std::panic::catch_unwind(move || fmt.set_string(8, &GameString::new())).is_err()
        );
    }
}