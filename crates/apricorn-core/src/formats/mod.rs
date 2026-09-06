//! NitroSDK file formats stored inside the ROM.
//!
//! The NitroFS is only a shell; the game's actual content lives in NARC
//! archives (see [`narc`]), which in turn hold the graphics/audio/text
//! formats (NCGR, NCLR, SDAT, …) that later modules parse.

pub mod narc;
pub mod ncgr;
pub mod nclr;
pub mod nscr;

pub use narc::{Narc, is_narc};
pub use ncgr::{CharMapping, Ncgr, is_ncgr};
pub use nclr::{Nclr, Pmcp, is_nclr};
pub use nscr::{Nscr, is_nscr};

use crate::nds::{NdsError, u16le, u32le};

/// A section of a Nitro container file (NCGR, NCLR, NSCR, …).
///
/// Every section begins with a reversed four-letter magic and a size that
/// counts the section *including* its 8-byte header — the same scheme as
/// the NARC chunks (see [`narc`]).
pub(crate) struct NitroSection {
    /// The section magic as it appears in the file, reversed (e.g.
    /// `RAHC` for a NCGR's CHAR section).
    pub magic: [u8; 4],
    /// Absolute offset of the section header within the file.
    pub offset: usize,
    /// Section size, counting its 8-byte header.
    pub size: usize,
}

/// Parses the shared Nitro container header and walks its section list.
///
/// All three graphics formats open with the same 0x10-byte header layout
/// as a NARC (magic, byte-order mark, version, file size, header size
/// 0x10, section count), followed by the sections themselves — but the
/// byte-order mark differs: the NNS graphics containers store 0xFEFF
/// (bytes `FF FE`), where a NARC stores 0xFFFE. This validates the
/// header and that the sections tile the file exactly.
///
/// # Errors
/// Returns an [`NdsError`] if the header or section list is truncated,
/// inconsistent, or does not cover the file exactly.
pub(crate) fn nitro_sections(
    data: &[u8],
    magic: &[u8; 4],
    max_sections: usize,
) -> Result<(u16, Vec<NitroSection>), NdsError> {
    if data.len() < 0x10 {
        return Err(NdsError::Truncated {
            what: "Nitro container header",
            need: 0x10,
            got: data.len(),
        });
    }
    if &data[0..4] != magic {
        return Err(NdsError::Invalid {
            what: "not the expected Nitro format",
        });
    }
    if u16le(data, 0x04)? != 0xFEFF {
        return Err(NdsError::Invalid {
            what: "Nitro byte-order mark",
        });
    }
    let version = u16le(data, 0x06)?;
    let file_size = u32le(data, 0x08)? as usize;
    if file_size != data.len() {
        return Err(NdsError::Invalid {
            what: "Nitro file size does not match its data",
        });
    }
    if u16le(data, 0x0C)? != 0x10 {
        return Err(NdsError::Invalid {
            what: "Nitro header size",
        });
    }
    let section_count = u16le(data, 0x0E)? as usize;
    if section_count == 0 || section_count > max_sections {
        return Err(NdsError::Invalid {
            what: "Nitro section count",
        });
    }

    let mut sections = Vec::with_capacity(section_count);
    let mut off = 0x10;
    for _ in 0..section_count {
        let head = data.get(off..off + 8).ok_or(NdsError::Truncated {
            what: "Nitro section header",
            need: off + 8,
            got: data.len(),
        })?;
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&head[0..4]);
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        if size < 8 || off.checked_add(size).is_none_or(|end| end > data.len()) {
            return Err(NdsError::Invalid {
                what: "Nitro section overruns the file",
            });
        }
        sections.push(NitroSection {
            magic,
            offset: off,
            size,
        });
        off += size;
    }
    if off != data.len() {
        return Err(NdsError::Invalid {
            what: "Nitro sections do not cover the whole file",
        });
    }
    Ok((version, sections))
}

/// A Nitro pixel format, from the SDK's `GXTexFmt` enum.
///
/// `GX_TEXFMT_PLTT16` (3) is 4-bit indexed color; `GX_TEXFMT_PLTT256`
/// (4) is 8-bit. These are the only two the 2D formats use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFmt {
    /// 4 bpp, 16-color palettes (`GX_TEXFMT_PLTT16`, raw value 3).
    Pltt16,
    /// 8 bpp, 256-color palettes (`GX_TEXFMT_PLTT256`, raw value 4).
    Pltt256,
}

impl PixelFmt {
    /// Decodes the raw `GXTexFmt` value stored in a CHAR or PLTT section.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any format other than PLTT16/PLTT256.
    pub(crate) fn from_raw(raw: u32) -> Result<Self, NdsError> {
        match raw {
            3 => Ok(Self::Pltt16),
            4 => Ok(Self::Pltt256),
            _ => Err(NdsError::Invalid {
                what: "unsupported GXTexFmt (only 4/8-bit indexed exist)",
            }),
        }
    }

    /// Bits per pixel: 4 or 8.
    #[must_use]
    pub fn bpp(self) -> u8 {
        match self {
            Self::Pltt16 => 4,
            Self::Pltt256 => 8,
        }
    }

    /// The number of bytes in one 8×8 tile.
    #[must_use]
    pub fn tile_size(self) -> usize {
        8 * 8 * usize::from(self.bpp()) / 8
    }
}
