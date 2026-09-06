//! NitroSDK file formats stored inside the ROM.
//!
//! The NitroFS is only a shell; the game's actual content lives in NARC
//! archives (see [`narc`]), which in turn hold the graphics/audio/text
//! formats (NCGR, NCLR, NSCR, NCER, NANR, SDAT, …) that later modules
//! parse.

pub mod btx;
pub mod msg;
pub mod nanr;
pub mod narc;
pub mod ncer;
pub mod ncgr;
pub mod nclr;
pub mod nscr;
pub mod sdat;

pub use btx::{Btx, BtxPalette, BtxTexture, TexFmt, is_btx};
pub use msg::{EOS, MsgBank};
pub use nanr::{AnimElement, AnimResult, Nanr, PlayMode, Uaat, is_nanr};
pub use narc::{Narc, is_narc};
pub use ncer::{BoundingBox, Cell, CellMapping, Ncer, Ucat, VramTransfer, is_ncer};
pub use ncgr::{CharMapping, Ncgr, is_ncgr};
pub use nclr::{Nclr, Pmcp, is_nclr};
pub use nscr::{Nscr, is_nscr};
pub use sdat::{
    BankInfo, GroupItem, GroupItemKind, PlayerInfo, Sdat, SeqArcInfo, SseqInfo, StrmInfo, SwarInfo,
    is_sdat,
};

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

/// Parses the label table of an `LBAL` section (the NCER/NANR label bank).
///
/// The section body is unusual: a bare array of `u32` string offsets with
/// **no count header**, followed by NUL-terminated ASCII strings. The
/// offsets are relative to the *end* of the offset array itself, are
/// strictly increasing, and the first is always 0 — so the count is
/// derived by scanning while the values remain plausible (the game's own
/// loader does the same).
///
/// `off`/`size` locate the section (header included), as produced by
/// [`nitro_sections`].
pub(crate) fn nitro_labels(data: &[u8], off: usize, size: usize) -> Result<Vec<&str>, NdsError> {
    let body = data.get(off + 8..off + size).ok_or(NdsError::Truncated {
        what: "LBAL section body",
        need: off + size,
        got: data.len(),
    })?;

    // Offset scan: strictly increasing, starting at 0, pointing inside
    // the section. The first value that breaks the pattern is the first
    // byte of the string area.
    let mut offsets: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while body.len() >= 4 * (i + 1) {
        let value = u32le(body, 4 * i)? as usize;
        let plausible =
            value < body.len() && (i == 0 && value == 0 || i > 0 && value > offsets[i - 1]);
        if !plausible {
            break;
        }
        offsets.push(value);
        i += 1;
    }

    let strings_base = 4 * offsets.len();
    let mut labels = Vec::with_capacity(offsets.len());
    for &value in &offsets {
        let start = strings_base + value;
        let end = body[start..]
            .iter()
            .position(|&b| b == 0)
            .map(|nul| start + nul)
            .ok_or(NdsError::Invalid {
                what: "LBAL label is not NUL-terminated",
            })?;
        labels.push(
            core::str::from_utf8(&body[start..end]).map_err(|_| NdsError::Invalid {
                what: "LBAL label is not ASCII",
            })?,
        );
    }
    Ok(labels)
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

    /// The raw `GXTexFmt` value a CHAR or PLTT section stores (the
    /// inverse of [`PixelFmt::from_raw`]; used by the round-trip
    /// serializers).
    #[must_use]
    pub(crate) fn raw(self) -> u32 {
        match self {
            Self::Pltt16 => 3,
            Self::Pltt256 => 4,
        }
    }

    /// The number of bytes in one 8×8 tile.
    #[must_use]
    pub fn tile_size(self) -> usize {
        8 * 8 * usize::from(self.bpp()) / 8
    }
}
