//! The text layer: decoding MAT message units against the generation
//! charmap, and the game's variable formatting (PLAN.md Phase 4,
//! step 2).
//!
//! [`formats::msg`](crate::formats::msg)'s `MsgBank` decrypts a bank
//! into raw u16 code units; this module turns those units into text:
//!
//! * [`charmap`] — the committed generation charmap (`charmap.txt`)
//!   and its lookups.
//! * [`ctrl`] — the `0xFFFE` extended-control-code walker (pret's
//!   `string_control_code.c`).
//! * [`decode`] — a strict walk over one message producing a
//!   [`DecodedMessage`]: every unit must be a charmap character, a
//!   well-formed control-code block, or a packed trainer name. The
//!   strictness *is* the validation: every one of the 49,984 retail
//!   messages decodes with zero violations
//!   (`tests/text_hg.rs` pins the census).
//! * [`string`] — the game's `String` (`pm_string.c`):
//!   [`GameString`], `String16_FormatInteger`, and TRNAME 9-bit
//!   packed-name unpack/pack.
//! * [`format`] — the game's variable formatting (`message_format.c`):
//!   [`MessageFormat`] placeholder expansion.
//!
//! Behavioral parity lives in u16 code-unit space — the game's logic
//! operates on units, so [`GameString`] and every port here do too.
//! Unicode output (`to_text`) is a **lossy debug rendering** through
//! the charmap, not a serialization; the pret-text format is tooling,
//! not engine, and is deferred.
//!
//! **The retail image is the oracle.** Every invariant this module
//! enforces at decode time (charmap coverage, control-code framing,
//! TRNAME termination and message-finality) was verified across all
//! 49,984 messages of `a/0/2/7` before becoming a parse-time check.

pub mod charmap;
pub mod ctrl;
pub mod format;
pub mod string;

use crate::formats::EOS;
use ctrl::{EXT_CTRL_CODE_BEGIN, TRNAMECODE, is_strvar_code, parse_ext_ctrl};
use charmap::{char_of, command_name};
use string::unpack_trainer_name;

/// Failures while decoding or formatting text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextError {
    /// A structural requirement the retail image always satisfies.
    Invalid {
        /// What was malformed.
        what: &'static str,
    },
    /// A `0xFFFE` block's declared size overruns its message.
    CtrlOverrun {
        /// The block's control code.
        code: u16,
        /// The block's declared size.
        size: u16,
    },
    /// A unit that is neither special nor in the charmap.
    UnmappedUnit {
        /// The offending code unit.
        unit: u16,
    },
    /// A packed trainer-name stream ran past its input without the
    /// `0x1FF` terminator.
    TrnameOverrun,
    /// A strvar referenced a field the [`format::MessageFormat`] does
    /// not have.
    NoSuchField {
        /// The strvar's `field[0]`.
        fieldno: u16,
    },
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Invalid { what } => write!(f, "invalid text: {what}"),
            Self::CtrlOverrun { code, size } => {
                write!(f, "ext control code 0x{code:04X} (size {size}) overruns its message")
            }
            Self::UnmappedUnit { unit } => write!(f, "unmapped code unit 0x{unit:04X}"),
            Self::TrnameOverrun => write!(f, "packed trainer name never terminates"),
            Self::NoSuchField { fieldno } => write!(f, "no placeholder field {fieldno}"),
        }
    }
}

impl std::error::Error for TextError {}

/// One decoded element of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token<'a> {
    /// A plain character unit present in the charmap (including
    /// `0xE000` LF, which renders as `'\n'`).
    Char {
        /// The source code unit.
        unit: u16,
    },
    /// A `0xFFFE` extended control-code block.
    Ctrl {
        /// The block's control code.
        code: u16,
        /// The block's field units (`fields.len()` is its size).
        fields: &'a [u16],
    },
    /// A `0xF100` packed trainer-name block: the unpacked characters
    /// plus the raw source span (marker and packed units, without the
    /// message EOS).
    TrainerName {
        /// The unpacked character units.
        chars: Vec<u16>,
        /// The raw span, from the `0xF100` marker to the block's end.
        units: &'a [u16],
    },
}

/// A strictly decoded message: its tokens, plus the source units the
/// tokens were decoded from (EOS included).
///
/// Borrows the source slice — a `DecodedMessage` cannot outlive its
/// [`MsgBank`](crate::formats::MsgBank).
#[derive(Debug)]
pub struct DecodedMessage<'a> {
    /// The source units, exactly as passed to [`decode`] (with EOS).
    units: &'a [u16],
    /// The decoded elements, in source order.
    tokens: Vec<Token<'a>>,
}

impl<'a> DecodedMessage<'a> {
    /// The decoded elements, in source order.
    #[must_use]
    pub fn tokens(&self) -> &[Token<'a>] {
        &self.tokens
    }

    /// The source units (EOS included).
    #[must_use]
    pub fn source(&self) -> &'a [u16] {
        self.units
    }

    /// Reassembles the tokens into code units — byte-identical to the
    /// source (control blocks and TRNAME spans copy through raw, and
    /// the trailing EOS is re-appended). The round-trip half of the
    /// decoder guard: `decode` then `to_units` is the identity.
    #[must_use]
    pub fn to_units(&self) -> Vec<u16> {
        let mut out = Vec::with_capacity(self.units.len());
        for token in &self.tokens {
            match *token {
                Token::Char { unit } => out.push(unit),
                Token::Ctrl { code, fields } => {
                    out.push(EXT_CTRL_CODE_BEGIN);
                    out.push(code);
                    out.push(fields.len() as u16);
                    out.extend_from_slice(fields);
                }
                Token::TrainerName { units, .. } => out.extend_from_slice(units),
            }
        }
        out.push(EOS);
        out
    }

    /// Lossy debug rendering: chars through the charmap (`'\n'` for
    /// LF), strvars as `{STRVAR#N}`, other control codes by pret
    /// command name (`{COLOR 3}`) or raw (`{0x0207}`), packed trainer
    /// names as the unpacked name in brackets (`[Don]`).
    ///
    /// Debug-only output — not the pret-text serialization.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for token in &self.tokens {
            match *token {
                Token::Char { unit } => out.push(char_of(unit).unwrap_or('\u{fffd}')),
                Token::Ctrl { code, fields } => render_ctrl(code, fields, &mut out),
                Token::TrainerName { ref chars, .. } => {
                    out.push('[');
                    for &unit in chars {
                        out.push(char_of(unit).unwrap_or('\u{fffd}'));
                    }
                    out.push(']');
                }
            }
        }
        out
    }
}

/// Strictly decodes one MAT message (with its trailing EOS).
///
/// Every unit must be a charmap character, a well-formed `0xFFFE`
/// control-code block, or a packed trainer-name block whose 9-bit
/// stream terminates; a TRNAME block must end its message (all retail
/// blocks do). Nothing is dropped or guessed — decode either yields
/// every element or names the exact defect.
///
/// # Errors
/// [`TextError::Invalid`] if the message does not end with EOS or a
/// TRNAME block is not message-final; [`TextError::CtrlOverrun`] for a
/// block overrunning its message; [`TextError::UnmappedUnit`] for a
/// plain unit outside the charmap. [`TextError::TrnameOverrun`] is
/// propagated defensively, but is unreachable here: every read window
/// over an EOS-terminated message's all-ones final unit yields the TRNAME
/// terminator, so a TRNAME stream always terminates.
pub fn decode<'a>(units: &'a [u16]) -> Result<DecodedMessage<'a>, TextError> {
    if units.last() != Some(&EOS) {
        return Err(TextError::Invalid {
            what: "message does not end with EOS",
        });
    }
    let end = units.len() - 1;
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < end {
        let unit = units[i];
        if unit == EXT_CTRL_CODE_BEGIN {
            let (ctrl, len) = parse_ext_ctrl(&units[i..end])?;
            tokens.push(Token::Ctrl {
                code: ctrl.code,
                fields: ctrl.fields,
            });
            i += len;
        } else if unit == TRNAMECODE {
            let (chars, consumed) = unpack_trainer_name(&units[i..])?;
            // consumed is the walker's terminator index relative to
            // the marker; the block must run to the message end —
            // the EOS-spanning terminator style depends on it.
            let next = i + 1 + consumed;
            if next < end {
                return Err(TextError::Invalid {
                    what: "trainer name block does not end its message",
                });
            }
            tokens.push(Token::TrainerName {
                chars,
                units: &units[i..end],
            });
            i = next;
        } else if char_of(unit).is_some() {
            tokens.push(Token::Char { unit });
            i += 1;
        } else {
            return Err(TextError::UnmappedUnit { unit });
        }
    }
    Ok(DecodedMessage { units, tokens })
}

/// Renders one control-code block for the lossy debug output.
fn render_ctrl(code: u16, fields: &[u16], out: &mut String) {
    if is_strvar_code(code) {
        let fieldno = fields.first().copied().unwrap_or(0);
        out.push_str(&format!("{{STRVAR#{fieldno}}}"));
    } else if let Some(name) = command_name(code) {
        out.push('{');
        out.push_str(name);
        for &field in fields {
            out.push_str(&format!(" {field}"));
        }
        out.push('}');
    } else {
        out.push_str(&format!("{{0x{code:04X}"));
        for &field in fields {
            out.push_str(&format!(" {field}"));
        }
        out.push('}');
    }
}

/// Renders raw code units lossily for debug output ([`GameString::to_text`]):
/// chars through the charmap, control codes by name, TRNAME unpacked
/// in brackets. Malformed tails render U+FFFD and stop.
pub(crate) fn render_lossy_units(units: &[u16]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < units.len() && units[i] != EOS {
        let unit = units[i];
        if unit == EXT_CTRL_CODE_BEGIN {
            match parse_ext_ctrl(&units[i..]) {
                Ok((ctrl, len)) => {
                    render_ctrl(ctrl.code, ctrl.fields, &mut out);
                    i += len;
                }
                Err(_) => {
                    out.push('\u{fffd}');
                    break;
                }
            }
        } else if unit == TRNAMECODE {
            // A GameString carries no EOS, but the C's invariant
            // `data[size] == EOS` means the stream may legitimately
            // need that all-ones unit for its terminator — retry with
            // one appended before calling the stream malformed.
            let unpacked = unpack_trainer_name(&units[i..]).or_else(|_| {
                let mut tail = units[i..].to_vec();
                tail.push(EOS);
                unpack_trainer_name(&tail)
            });
            match unpacked {
                Ok((chars, consumed)) => {
                    out.push('[');
                    for &u in &chars {
                        out.push(char_of(u).unwrap_or('\u{fffd}'));
                    }
                    out.push(']');
                    i += consumed + 1;
                }
                Err(_) => {
                    out.push('\u{fffd}');
                    break;
                }
            }
        } else {
            out.push(char_of(unit).unwrap_or('\u{fffd}'));
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `charcode.h`-derived EN letters.
    fn en(s: &str) -> Vec<u16> {
        s.chars()
            .map(|c| match c {
                'a'..='z' => 325 + (c as u16 - u16::from(b'a')),
                'A'..='Z' => 299 + (c as u16 - u16::from(b'A')),
                '!' => 427,
                _ => panic!("test helper: unexpected char {c:?}"),
            })
            .collect()
    }

    #[test]
    fn decodes_every_token_kind_and_round_trips() {
        // A message with one of everything: text, LF, a ctrl block,
        // and an EOS-spanning TRNAME ("Don" from bank 246).
        let mut units = en("A");
        units.push(ctrl::CHAR_LF);
        units.extend_from_slice(&en("b!"));
        units.extend_from_slice(&[0xFFFE, 0xFF00, 1, 0x0003]); // {COLOR 3}
        units.push(TRNAMECODE);
        units.extend_from_slice(&[0x272E, 0x7A95]); // packed "Don"
        units.push(EOS);

        let decoded = decode(&units).expect("every element is valid");
        // A, LF, b, !, ctrl, TRNAME — "b!" is two Char tokens.
        assert_eq!(decoded.tokens().len(), 6);
        assert_eq!(decoded.source(), &units[..]);
        assert_eq!(
            decoded.tokens()[0],
            Token::Char {
                unit: en("A")[0]
            }
        );
        assert_eq!(
            decoded.tokens()[3..5],
            [
                Token::Char {
                    unit: en("!")[0]
                },
                Token::Ctrl {
                    code: 0xFF00,
                    fields: &[0x0003][..]
                },
            ]
        );
        match decoded.tokens()[5] {
            Token::TrainerName { ref chars, units } => {
                assert_eq!(chars, &en("Don"));
                assert_eq!(units, &[TRNAMECODE, 0x272E, 0x7A95][..]);
            }
            ref other => panic!("expected a trainer name, got {other:?}"),
        }

        // Byte-identity: reassembly is the source, exactly.
        assert_eq!(decoded.to_units(), units);
        assert_eq!(decoded.to_text(), "A\nb!{COLOR 3}[Don]");
    }

    #[test]
    fn rejects_each_defect_by_name() {
        use TextError::*;

        // No trailing EOS.
        assert_eq!(
            decode(&en("A")).unwrap_err(),
            Invalid {
                what: "message does not end with EOS"
            }
        );

        // A ctrl block overrunning the message.
        let units = [0xFFFE, 0xFF00, 2, 0x0003, EOS];
        assert_eq!(
            decode(&units).unwrap_err(),
            CtrlOverrun {
                code: 0xFF00,
                size: 2
            }
        );

        // A plain unit outside the charmap (0xFFFF mid-message is not
        // mapped; nothing in 0xE001..0xEFFF or 0xF101..0xFFFD is).
        assert_eq!(
            decode(&[0xE001, EOS]).unwrap_err(),
            UnmappedUnit { unit: 0xE001 }
        );
        assert_eq!(
            decode(&[0xFFFF, EOS]).unwrap_err(),
            UnmappedUnit { unit: 0xFFFF }
        );

        // A TRNAME stream that never terminates is only reachable on
        // input without the trailing EOS: in any EOS-terminated message
        // every read window over the all-ones EOS unit yields the
        // 0x1FF terminator, so decode can never overrun a TRNAME
        // (string.rs pins the overrun itself on a truncated stream).

        // A mid-message TRNAME block: retail blocks always run to the
        // end of their message. The fixture needs a name whose
        // terminator read ends exactly on a unit boundary (4 chars =
        // 36 bits: the walker advances past the saturated tail unit
        // before finding the terminator, so `consumed` is one past
        // the last touched unit) — shorter, EOS-straddling names
        // instead swallow a single trailing unit into the terminator.
        // Two trailing units leave the block one short of the end.
        let mut units = vec![TRNAMECODE];
        units.extend_from_slice(&string::pack_trainer_name(&en("Blue")).expect("9-bit"));
        units.extend_from_slice(&en("AB"));
        units.push(EOS);
        assert_eq!(
            decode(&units).unwrap_err(),
            Invalid {
                what: "trainer name block does not end its message"
            }
        );
    }

    #[test]
    fn empty_message_decodes_to_itself() {
        let decoded = decode(&[EOS]).expect("empty message is valid");
        assert!(decoded.tokens().is_empty());
        assert_eq!(decoded.to_units(), &[EOS]);
        assert_eq!(decoded.to_text(), "");
    }
}