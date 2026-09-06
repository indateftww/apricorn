//! NSCR — Nitro Screen: BG map data (which tile goes where). Magic `RCSN`.
//!
//! A single **SCRN** (`NRCS`) section holding a serialized
//! `NNSG2dScreenData` struct followed immediately by the map entries:
//!
//! ```text
//! +0x00 2  width        screen width in PIXELS (the game reads this,
//!                      not tile columns; 32 tiles = 256 px reads as 256)
//! +0x02 2  height       screen height in pixels
//! +0x04 2  colorMode    GX_BG_COLORMODE_16 (0) / _256 (1); HeartGold
//!                      also stores 2 on 17 files (meaning unpinned)
//! +0x06 2  screenFormat 0 = text BG (u16 entries: tile | palette<<12 |
//!                      flip bits); 1 and 2 appear on 19 retail files
//!                      (rotation-screen variants; Phase 3 pins the HW
//!                      meaning)
//! +0x08 4  szByte       map entry bytes that follow this header
//! ```
//!
//! For the 781 standard text screens the entries are `width/8 × height/8`
//! little-endian u16s. The 10 `screenFormat == 1` files instead carry
//! `width/8 × height/8` u8 entries (half the formula), so treat `szByte`
//! as authoritative, not the dimensions.
//!
//! Retail HeartGold (US): 791 members. See `docs/nitro-gfx.md` for the
//! worked ground truth.

use crate::formats::nitro_sections;
use crate::nds::{NdsError, u16le, u32le};

/// A parsed NSCR. Borrows the file bytes; see [`Nscr::parse`].
#[derive(Debug)]
pub struct Nscr<'a> {
    /// Container version (always 0x0100 in HeartGold).
    version: u16,
    /// Screen width in pixels.
    width: u16,
    /// Screen height in pixels.
    height: u16,
    /// Raw `colorMode`: 0 = 16 colors, 1 = 256 colors (`GX_BG_COLORMODE_*);
    /// 2 appears on 17 retail files with no SDK counterpart.
    color_mode: u16,
    /// Raw `screenFormat`: 0 = text BG; 1 and 2 appear on 19 retail files.
    screen_format: u16,
    entries: &'a [u8],
}

/// Whether `data` begins with a NSCR header.
#[must_use]
pub fn is_nscr(data: &[u8]) -> bool {
    data.get(0..6) == Some(&[b'R', b'C', b'S', b'N', 0xFF, 0xFE])
}

impl<'a> Nscr<'a> {
    /// Parses a complete NSCR file.
    ///
    /// # Errors
    /// Returns an [`NdsError`] if the container or SCRN section is
    /// malformed or inconsistent.
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        let (version, sections) = nitro_sections(data, b"RCSN", 1)?;
        let scrn = &sections[0];
        if scrn.magic != *b"NRCS" {
            return Err(NdsError::Invalid {
                what: "NSCR's first section is not SCRN",
            });
        }
        let width = u16le(data, scrn.offset + 8)?;
        let height = u16le(data, scrn.offset + 0xA)?;
        let color_mode = u16le(data, scrn.offset + 0xC)?;
        let screen_format = u16le(data, scrn.offset + 0xE)?;
        let sz_byte = u32le(data, scrn.offset + 0x10)? as usize;
        if width % 8 != 0 || height % 8 != 0 {
            return Err(NdsError::Invalid {
                what: "NSCR dimensions are not a whole number of tiles",
            });
        }
        if sz_byte != scrn.size - 0x14 {
            return Err(NdsError::Invalid {
                what: "SCRN section size disagrees with its map data",
            });
        }
        let entries = data
            .get(scrn.offset + 0x14..scrn.offset + scrn.size)
            .ok_or(NdsError::Truncated {
                what: "SCRN map entries",
                need: scrn.offset + scrn.size,
                got: data.len(),
            })?;

        Ok(Self {
            version,
            width,
            height,
            color_mode,
            screen_format,
            entries,
        })
    }

    /// The container version (always 0x0100 in HeartGold).
    #[must_use]
    pub fn version(&self) -> u16 {
        self.version
    }

    /// The screen width in pixels (always a multiple of 8).
    #[must_use]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// The screen height in pixels (always a multiple of 8).
    #[must_use]
    pub fn height(&self) -> u16 {
        self.height
    }

    /// The screen width in 8×8 tiles.
    #[must_use]
    pub fn width_tiles(&self) -> u16 {
        self.width / 8
    }

    /// The screen height in 8×8 tiles.
    #[must_use]
    pub fn height_tiles(&self) -> u16 {
        self.height / 8
    }

    /// The raw `colorMode`: 0 = 16 colors, 1 = 256 colors, and 2 on 17
    /// HeartGold files where the SDK enum runs out. Not yet pinned.
    #[must_use]
    pub fn color_mode(&self) -> u16 {
        self.color_mode
    }

    /// The raw `screenFormat`: 0 = text BG (u16 entries); 1 and 2 appear
    /// on 19 retail files (rotation-screen variants). Not yet pinned.
    #[must_use]
    pub fn screen_format(&self) -> u16 {
        self.screen_format
    }

    /// The map entry bytes (`szByte` of them), starting right after the
    /// screen header. Interpretation depends on [`Nscr::screen_format`].
    #[must_use]
    pub fn entries(&self) -> &'a [u8] {
        self.entries
    }

    /// Entry `(x, y)` for a standard text screen (`screen_format == 0`):
    /// tile index | palette << 12 | hflip 0x400 | vflip 0x800.
    ///
    /// # Errors
    /// Returns an [`NdsError`] if the format is not a text screen or the
    /// coordinate is out of range.
    pub fn entry(&self, x: u16, y: u16) -> Result<u16, NdsError> {
        if self.screen_format != 0 {
            return Err(NdsError::Invalid {
                what: "entry() only decodes text-format screens",
            });
        }
        if x >= self.width_tiles() || y >= self.height_tiles() {
            return Err(NdsError::Invalid {
                what: "screen coordinate out of range",
            });
        }
        let idx = usize::from(y) * usize::from(self.width_tiles()) + usize::from(x);
        u16le(self.entries, 2 * idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a NSCR in memory: a 32×16-pixel (4×2-tile) text screen
    /// with eight entries.
    fn build_nscr() -> Vec<u8> {
        let entries: [u8; 16] = [
            0x4A, 0x00, 0x4B, 0x00, 0x4C, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, // last: 0x040C
        ];
        let scrn_size = 8 + 0x0C + entries.len();
        let total = 0x10 + scrn_size;

        let mut rom = vec![0u8; total];
        rom[0..4].copy_from_slice(b"RCSN");
        rom[4..6].copy_from_slice(&0xFEFFu16.to_le_bytes());
        rom[6..8].copy_from_slice(&0x0100u16.to_le_bytes());
        rom[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        rom[0xC..0xE].copy_from_slice(&0x10u16.to_le_bytes());
        rom[0xE..0x10].copy_from_slice(&1u16.to_le_bytes());

        rom[0x10..0x14].copy_from_slice(b"NRCS");
        rom[0x14..0x18].copy_from_slice(&(scrn_size as u32).to_le_bytes());
        rom[0x18..0x1A].copy_from_slice(&32u16.to_le_bytes()); // width px
        rom[0x1A..0x1C].copy_from_slice(&16u16.to_le_bytes()); // height px
        rom[0x1C..0x1E].copy_from_slice(&0u16.to_le_bytes()); // 16 colors
        rom[0x1E..0x20].copy_from_slice(&0u16.to_le_bytes()); // text format
        rom[0x20..0x24].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        rom[0x24..0x24 + entries.len()].copy_from_slice(&entries);
        rom
    }

    #[test]
    fn parses_text_screen() {
        let data = build_nscr();
        assert!(is_nscr(&data));
        let nscr = Nscr::parse(&data).expect("NSCR must parse");

        assert_eq!(nscr.width(), 32);
        assert_eq!(nscr.height(), 16);
        assert_eq!(nscr.width_tiles(), 4);
        assert_eq!(nscr.height_tiles(), 2);
        assert_eq!(nscr.color_mode(), 0);
        assert_eq!(nscr.screen_format(), 0);
        assert_eq!(nscr.entries(), &data[0x24..0x34]);
        assert_eq!(nscr.entry(0, 0), Ok(0x004A));
        assert_eq!(nscr.entry(1, 0), Ok(0x004B));
        // Palette 4, hflip: entry 0x040C.
        assert_eq!(nscr.entry(3, 1), Ok(0x040C));
        assert!(nscr.entry(4, 0).is_err());
        assert!(nscr.entry(0, 2).is_err());
    }

    #[test]
    fn rejects_broken_nscr() {
        let good = build_nscr();

        let mut bad_magic = good.clone();
        bad_magic[0..4].copy_from_slice(b"RCSM");
        assert!(Nscr::parse(&bad_magic).is_err());

        assert!(Nscr::parse(&good[..good.len() - 2]).is_err());

        // Dimensions not tile-aligned.
        let mut bad_dims = good.clone();
        bad_dims[0x18..0x1A].copy_from_slice(&33u16.to_le_bytes());
        assert!(Nscr::parse(&bad_dims).is_err());

        // szByte disagrees with the section size.
        let mut bad_size = good.clone();
        bad_size[0x20..0x24].copy_from_slice(&4u32.to_le_bytes());
        assert!(Nscr::parse(&bad_size).is_err());

        assert!(!is_nscr(b"not an nscr at all"));
        assert!(Nscr::parse(b"RCSN").is_err());
    }
}
