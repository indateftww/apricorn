//! NDS ROM container: cartridge header, ARM9 overlay table, and the Nitro
//! filesystem (File Name Table + File Allocation Table).
//!
//! Field layout and algorithm notes were cross-validated against
//! `hg_usa.nds` (retail HeartGold US) and pret/pokeheartgold's build
//! inputs; see `docs/nds-container.md` for the worked example.

pub mod blz;
pub mod header;
pub mod nitrofs;
pub mod overlay;
pub mod rom;

pub use header::{ArmBinary, Header, Region, crc16_arc};
pub use nitrofs::{NitroDir, NitroFile, NitroFs};
pub use overlay::Overlay;
pub use rom::NdsRom;

use core::fmt;

/// Errors produced while parsing NDS ROM structures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NdsError {
    /// A structure extends beyond the end of the data provided.
    Truncated {
        /// What was being parsed.
        what: &'static str,
        /// Number of bytes needed.
        need: usize,
        /// Number of bytes available.
        got: usize,
    },
    /// A value is out of range or inconsistent with the rest of the image.
    Invalid {
        /// What was being parsed.
        what: &'static str,
    },
}

impl fmt::Display for NdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NdsError::Truncated { what, need, got } => {
                write!(f, "{what} is truncated: need {need} bytes, got {got}")
            }
            NdsError::Invalid { what } => write!(f, "{what} is invalid"),
        }
    }
}

impl std::error::Error for NdsError {}

/// Reads a little-endian `u16` at `offset`.
///
/// # Errors
/// Returns [`NdsError::Truncated`] if the slice is too short.
pub(crate) fn u16le(data: &[u8], offset: usize) -> Result<u16, NdsError> {
    let b = data
        .get(offset..offset + 2)
        .ok_or_else(|| NdsError::Truncated {
            what: "u16",
            need: offset + 2,
            got: data.len(),
        })?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

/// Reads a little-endian `u32` at `offset`.
///
/// # Errors
/// Returns [`NdsError::Truncated`] if the slice is too short.
pub(crate) fn u32le(data: &[u8], offset: usize) -> Result<u32, NdsError> {
    let b = data
        .get(offset..offset + 4)
        .ok_or_else(|| NdsError::Truncated {
            what: "u32",
            need: offset + 4,
            got: data.len(),
        })?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Borrows `size` bytes at `offset` from `data`.
///
/// # Errors
/// Returns [`NdsError::Truncated`] if the range falls outside `data`.
pub(crate) fn slice<'a>(
    data: &'a [u8],
    offset: u32,
    size: u32,
    what: &'static str,
) -> Result<&'a [u8], NdsError> {
    let start = offset as usize;
    let end = start.checked_add(size as usize).ok_or(NdsError::Invalid {
        what: "region end offset overflows",
    })?;
    data.get(start..end).ok_or(NdsError::Truncated {
        what,
        need: end,
        got: data.len(),
    })
}
