//! NCLR — Nitro Color: BG/sprite palettes. Magic `RLCN`.
//!
//! Beyond the shared Nitro container header (see [`crate::formats`]) a
//! NCLR carries up to two sections, in order:
//!
//! - **PLTT** (`TTLP`) — a serialized `NNSG2dPaletteData` struct followed
//!   by the palette bytes:
//!
//!   ```text
//!   +0x00 4  fmt     GXTexFmt (3 = 4bpp/16 colors, 4 = 8bpp/256 colors)
//!                     The follower-sprite archives set extra flag bits
//!                     (raw 0x000A0004); parse them via fmt_raw().
//!   +0x04 4  bExt    1 = extended-palette bank (multiple 256-color sets)
//!   +0x08 4  szByte  logical palette size the game consumes
//!   +0x0C 4  dataOff palette offset from the struct start (always 16)
//!   ```
//!
//!   `szByte` can exceed the bytes actually stored: on 196 retail files
//!   the section holds only the *unique* palettes while the PMCP section
//!   maps each palette slot onto one of them (see [`Pmcp`]). Use
//!   [`Nclr::is_compressed`] to tell the two cases apart.
//! - **PMCP** (`PMCP`, optional) — a serialized `NNSG2dPaletteCompressInfo`
//!   struct: `u16 numPalette`, `u16 pad (0xBEEF)`, `u32 table offset
//!   (always 8)`, then `numPalette` u16 indices into the stored palettes.
//!
//! Retail HeartGold (US): 4,945 members. See `docs/nitro-gfx.md` for the
//! worked ground truth.

use crate::formats::{NitroSection, PixelFmt, nitro_sections};
use crate::nds::{NdsError, u32le};

/// The raw `fmt` value on 2,152 follower-sprite palettes
/// (`pokegra`, `otherpoke`, `a/0/0/4`, `a/1/1/4`). Every one of them
/// stores an ordinary 16-color (32-byte) palette, and the game's own
/// code only ever compares `fmt == GX_TEXFMT_PLTT256`, so these land in
/// the 16-color path (`obj_pltt_transfer.c` in pret/pokeheartgold).
/// What the `0x000A` half means is not pinned; keep it via
/// [`Nclr::fmt_raw`].
const FLAGGED_16C_FMT: u32 = 0x000A_0004;

/// The PMCP palette-mapping table: which stored palette each of the
/// palette slots uses.
///
/// This is the NitroSDK's `NNSG2dPaletteCompressInfo` on disk.
#[derive(Debug)]
pub struct Pmcp {
    /// The number of palette slots the game expects.
    num_palettes: u16,
    /// For each slot, the index of the stored palette it reuses. One u16
    /// per slot, in slot order.
    indices: Vec<u16>,
}

impl Pmcp {
    /// The number of palette slots.
    #[must_use]
    pub fn num_palettes(&self) -> u16 {
        self.num_palettes
    }

    /// The stored-palette index that palette slot `slot` uses.
    ///
    /// # Errors
    /// Returns an [`NdsError`] if the slot is out of range.
    pub fn palette_of(&self, slot: usize) -> Result<u16, NdsError> {
        self.indices.get(slot).copied().ok_or(NdsError::Invalid {
            what: "PMCP palette slot out of range",
        })
    }
}

/// A parsed NCLR. Borrows the file bytes; see [`Nclr::parse`].
#[derive(Debug)]
pub struct Nclr<'a> {
    /// Container version (always 0x0100 in HeartGold).
    version: u16,
    pixel_fmt: PixelFmt,
    /// The raw `fmt` field, including any flag bits above the GXTexFmt
    /// value (the follower-sprite archives store 0x000A0004).
    fmt_raw: u32,
    /// The SDK's `bExtendedPlt`: the section is an extended-palette bank
    /// holding several 256-color palettes.
    extended: bool,
    /// The palette bytes actually stored in the PLTT section.
    stored: &'a [u8],
    /// The logical palette size (`szByte`) the game consumes. Larger
    /// than `stored` when the file is compressed via PMCP.
    logical_size: u32,
    pmcp: Option<Pmcp>,
}

/// Whether `data` begins with a NCLR header.
#[must_use]
pub fn is_nclr(data: &[u8]) -> bool {
    data.get(0..6) == Some(&[b'R', b'L', b'C', b'N', 0xFF, 0xFE])
}

impl<'a> Nclr<'a> {
    /// Parses a complete NCLR file.
    ///
    /// # Errors
    /// Returns an [`NdsError`] if the container, PLTT section, or PMCP
    /// section is malformed or inconsistent.
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        let (version, sections) = nitro_sections(data, b"RLCN", 2)?;
        let pltt = &sections[0];
        if pltt.magic != *b"TTLP" {
            return Err(NdsError::Invalid {
                what: "NCLR's first section is not PLTT",
            });
        }
        let fmt_raw = u32le(data, pltt.offset + 8)?;
        let pixel_fmt = match fmt_raw {
            3 => PixelFmt::Pltt16,
            4 => PixelFmt::Pltt256,
            FLAGGED_16C_FMT => PixelFmt::Pltt16,
            _ => {
                return Err(NdsError::Invalid {
                    what: "unsupported palette GXTexFmt",
                });
            }
        };
        let extended = u32le(data, pltt.offset + 0xC)? != 0;
        let logical_size = u32le(data, pltt.offset + 0x10)?;
        let data_off = u32le(data, pltt.offset + 0x14)? as usize;
        if data_off != 0x10 {
            return Err(NdsError::Invalid {
                what: "PLTT palette offset (always 0x10 on retail files)",
            });
        }
        let stored = data
            .get(pltt.offset + 8 + data_off..pltt.offset + pltt.size)
            .ok_or(NdsError::Truncated {
                what: "PLTT palette data",
                need: pltt.offset + pltt.size,
                got: data.len(),
            })?;

        let mut pmcp = None;
        if let Some(sec) = sections.get(1) {
            if sec.magic != *b"PMCP" {
                return Err(NdsError::Invalid {
                    what: "NCLR's second section is not PMCP",
                });
            }
            pmcp = Some(parse_pmcp(data, sec)?);
        }

        Ok(Self {
            version,
            pixel_fmt,
            fmt_raw,
            extended,
            stored,
            logical_size,
            pmcp,
        })
    }

    /// The container version (always 0x0100 in HeartGold).
    #[must_use]
    pub fn version(&self) -> u16 {
        self.version
    }

    /// The pixel format the palettes are indexed against (4 or 8 bpp).
    #[must_use]
    pub fn pixel_fmt(&self) -> PixelFmt {
        self.pixel_fmt
    }

    /// Bits per pixel: 4 or 8.
    #[must_use]
    pub fn bpp(&self) -> u8 {
        self.pixel_fmt.bpp()
    }

    /// The raw `fmt` field as stored, including any flag bits above the
    /// `GXTexFmt` value. HeartGold's follower-sprite archives
    /// (`pokegra`, `otherpoke`, `a/0/0/4`, `a/1/1/4`) set 0x000A0004
    /// here while storing ordinary 16-color palettes; what the 0x0A
    /// half means is not pinned down yet, so treat it as opaque.
    #[must_use]
    pub fn fmt_raw(&self) -> u32 {
        self.fmt_raw
    }

    /// Whether this is an extended-palette bank (`bExtendedPlt`):
    /// several 256-color palettes concatenated.
    #[must_use]
    pub fn is_extended(&self) -> bool {
        self.extended
    }

    /// The palette bytes actually stored in the file.
    ///
    /// BGR555 colors, one per 2 bytes, `2^bpp` per palette.
    #[must_use]
    pub fn palette_data(&self) -> &'a [u8] {
        self.stored
    }

    /// The number of stored colors (palette bytes / 2).
    #[must_use]
    pub fn color_count(&self) -> usize {
        self.stored.len() / 2
    }

    /// The logical palette size (`szByte`) the game consumes.
    #[must_use]
    pub fn logical_size(&self) -> u32 {
        self.logical_size
    }

    /// Whether the file is palette-compressed: the section stores fewer
    /// bytes than the game consumes, and the PMCP table says which stored
    /// palette each slot reuses. 196 retail files are compressed.
    #[must_use]
    pub fn is_compressed(&self) -> bool {
        self.logical_size as usize > self.stored.len()
    }

    /// The PMCP palette-mapping table, if present.
    #[must_use]
    pub fn pmcp(&self) -> Option<&Pmcp> {
        self.pmcp.as_ref()
    }
}

fn parse_pmcp(data: &[u8], sec: &NitroSection) -> Result<Pmcp, NdsError> {
    let num = crate::nds::u16le(data, sec.offset + 8)?;
    let pad = crate::nds::u16le(data, sec.offset + 0xA)?;
    let table_off = u32le(data, sec.offset + 0xC)? as usize;
    if pad != 0xBEEF {
        return Err(NdsError::Invalid {
            what: "PMCP pad (the SDK's 0xBEEF marker)",
        });
    }
    if table_off != 8 {
        return Err(NdsError::Invalid {
            what: "PMCP table offset (always 8 on retail files)",
        });
    }
    if sec.size != 8 + table_off + 2 * usize::from(num) {
        return Err(NdsError::Invalid {
            what: "PMCP section size disagrees with the palette count",
        });
    }
    let mut indices = Vec::with_capacity(num.into());
    for i in 0..num {
        indices.push(crate::nds::u16le(
            data,
            sec.offset + 8 + table_off + 2 * usize::from(i),
        )?);
    }
    Ok(Pmcp {
        num_palettes: num,
        indices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a NCLR in memory with two stored 16-color palettes and a
    /// PMCP mapping three slots onto them.
    fn build_compressed_nclr() -> Vec<u8> {
        let colors: [u8; 2 * 16 * 2] = [0x55; 2 * 16 * 2];
        let pltt_size = 8 + 0x10 + colors.len();
        let pmcp = [0x03u16, 0xBEEF];
        let pmcp_indices = [0x0000u16, 0x0000, 0x0001];
        let pmcp_size = 8 + 8 + pmcp_indices.len() * 2;
        let total = 0x10 + pltt_size + pmcp_size;

        let mut rom = vec![0u8; total];
        rom[0..4].copy_from_slice(b"RLCN");
        rom[4..6].copy_from_slice(&0xFEFFu16.to_le_bytes());
        rom[6..8].copy_from_slice(&0x0100u16.to_le_bytes());
        rom[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        rom[0xC..0xE].copy_from_slice(&0x10u16.to_le_bytes());
        rom[0xE..0x10].copy_from_slice(&2u16.to_le_bytes());

        rom[0x10..0x14].copy_from_slice(b"TTLP");
        rom[0x14..0x18].copy_from_slice(&(pltt_size as u32).to_le_bytes());
        rom[0x18..0x1C].copy_from_slice(&3u32.to_le_bytes()); // PLTT16
        rom[0x1C..0x20].copy_from_slice(&0u32.to_le_bytes()); // not extended
        rom[0x20..0x24].copy_from_slice(&((colors.len() * 3) as u32).to_le_bytes()); // logical: 3 palettes
        rom[0x24..0x28].copy_from_slice(&0x10u32.to_le_bytes()); // data offset
        rom[0x28..0x28 + colors.len()].copy_from_slice(&colors);

        let off = 0x10 + pltt_size;
        rom[off..off + 4].copy_from_slice(b"PMCP");
        rom[off + 4..off + 8].copy_from_slice(&(pmcp_size as u32).to_le_bytes());
        for (i, v) in pmcp.iter().enumerate() {
            rom[off + 8 + 2 * i..off + 10 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        rom[off + 0xC..off + 0x10].copy_from_slice(&8u32.to_le_bytes());
        for (i, v) in pmcp_indices.iter().enumerate() {
            rom[off + 0x10 + 2 * i..off + 0x12 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        rom
    }

    #[test]
    fn parses_flagged_16color_fmt() {
        // The follower-sprite palettes store 0x000A0004 but are ordinary
        // 16-color palettes; the game's exact `== GX_TEXFMT_PLTT256`
        // comparison treats them as such.
        let mut data = build_compressed_nclr();
        data[0x18..0x1C].copy_from_slice(&FLAGGED_16C_FMT.to_le_bytes());
        let nclr = Nclr::parse(&data).expect("flagged-fmt NCLR must parse");
        assert_eq!(nclr.fmt_raw(), FLAGGED_16C_FMT);
        assert_eq!(nclr.pixel_fmt(), PixelFmt::Pltt16);
        assert_eq!(nclr.bpp(), 4);
    }

    #[test]
    fn parses_compressed_palettes() {
        let data = build_compressed_nclr();
        assert!(is_nclr(&data));
        let nclr = Nclr::parse(&data).expect("compressed NCLR must parse");

        assert_eq!(nclr.version(), 0x0100);
        assert_eq!(nclr.pixel_fmt(), PixelFmt::Pltt16);
        assert_eq!(nclr.bpp(), 4);
        assert_eq!(nclr.fmt_raw(), 3);
        assert!(!nclr.is_extended());
        assert_eq!(nclr.color_count(), 32); // 2 stored palettes
        assert!(nclr.is_compressed());
        assert_eq!(nclr.logical_size(), 192); // 3 × 32 bytes

        let pmcp = nclr.pmcp().expect("PMCP present");
        assert_eq!(pmcp.num_palettes(), 3);
        assert_eq!(pmcp.palette_of(0), Ok(0));
        assert_eq!(pmcp.palette_of(1), Ok(0));
        assert_eq!(pmcp.palette_of(2), Ok(1));
        assert!(pmcp.palette_of(3).is_err());
    }

    #[test]
    fn rejects_broken_nclr() {
        let good = build_compressed_nclr();

        let mut bad_magic = good.clone();
        bad_magic[0..4].copy_from_slice(b"RLCM");
        assert!(Nclr::parse(&bad_magic).is_err());

        assert!(Nclr::parse(&good[..good.len() - 2]).is_err());

        // GXTexFmt 5 (COMP4x4) never appears in a 2D palette.
        let mut bad_fmt = good.clone();
        bad_fmt[0x18..0x1C].copy_from_slice(&5u32.to_le_bytes());
        assert!(Nclr::parse(&bad_fmt).is_err());

        // Palette offset moved off 0x10.
        let mut bad_off = good.clone();
        bad_off[0x24..0x28].copy_from_slice(&0x20u32.to_le_bytes());
        assert!(Nclr::parse(&bad_off).is_err());

        // The SDK's 0xBEEF pad marker replaced (PMCP is at 0x68, pad at +0xA).
        let mut bad_pad = good.clone();
        bad_pad[0x72..0x74].copy_from_slice(&0x0000u16.to_le_bytes());
        assert!(Nclr::parse(&bad_pad).is_err());

        assert!(!is_nclr(b"not an nclr at all"));
        assert!(Nclr::parse(b"RLCN").is_err());
    }
}
