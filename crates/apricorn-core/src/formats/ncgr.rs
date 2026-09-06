//! NCGR — Nitro Character: raw BG/sprite tile data. Magic `RGCN`.
//!
//! A NCGR holds one sheet of 8×8 tiles. Beyond the shared Nitro container
//! header (see [`crate::formats`]) it carries up to two sections, in
//! order:
//!
//! - **CHAR** (`RAHC`) — the tile data, prefixed by a serialized
//!   `NNSG2dCharacterData` struct from the NitroSDK. The game loads NCGRs
//!   straight into this struct via `NNS_G2dGetUnpackedCharacterData`, so
//!   the field order here *is* the SDK's struct layout:
//!
//!   ```text
//!   +0x00 2  H        tile rows (0xFFFF = none; linear OBJ data)
//!   +0x02 2  W        tile columns (0xFFFF = none)
//!   +0x04 4  fmt      GXTexFmt (3 = 4bpp, 4 = 8bpp; see PixelFmt)
//!   +0x08 4  mapping  GXOBJVRamModeChar (see CharMapping)
//!   +0x0C 4  charFmt  low byte = character format; bit 8 = VRAM-transfer
//!   +0x10 4  szByte   tile data size in bytes
//!   +0x14 4  dataOff  tile data offset from the struct start (always 24)
//!   ```
//!
//!   followed by `szByte` bytes of tile data. For BG use the data is
//!   linear (row-major), not GBA-tiled.
//! - **CPOS** (`SOPC`, optional) — the sheet size again, width-then-height
//!   in tiles (note the CHAR struct above stores height first). Always
//!   `u32 0, u16 W, u16 H`. Present on 860 retail files.
//!
//! Retail HeartGold (US): 7,937 members, 7,888 of them 4bpp. See
//! `docs/nitro-gfx.md` for the worked ground truth.

use crate::formats::{NitroSection, PixelFmt, nitro_sections};
use crate::nds::{NdsError, u16le, u32le};

/// How a sheet of OBJ tiles maps into VRAM (`GXOBJVRamModeChar`).
///
/// The enum values are the packed register bits the SDK writes directly
/// into `GX_DISPCNT` (OBJMAP bit 4, extended-OBJ size bits 20-21) — which
/// is why the 1D_64K and larger modes look like `0x00100010` etc. on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CharMapping {
    /// 2D mapping (BG-style tiling of OBJ VRAM).
    TwoD,
    /// 1D mapping, 32 KB boundary.
    OneD32K,
    /// 1D mapping, 64 KB boundary (raw value `0x00100010`).
    OneD64K,
    /// 1D mapping, 128 KB boundary (raw value `0x00200010`).
    OneD128K,
    /// 1D mapping, 256 KB boundary (raw value `0x00300010`).
    OneD256K,
}

impl CharMapping {
    /// Decodes the raw `GXOBJVRamModeChar` value.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any value outside the enum.
    pub(crate) fn from_raw(raw: u32) -> Result<Self, NdsError> {
        match raw {
            0 => Ok(Self::TwoD),
            0x10 => Ok(Self::OneD32K),
            0x0010_0010 => Ok(Self::OneD64K),
            0x0020_0010 => Ok(Self::OneD128K),
            0x0030_0010 => Ok(Self::OneD256K),
            _ => Err(NdsError::Invalid {
                what: "unknown GXOBJVRamModeChar",
            }),
        }
    }

    /// The raw `GXOBJVRamModeChar` value (the inverse of
    /// [`CharMapping::from_raw`]; used by the round-trip serializer).
    #[must_use]
    pub(crate) fn raw(self) -> u32 {
        match self {
            Self::TwoD => 0,
            Self::OneD32K => 0x10,
            Self::OneD64K => 0x0010_0010,
            Self::OneD128K => 0x0020_0010,
            Self::OneD256K => 0x0030_0010,
        }
    }
}

/// A parsed NCGR. Borrows the file bytes; see [`Ncgr::parse`].
#[derive(Debug)]
pub struct Ncgr<'a> {
    /// Container version (0x0100 or 0x0101 in HeartGold).
    version: u16,
    /// Tile rows (`H`); `0xFFFF` when the sheet has no grid.
    height: u16,
    /// Tile columns (`W`); `0xFFFF` when the sheet has no grid.
    width: u16,
    pixel_fmt: PixelFmt,
    mapping: CharMapping,
    /// Raw `characterFmt`: low byte is the character format, bit 8 the
    /// VRAM-transfer flag (set on files with an attached transfer area).
    character_fmt: u32,
    tiles: &'a [u8],
    /// CPOS sheet size, `(width, height)` in tiles.
    cpos: Option<(u16, u16)>,
}

/// Whether `data` begins with a NCGR header.
#[must_use]
pub fn is_ncgr(data: &[u8]) -> bool {
    data.get(0..6) == Some(&[b'R', b'G', b'C', b'N', 0xFF, 0xFE])
}

impl<'a> Ncgr<'a> {
    /// Parses a complete NCGR file.
    ///
    /// # Errors
    /// Returns an [`NdsError`] if the container, CHAR section, or CPOS
    /// section is malformed or inconsistent.
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        let (version, sections) = nitro_sections(data, b"RGCN", 2)?;
        Self::from_sections(data, version, &sections)
    }

    fn from_sections(
        data: &'a [u8],
        version: u16,
        sections: &[NitroSection],
    ) -> Result<Self, NdsError> {
        let char = &sections[0];
        if char.magic != *b"RAHC" {
            return Err(NdsError::Invalid {
                what: "NCGR's first section is not CHAR",
            });
        }
        let height = u16le(data, char.offset + 8)?;
        let width = u16le(data, char.offset + 0xA)?;
        let pixel_fmt = PixelFmt::from_raw(u32le(data, char.offset + 0xC)?)?;
        let mapping = CharMapping::from_raw(u32le(data, char.offset + 0x10)?)?;
        let character_fmt = u32le(data, char.offset + 0x14)?;
        let sz_byte = u32le(data, char.offset + 0x18)? as usize;
        let data_off = u32le(data, char.offset + 0x1C)? as usize;
        if data_off != 0x18 {
            return Err(NdsError::Invalid {
                what: "CHAR tile data offset (always 0x18 on retail files)",
            });
        }
        if 8 + data_off + sz_byte != char.size {
            return Err(NdsError::Invalid {
                what: "CHAR section size disagrees with its tile data",
            });
        }
        let tiles = data
            .get(char.offset + 8 + data_off..char.offset + char.size)
            .ok_or(NdsError::Truncated {
                what: "CHAR tile data",
                need: char.offset + char.size,
                got: data.len(),
            })?;

        // Optional CPOS: the sheet size again, width-then-height in tiles.
        let mut cpos = None;
        if let Some(sec) = sections.get(1) {
            if sec.magic != *b"SOPC" {
                return Err(NdsError::Invalid {
                    what: "NCGR's second section is not CPOS",
                });
            }
            if sec.size != 0x10 || u32le(data, sec.offset + 8)? != 0 {
                return Err(NdsError::Invalid {
                    what: "CPOS section body",
                });
            }
            let w = u16le(data, sec.offset + 0xC)?;
            let h = u16le(data, sec.offset + 0xE)?;
            cpos = Some((w, h));
        }

        Ok(Self {
            version,
            height,
            width,
            pixel_fmt,
            mapping,
            character_fmt,
            tiles,
            cpos,
        })
    }

    /// The container version (0x0100 or 0x0101 in HeartGold).
    #[must_use]
    pub fn version(&self) -> u16 {
        self.version
    }

    /// The sheet height in 8×8 tiles, if the file declares a grid.
    ///
    /// `None` for linear OBJ character data (`H == 0xFFFF`).
    #[must_use]
    pub fn height(&self) -> Option<u16> {
        (self.height != 0xFFFF).then_some(self.height)
    }

    /// The sheet width in 8×8 tiles, if the file declares a grid.
    ///
    /// `None` for linear OBJ character data (`W == 0xFFFF`).
    #[must_use]
    pub fn width(&self) -> Option<u16> {
        (self.width != 0xFFFF).then_some(self.width)
    }

    /// The pixel format (4 or 8 bpp).
    #[must_use]
    pub fn pixel_fmt(&self) -> PixelFmt {
        self.pixel_fmt
    }

    /// Bits per pixel: 4 or 8.
    #[must_use]
    pub fn bpp(&self) -> u8 {
        self.pixel_fmt.bpp()
    }

    /// The OBJ VRAM mapping mode.
    #[must_use]
    pub fn mapping(&self) -> CharMapping {
        self.mapping
    }

    /// The raw `characterFmt` field: its low byte is the character
    /// format and bit 8 is the SDK's VRAM-transfer-data flag.
    #[must_use]
    pub fn character_fmt(&self) -> u32 {
        self.character_fmt
    }

    /// Whether the file carries the SDK's VRAM-transfer-data flag
    /// (`characterFmt` bit 8), meaning an OBJ transfer task owns its VRAM
    /// placement.
    #[must_use]
    pub fn has_vram_transfer(&self) -> bool {
        self.character_fmt & 0x100 != 0
    }

    /// The tile data, `szByte` bytes of raw pixels.
    ///
    /// For BG use this is linear (row-major) 8×8 tile data, not the
    /// GBA 8×8-tiled arrangement.
    #[must_use]
    pub fn tile_data(&self) -> &'a [u8] {
        self.tiles
    }

    /// The number of complete 8×8 tiles in the sheet.
    #[must_use]
    pub fn tile_count(&self) -> usize {
        self.tiles.len() / self.pixel_fmt.tile_size()
    }

    /// The CPOS sheet size, `(width, height)` in tiles, if the file has a
    /// CPOS section.
    #[must_use]
    pub fn cpos(&self) -> Option<(u16, u16)> {
        self.cpos
    }

    /// Re-serializes the parsed NCGR into its container form.
    ///
    /// Byte-exact: the parse retains every stored field (the container
    /// version, the grid, the formats, `characterFmt`, the tile bytes, and
    /// the CPOS pair), so this is the round-trip half of the parser guard
    /// (`tests/roundtrip_hg.rs` re-serializes every NCGR in the ROM and
    /// byte-compares against the original).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let char_size = 8 + 0x18 + self.tiles.len();
        let cpos_size = if self.cpos.is_some() { 0x10 } else { 0 };
        let total = 0x10 + char_size + cpos_size;

        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"RGCN");
        out.extend_from_slice(&0xFEFFu16.to_le_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&0x10u16.to_le_bytes());
        out.extend_from_slice(&(u16::from(self.cpos.is_some()) + 1).to_le_bytes());
        out.extend_from_slice(b"RAHC");
        out.extend_from_slice(&(char_size as u32).to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.pixel_fmt.raw().to_le_bytes());
        out.extend_from_slice(&self.mapping.raw().to_le_bytes());
        out.extend_from_slice(&self.character_fmt.to_le_bytes());
        out.extend_from_slice(&(self.tiles.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x18u32.to_le_bytes());
        out.extend_from_slice(self.tiles);

        if let Some((w, h)) = self.cpos {
            out.extend_from_slice(b"SOPC");
            out.extend_from_slice(&0x10u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&w.to_le_bytes());
            out.extend_from_slice(&h.to_le_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a NCGR in memory: one CHAR section with a 4bpp 2×1-tile
    /// sheet (16 bytes of pixels), optionally followed by a CPOS section.
    fn build_ncgr(with_cpos: bool) -> Vec<u8> {
        let tiles: &[u8] = &[0xAB; 2 * 32];
        let char_size = 8 + 0x18 + tiles.len();
        let cpos_size = if with_cpos { 0x10 } else { 0 };
        let total = 0x10 + char_size + cpos_size;

        let mut rom = vec![0u8; total];
        rom[0..4].copy_from_slice(b"RGCN");
        rom[4..6].copy_from_slice(&0xFEFFu16.to_le_bytes());
        rom[6..8].copy_from_slice(&0x0100u16.to_le_bytes());
        rom[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        rom[0xC..0xE].copy_from_slice(&0x10u16.to_le_bytes());
        rom[0xE..0x10].copy_from_slice(&(if with_cpos { 2u16 } else { 1u16 }).to_le_bytes());

        rom[0x10..0x14].copy_from_slice(b"RAHC");
        rom[0x14..0x18].copy_from_slice(&(char_size as u32).to_le_bytes());
        let hdr = 0x18;
        rom[hdr..hdr + 2].copy_from_slice(&1u16.to_le_bytes()); // H
        rom[hdr + 2..hdr + 4].copy_from_slice(&2u16.to_le_bytes()); // W
        rom[hdr + 4..hdr + 8].copy_from_slice(&3u32.to_le_bytes()); // PLTT16
        rom[hdr + 8..hdr + 12].copy_from_slice(&0u32.to_le_bytes()); // 2D
        rom[hdr + 12..hdr + 16].copy_from_slice(&1u32.to_le_bytes()); // charFmt
        rom[hdr + 16..hdr + 20].copy_from_slice(&(tiles.len() as u32).to_le_bytes());
        rom[hdr + 20..hdr + 24].copy_from_slice(&0x18u32.to_le_bytes());
        rom[hdr + 24..hdr + 24 + tiles.len()].copy_from_slice(tiles);

        if with_cpos {
            let off = 0x10 + char_size;
            rom[off..off + 4].copy_from_slice(b"SOPC");
            rom[off + 4..off + 8].copy_from_slice(&0x10u32.to_le_bytes());
            rom[off + 0xC..off + 0xE].copy_from_slice(&2u16.to_le_bytes()); // W
            rom[off + 0xE..off + 0x10].copy_from_slice(&1u16.to_le_bytes()); // H
        }
        rom
    }

    #[test]
    fn parses_char_with_and_without_cpos() {
        let data = build_ncgr(false);
        assert!(is_ncgr(&data));
        let ncgr = Ncgr::parse(&data).expect("CHAR-only NCGR must parse");
        assert_eq!(ncgr.height(), Some(1));
        assert_eq!(ncgr.width(), Some(2));
        assert_eq!(ncgr.pixel_fmt(), PixelFmt::Pltt16);
        assert_eq!(ncgr.bpp(), 4);
        assert_eq!(ncgr.mapping(), CharMapping::TwoD);
        assert!(!ncgr.has_vram_transfer());
        assert_eq!(ncgr.tile_data(), &[0xAB; 64]);
        assert_eq!(ncgr.tile_count(), 2);
        assert_eq!(ncgr.cpos(), None);

        let data = build_ncgr(true);
        let ncgr = Ncgr::parse(&data).expect("CHAR+CPOS NCGR must parse");
        assert_eq!(ncgr.cpos(), Some((2, 1)));
    }

    #[test]
    fn decodes_1d_mapping_modes() {
        for (raw, expected) in [
            (0u32, CharMapping::TwoD),
            (0x10, CharMapping::OneD32K),
            (0x0010_0010, CharMapping::OneD64K),
            (0x0020_0010, CharMapping::OneD128K),
            (0x0030_0010, CharMapping::OneD256K),
        ] {
            let mut data = build_ncgr(false);
            data[0x18 + 8..0x18 + 12].copy_from_slice(&raw.to_le_bytes());
            let ncgr = Ncgr::parse(&data).expect("mapping-mode NCGR must parse");
            assert_eq!(ncgr.mapping(), expected);
            assert_eq!(ncgr.to_bytes(), data, "round-trips the raw mapping");
        }
    }

    #[test]
    fn round_trips_synthetic_files() {
        for with_cpos in [false, true] {
            let data = build_ncgr(with_cpos);
            let ncgr = Ncgr::parse(&data).expect("synthetic NCGR must parse");
            assert_eq!(ncgr.to_bytes(), data, "byte-exact with and without CPOS");
        }
    }

    #[test]
    fn rejects_broken_ncgr() {
        let good = build_ncgr(false);

        let mut bad_magic = good.clone();
        bad_magic[0..4].copy_from_slice(b"RGCM");
        assert!(Ncgr::parse(&bad_magic).is_err());

        assert!(Ncgr::parse(&good[..good.len() - 2]).is_err());

        // GXTexFmt 6 (DIRECT) is not a 2D format.
        let mut bad_fmt = good.clone();
        bad_fmt[0x1C..0x20].copy_from_slice(&6u32.to_le_bytes());
        assert!(Ncgr::parse(&bad_fmt).is_err());

        // Unknown OBJ mapping.
        let mut bad_map = good.clone();
        bad_map[0x20..0x24].copy_from_slice(&0x40u32.to_le_bytes());
        assert!(Ncgr::parse(&bad_map).is_err());

        // Tile data offset moved off 0x18.
        let mut bad_off = good.clone();
        bad_off[0x2C..0x30].copy_from_slice(&0x20u32.to_le_bytes());
        assert!(Ncgr::parse(&bad_off).is_err());

        assert!(!is_ncgr(b"not an ncgr at all"));
        assert!(Ncgr::parse(b"RGCN").is_err());
    }
}
