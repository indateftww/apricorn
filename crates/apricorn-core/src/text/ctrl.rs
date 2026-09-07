//! The extended-control-code walker — a port of pret's
//! `string_control_code.c` and `constants/charcode.h`.
//!
//! Control codes are carried by `0xFFFE` blocks:
//!
//! ```text
//! 0xFFFE  code  size  size × u16 fields
//! ```
//!
//! `code`'s high byte classifies the block; four classes are *string
//! variables* — placeholders the [`MessageFormat`](crate::text::format::MessageFormat)
//! substitutes at display time ([`is_strvar_code`]). Retail HeartGold
//! (US) uses 60 distinct codes with sizes 0–2 only; the buffer index
//! lives in `field[0]`, and the code's *low byte is a tooling tag, not
//! the index* — the game only ever reads `field[0]` (bank 0, message 10
//! uses one code with two different field values).

use crate::text::TextError;

/// `0xFFFE` — an extended control-code block begins here
/// (`EXT_CTRL_CODE_BEGIN`).
pub const EXT_CTRL_CODE_BEGIN: u16 = 0xFFFE;

/// `0xE000` — linefeed (`CHAR_LF`).
pub const CHAR_LF: u16 = 0xE000;

/// `0xF100` — a packed trainer-name block begins here (`TRNAMECODE`).
pub const TRNAMECODE: u16 = 0xF100;

/// `0x1FF` — the 9-bit mask of packed trainer-name chars (`TRNAME_MASK`).
pub const TRNAME_MASK: u16 = 0x1FF;

/// `0x1FF` — the packed trainer-name stream's terminator (`EOS_TRNAME`).
pub const EOS_TRNAME: u16 = 0x1FF;

/// The strvar code classes: a block whose `code & 0xFF00` is one of
/// these is a string variable ([`is_strvar_code`]). From pret's
/// `MsgArray_ControlCodeIsStrVar` — note `0x3400`, not `0x0400`-style
/// shorthand: the mask keeps the full high byte.
pub const STRVAR_CLASSES: [u16; 4] = [0x0100, 0x0300, 0x0400, 0x3400];

/// A parsed `0xFFFE` block: the control code plus its `size` field
/// units (`fields.len()` is the block's `size`).
///
/// Borrows the fields from the message being walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtCtrl<'a> {
    /// The control code (`arr[1]`); its high byte classifies the block.
    pub code: u16,
    /// The block's field units (`arr[3..3 + size]`).
    pub fields: &'a [u16],
}

/// Whether `code` marks a string-variable block
/// (`MsgArray_ControlCodeIsStrVar`).
#[must_use]
pub fn is_strvar_code(code: u16) -> bool {
    STRVAR_CLASSES.contains(&(code & 0xFF00))
}

/// Parses the `0xFFFE` block at `units[0]`, returning it and the units
/// it consumes (`3 + size`).
///
/// The game's walker (`MsgArray_SkipControlCode`) just skips past the
/// block; this port also rejects a block whose fields would run past
/// the end of the slice, where the C would read past its message.
///
/// # Errors
/// [`TextError::Invalid`] if `units[0]` is not the begin marker or the
/// header is truncated; [`TextError::CtrlOverrun`] if the declared
/// size overruns the slice.
pub fn parse_ext_ctrl(units: &[u16]) -> Result<(ExtCtrl<'_>, usize), TextError> {
    if units.first() != Some(&EXT_CTRL_CODE_BEGIN) {
        return Err(TextError::Invalid {
            what: "ext control code must start with 0xFFFE",
        });
    }
    if units.len() < 3 {
        return Err(TextError::Invalid {
            what: "ext control code header is truncated",
        });
    }
    let code = units[1];
    let size = units[2];
    if units.len() < 3 + usize::from(size) {
        return Err(TextError::CtrlOverrun { code, size });
    }
    Ok((
        ExtCtrl {
            code,
            fields: &units[3..3 + usize::from(size)],
        },
        3 + usize::from(size),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strvar_classes_match_pret() {
        // The four classes, by their retail representatives.
        assert!(is_strvar_code(0x0101));
        assert!(is_strvar_code(0x0300));
        assert!(is_strvar_code(0x0400));
        assert!(is_strvar_code(0x3400));
        // Non-strvar control codes.
        assert!(!is_strvar_code(0x0200)); // {YESNO}
        assert!(!is_strvar_code(0xFF00)); // {COLOR}
        assert!(!is_strvar_code(0xFF01)); // {SIZE}
        assert!(!is_strvar_code(0x0205)); // {ALN_CENTER}
    }

    #[test]
    fn parses_blocks_of_every_retail_size() {
        // size 0: 0xFFFE 0x0205 0
        let (ctrl, len) = parse_ext_ctrl(&[0xFFFE, 0x0205, 0]).expect("size-0 block");
        assert_eq!(ctrl.code, 0x0205);
        assert_eq!(ctrl.fields, &[]);
        assert_eq!(len, 3);

        // size 1: 0xFFFE 0xFF00 1 <color>
        let (ctrl, len) =
            parse_ext_ctrl(&[0xFFFE, 0xFF00, 1, 0x0003]).expect("size-1 block");
        assert_eq!(ctrl.code, 0xFF00);
        assert_eq!(ctrl.fields, &[0x0003]);
        assert_eq!(len, 4);

        // size 2: 0xFFFE 0x0101 2 <fieldno> <unused>
        let (ctrl, len) =
            parse_ext_ctrl(&[0xFFFE, 0x0101, 2, 0, 0]).expect("size-2 block");
        assert!(is_strvar_code(ctrl.code));
        assert_eq!(ctrl.fields, &[0, 0]);
        assert_eq!(len, 5);
    }

    #[test]
    fn rejects_broken_blocks() {
        // Not a control code.
        assert!(parse_ext_ctrl(&[0x12B, 0x0205, 0]).is_err());
        assert!(parse_ext_ctrl(&[]).is_err());
        // Truncated header.
        assert!(parse_ext_ctrl(&[0xFFFE, 0x0205]).is_err());
        // Declared size overruns the slice.
        assert!(parse_ext_ctrl(&[0xFFFE, 0x0101, 2, 0]).is_err());
        let err = parse_ext_ctrl(&[0xFFFE, 0xFF00, 1]).expect_err("must overrun");
        assert_eq!(
            err,
            TextError::CtrlOverrun {
                code: 0xFF00,
                size: 1
            }
        );
    }
}