//! The font system — a port of pret's `font_data.c` and the glyph
//! decode of `text.c` (Phase 4, step 5).
//!
//! HGSS fonts are raw binary members of `NARC_graphic_font`
//! (`a/0/1/6`) — not Nitro containers — so unlike the NCGR/NCLR/NSCR
//! pipeline there is no format parser or cache chunk to travel: the
//! [`Font`] here *is* the decode, straight from the member bytes.
//!
//! Layout (pret `struct FontHeader`, `FontData_Init`):
//!
//! ```text
//! u32 headerSize       glyph data starts here
//! u32 widthDataStart   numGlyphs width bytes start here
//! u32 numGlyphs
//! u8  fixedWidth       per-glyph advance when the width table is off
//! u8  fixedHeight      pixel rows copied per glyph (ret->height)
//! u8  glyphWidth       glyph width in tiles (1–2)
//! u8  glyphHeight      glyph height in tiles (1–2)
//! ```
//!
//! Each glyph is `16 × glyphWidth × glyphHeight` source bytes in
//! row-major tile order; each 16-byte source tile is 8 little-endian
//! u16s, one per row, where the u16's *high* byte holds the left half
//! row and the low byte the right half — every 2 bits one pixel's
//! level: `0` transparent, `1` foreground, `2` shadow, `3` background
//! (`DecompressGlyphTile` + `GenerateFontHalfRowLookupTable`, whose
//! `colors[]` is exactly that order, and whose index arithmetic puts
//! the byte's *high* 2-bit pair at the half row's leftmost pixel).
//! The levels, not the palette indices, are what this module stores:
//! the text colors are chosen per print by the message printer, the
//! same way the game rebuilds its half-row lookup table per
//! [`crate::app::text`] step.
//!
//! Decoded glyph pixels land in a uniform 16×16 block layout —
//! tile-major, matching `GlyphInfo`'s buffer, where `CopyGlyphToWindow`
//! reads row `py` of tile `(tx, ty)` at `(ty·2+tx)·64 + (py mod 8)·8 +
//! (px mod 8)` levels — the pixel-order translation of the C's packed
//! `(srcY/8·64) + (srcX/8·32)` addressing.

use crate::nds::NdsError;
use crate::text::ctrl::{EXT_CTRL_CODE_BEGIN, CHAR_LF};
use crate::formats::EOS;

/// The fallback glyph index the game's loader clamps to (`428 - 1`,
/// pret `TryLoadGlyph`) — a code unit beyond the table's last glyph
/// renders glyph 427 (the `?` box).
pub const FALLBACK_GLYPH_INDEX: u32 = 428 - 1;

/// One font's decoded data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    fixed_width: u8,
    fixed_height: u8,
    glyph_width: u8,
    glyph_height: u8,
    num_glyphs: u32,
    /// Per-glyph advance widths, `num_glyphs` entries — empty for a
    /// fixed-width font (the game never loads one; `sFontArcParam`
    /// marks every HGSS font variable-width).
    widths: Vec<u8>,
    /// Per-glyph decoded levels, `16·16` levels each in the block
    /// layout above, `num_glyphs` glyphs.
    glyphs: Vec<[u8; 16 * 16]>,
}

impl Font {
    /// Parses a font member (`FontData_Init` + the direct-mode
    /// preload of `InitFontResources_FromPreloaded`).
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the header is truncated or
    /// out-of-range (`glyphWidth`/`glyphHeight` beyond the 1–2 tiles
    /// the game's shape table addresses), the width table or glyph
    /// data would run past the member, or a variable-width font's
    /// table could not supply the fallback glyph's width — the
    /// out-of-bounds read the retail fonts never hit.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        let invalid = |what: &'static str| NdsError::Invalid { what };
        if data.len() < 16 {
            return Err(invalid("font header is truncated"));
        }
        let header_size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let width_start = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let num_glyphs = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let fixed_width = data[12];
        let fixed_height = data[13];
        let glyph_width = data[14];
        let glyph_height = data[15];
        if !(1..=2).contains(&glyph_width) || !(1..=2).contains(&glyph_height) {
            return Err(invalid("font glyph shape is beyond the game's table"));
        }
        if fixed_height == 0 || usize::from(fixed_height) > usize::from(glyph_height) * 8 {
            // The copy reads fixedHeight rows out of a glyphHeight-tile
            // buffer; the retail fonts never exceed it.
            return Err(invalid("font fixed height exceeds its glyph shape"));
        }

        let num_glyphs = usize::try_from(num_glyphs).map_err(|_| invalid("glyph count"))?;
        // Variable width (the only mode HGSS loads): numGlyphs bytes
        // at widthDataStart.
        let mut widths = Vec::new();
        if width_start != 0 {
            let start = usize::try_from(width_start).map_err(|_| invalid("width table offset"))?;
            let end = start.checked_add(num_glyphs).ok_or_else(|| invalid("width table size"))?;
            let bytes = data.get(start..end).ok_or_else(|| invalid("width table overruns the member"))?;
            widths = bytes.to_vec();
            // GetGlyphWidth_VariableWidth's fallback reads widths[427]
            // whenever an index lands past the table; a font whose
            // table cannot serve that read would be the game's
            // out-of-bounds access, not ours.
            if widths.len() <= FALLBACK_GLYPH_INDEX as usize {
                return Err(invalid("font width table is too short for the fallback glyph"));
            }
        }

        let glyph_bytes = 16 * usize::from(glyph_width) * usize::from(glyph_height);
        let glyphs_data = data
            .get(usize::try_from(header_size).map_err(|_| invalid("glyph data offset"))?..)
            .ok_or_else(|| invalid("glyph data overruns the member"))?;
        if glyphs_data.len() < num_glyphs
            .checked_mul(glyph_bytes)
            .ok_or_else(|| invalid("glyph data size"))?
        {
            return Err(invalid("glyph data is truncated"));
        }
        let mut glyphs = Vec::with_capacity(num_glyphs);
        for i in 0..num_glyphs {
            let src = &glyphs_data[i * glyph_bytes..][..glyph_bytes];
            glyphs.push(decompress_glyph(src, glyph_width, glyph_height));
        }
        Ok(Self {
            fixed_width,
            fixed_height,
            glyph_width,
            glyph_height,
            num_glyphs: num_glyphs as u32,
            widths,
            glyphs,
        })
    }

    /// The font's per-glyph advance for the (1-based) character `id`
    /// — `TryLoadGlyph`'s remap then `GetGlyphWidth`: indices within
    /// the table take `id - 1`, anything beyond takes the fallback
    /// glyph 427's width.
    #[must_use]
    pub fn char_width(&self, id: u16) -> u8 {
        let index = self.index_of(id);
        self.width_at(index)
    }

    /// The advance of glyph `index` (0-based) — `GetGlyphWidth_FixedWidth`
    /// when no table, else `GetGlyphWidth_VariableWidth` with its
    /// fallback.
    #[must_use]
    pub fn width_at(&self, index: u32) -> u8 {
        if self.widths.is_empty() {
            return self.fixed_width;
        }
        let i = if usize::try_from(index).unwrap_or(usize::MAX) < self.widths.len() {
            index as usize
        } else {
            FALLBACK_GLYPH_INDEX as usize
        };
        self.widths[i]
    }

    /// The (0-based) glyph index character `id` renders — pret's
    /// `TryLoadGlyph` remap: `id - 1` while within the table
    /// (its `<=` makes the last glyph reachable), else the fallback
    /// glyph 427.
    #[must_use]
    pub fn index_of(&self, id: u16) -> u32 {
        if u32::from(id) <= self.num_glyphs {
            u32::from(id) - 1
        } else {
            FALLBACK_GLYPH_INDEX
        }
    }

    /// One glyph's decoded levels: a fixed 16×16 block in the tile
    /// layout this module's header documents, plus its pixel
    /// dimensions — the `GlyphInfo` of `TryLoadGlyph`.
    #[must_use]
    pub fn glyph(&self, id: u16) -> Glyph {
        let index = self.index_of(id);
        let pixels = self
            .glyphs
            .get(index as usize)
            .copied()
            .unwrap_or([0; 16 * 16]);
        Glyph {
            pixels,
            width: self.width_at(index),
            height: self.fixed_height,
            tile_width: self.glyph_width,
            tile_height: self.glyph_height,
        }
    }

    /// The font's glyph count.
    #[must_use]
    pub fn num_glyphs(&self) -> u32 {
        self.num_glyphs
    }

    /// The pixel height every glyph of this font copies.
    #[must_use]
    pub fn fixed_height(&self) -> u8 {
        self.fixed_height
    }

    /// `GetStringWidth` (`font_data.c`): the advance of a whole
    /// unit string, control codes skipped, one `letter_spacing`
    /// subtracted at the end (the game counts a trailing spacing it
    /// never draws).
    #[must_use]
    pub fn string_width(&self, units: &[u16], letter_spacing: u8) -> u32 {
        let mut ret = 0;
        let mut i = 0;
        while i < units.len() && units[i] != EOS {
            if units[i] == EXT_CTRL_CODE_BEGIN {
                i += skip_ctrl(units, i);
                if i < units.len() && units[i] != EOS {
                    continue;
                }
                break;
            }
            ret += u32::from(self.char_width(units[i])) + u32::from(letter_spacing);
            i += 1;
        }
        ret.saturating_sub(u32::from(letter_spacing))
    }

    /// `GetStringWidthFirstLine`: like [`Self::string_width`] but
    /// stopping at the first linefeed.
    #[must_use]
    pub fn first_line_width(&self, units: &[u16], letter_spacing: u8) -> u32 {
        let mut ret = 0;
        let mut i = 0;
        while i < units.len() && units[i] != EOS && units[i] != CHAR_LF {
            if units[i] == EXT_CTRL_CODE_BEGIN {
                i += skip_ctrl(units, i);
                if i < units.len() && units[i] != EOS && units[i] != CHAR_LF {
                    continue;
                }
                break;
            }
            ret += u32::from(self.char_width(units[i])) + u32::from(letter_spacing);
            i += 1;
        }
        ret.saturating_sub(u32::from(letter_spacing))
    }

    /// `GetStringWidthMultiline`: the widest line's width.
    #[must_use]
    pub fn multiline_width(&self, units: &[u16], letter_spacing: u8) -> u32 {
        let mut cur: u32 = 0;
        let mut best: u32 = 0;
        let mut i = 0;
        while i < units.len() && units[i] != EOS {
            if units[i] == EXT_CTRL_CODE_BEGIN {
                i += skip_ctrl(units, i);
            } else if units[i] == CHAR_LF {
                if best < cur.saturating_sub(u32::from(letter_spacing)) {
                    best = cur - u32::from(letter_spacing);
                }
                cur = 0;
                i += 1;
            } else {
                cur += u32::from(self.char_width(units[i])) + u32::from(letter_spacing);
                i += 1;
            }
        }
        if best < cur.saturating_sub(u32::from(letter_spacing)) {
            best = cur - u32::from(letter_spacing);
        }
        best
    }
}

/// `MsgArray_SkipControlCode`'s advance from a `0xFFFE` marker:
/// `3 + size` units — the parse's counterpart in the width walkers,
/// where the game walks on faith.
fn skip_ctrl(units: &[u16], i: usize) -> usize {
    match units.get(i + 2) {
        Some(&size) => 3 + usize::from(size),
        None => units.len() - i,
    }
}

/// One decoded glyph — pret's `GlyphInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    /// Levels in the uniform 16×16 tile-major block layout (see the
    /// module header): `0` transparent, `1` foreground, `2` shadow,
    /// `3` background.
    pub pixels: [u8; 16 * 16],
    /// The advance width.
    pub width: u8,
    /// The pixel rows the copy blits (`fixedHeight`).
    pub height: u8,
    /// The glyph's width in 8px tiles (1–2).
    pub tile_width: u8,
    /// The glyph's height in 8px tiles (1–2).
    pub tile_height: u8,
}

impl Glyph {
    /// The level at glyph pixel `(px, py)`, clipped to the glyph's
    /// real tile extents.
    #[must_use]
    pub fn level(&self, px: usize, py: usize) -> u8 {
        if px >= usize::from(self.tile_width) * 8 || py >= usize::from(self.tile_height) * 8 {
            return 0;
        }
        let (tx, ty) = (px / 8, py / 8);
        self.pixels[(ty * 2 + tx) * 64 + (py % 8) * 8 + (px % 8)]
    }
}

/// Decodes one glyph's source tiles into the uniform block layout —
/// `DecompressGlyphTiles` over `DecompressGlyphTile`.
///
/// Source tiles come in row-major order over the glyph's `gw × gh`
/// tiles; each lands at the matching block, so an 8×16 glyph fills
/// the top and bottom-left blocks, a 16×8 the top-left and top-right.
fn decompress_glyph(src: &[u8], glyph_width: u8, glyph_height: u8) -> [u8; 16 * 16] {
    let mut out = [0u8; 16 * 16];
    for i in 0..usize::from(glyph_width) * usize::from(glyph_height) {
        let (tx, ty) = (i % usize::from(glyph_width), i / usize::from(glyph_width));
        let tile = &src[i * 16..][..16];
        for row in 0..8 {
            // One source u16 per row: high byte = pixels 0–3, low
            // byte = pixels 4–7; every 2 bits one level, the *high*
            // pair at the leftmost pixel — DecompressGlyphTile's table
            // lookups spelled out (the lookup's index arithmetic makes
            // the byte's bits 7-6 pixel 0 of the half row).
            let u = u16::from_le_bytes([tile[row * 2], tile[row * 2 + 1]]);
            let halves = [(u >> 8) as u8, (u & 0xFF) as u8];
            for (half, &byte) in halves.iter().enumerate() {
                let base = (ty * 2 + tx) * 64 + row * 8 + half * 4;
                out[base] = byte >> 6;
                out[base + 1] = (byte >> 4) & 0b11;
                out[base + 2] = (byte >> 2) & 0b11;
                out[base + 3] = byte & 0b11;
            }
        }
    }
    out
}

/// The per-font metrics the game prints with — pret's `sFontInfos`
/// (`src/font.c`), indexed by font id 0–5.
///
/// The dialog advance on linefeed is `maxLetterHeight`, *not* the
/// file's glyph height — the printer's line walk reads this table.
#[must_use]
pub fn font_info(font_id: u8) -> FontInfo {
    FONTS[usize::from(font_id.min(5))]
}

/// One font's print metrics — pret's `struct FontInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontInfo {
    /// The widest advance any glyph takes (the caret's column budget).
    pub max_letter_width: u8,
    /// The linefeed advance (pixels).
    pub max_letter_height: u8,
    /// The advance added after each glyph.
    pub letter_spacing: u8,
    /// The extra advance added on linefeed.
    pub line_spacing: u8,
    /// `unk14` — always 0 in HGSS.
    pub unk: u8,
    /// The default foreground palette index.
    pub fg_color: u8,
    /// The default background palette index.
    pub bg_color: u8,
    /// The default shadow palette index.
    pub shadow_color: u8,
}

/// `sFontInfos` (`src/font.c`) — fonts 0–4 share one metric set,
/// font 5 (the PBR menu font) is one pixel narrower.
const FONTS: [FontInfo; 6] = [
    FontInfo {
        max_letter_width: 0x0B,
        max_letter_height: 0x10,
        letter_spacing: 0,
        line_spacing: 0,
        unk: 0,
        fg_color: 1,
        bg_color: 0x0F,
        shadow_color: 2,
    },
    FontInfo {
        max_letter_width: 0x0B,
        max_letter_height: 0x10,
        letter_spacing: 0,
        line_spacing: 0,
        unk: 0,
        fg_color: 1,
        bg_color: 0x0F,
        shadow_color: 2,
    },
    FontInfo {
        max_letter_width: 0x0B,
        max_letter_height: 0x10,
        letter_spacing: 0,
        line_spacing: 0,
        unk: 0,
        fg_color: 1,
        bg_color: 0x0F,
        shadow_color: 2,
    },
    FontInfo {
        max_letter_width: 0x0B,
        max_letter_height: 0x10,
        letter_spacing: 0,
        line_spacing: 0,
        unk: 0,
        fg_color: 1,
        bg_color: 0x0F,
        shadow_color: 2,
    },
    FontInfo {
        max_letter_width: 0x0B,
        max_letter_height: 0x10,
        letter_spacing: 0,
        line_spacing: 0,
        unk: 0,
        fg_color: 1,
        bg_color: 0x0F,
        shadow_color: 2,
    },
    FontInfo {
        max_letter_width: 0x0A,
        max_letter_height: 0x10,
        letter_spacing: 0,
        line_spacing: 0,
        unk: 0,
        fg_color: 1,
        bg_color: 0x0F,
        shadow_color: 2,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The glyph count every parseable font must reach: the fallback
    /// glyph 427's width read keeps `num_glyphs >= 428` true for any
    /// font the parser accepts (the retail fonts all have 509).
    const PADDED_GLYPHS: u32 = FALLBACK_GLYPH_INDEX + 1;

    /// A minimal font, padded to the fallback coverage the parser
    /// demands: 8×8 glyphs with a width table (widths 8..12 cycling).
    fn font_bytes(num_glyphs: u32, glyph_width: u8, glyph_height: u8) -> Vec<u8> {
        let num_glyphs = num_glyphs.max(PADDED_GLYPHS);
        let header_size = 16u32 + num_glyphs; // widths after the header
        let glyph_bytes = 16 * usize::from(glyph_width) * usize::from(glyph_height);
        let mut data = Vec::new();
        data.extend_from_slice(&header_size.to_le_bytes());
        data.extend_from_slice(&16u32.to_le_bytes()); // widths at 16
        data.extend_from_slice(&num_glyphs.to_le_bytes());
        data.push(0); // fixedWidth
        data.push(glyph_height * 8); // fixedHeight
        data.push(glyph_width);
        data.push(glyph_height);
        for i in 0..num_glyphs {
            data.push(8 + (i as u8 % 5)); // widths 8..12
        }
        for i in 0..num_glyphs * glyph_bytes as u32 {
            // Deterministic pattern: level = (i / 2) % 4.
            data.push(((i / 2) % 4) as u8);
        }
        data
    }

    /// Glyph 0's data offset in a `font_bytes` fixture: right after
    /// the 16-byte header and the padded width table.
    const fn glyph0_at() -> usize {
        16 + PADDED_GLYPHS as usize
    }

    #[test]
    fn parses_header_and_widths() {
        let data = font_bytes(3, 1, 1);
        let font = Font::parse(&data).expect("parses");
        // The fixture pads to the fallback coverage; the widths it
        // asserts live in the first entries either way.
        assert_eq!(font.num_glyphs(), PADDED_GLYPHS);
        assert_eq!(font.fixed_height(), 8);
        assert_eq!(font.char_width(1), 8);
        assert_eq!(font.char_width(2), 9);
        // The last glyph is reachable: the loader's `<=` remap.
        assert_eq!(font.index_of(3), 2);
        // The padded table's last glyph is reachable through the
        // loader's `<=`, and anything beyond falls back to glyph 427.
        assert_eq!(font.index_of(PADDED_GLYPHS as u16), FALLBACK_GLYPH_INDEX);
        assert_eq!(font.index_of(PADDED_GLYPHS as u16 + 1), FALLBACK_GLYPH_INDEX);
    }

    #[test]
    fn glyph_decode_maps_half_rows() {
        let mut data = font_bytes(1, 1, 1);
        let glyph_at = glyph0_at();
        // Row 0's little-endian u16 is 0x1BE4: the high byte 0x1B is
        // the left half row, the low byte 0xE4 the right. Within each
        // byte the lookup's index arithmetic puts the *high* 2-bit
        // pair at the leftmost pixel: 0x1B = 00 01 10 11 → levels
        // 0,1,2,3; 0xE4 = 11 10 01 00 → 3,2,1,0.
        data[glyph_at..glyph_at + 2].copy_from_slice(&0x1BE4u16.to_le_bytes());
        let font = Font::parse(&data).expect("parses");
        let glyph = font.glyph(1);
        assert_eq!(
            &glyph.pixels[0..8],
            &[0, 1, 2, 3, 3, 2, 1, 0],
            "high byte is the left half row, high pair first"
        );
        // Row 1 decodes from the pattern bytes: source bytes 2-3 are
        // 0x01,0x01 → u16 0x0101, level 1 at pixel 3 of each half.
        assert_eq!(&glyph.pixels[8..16], &[0, 0, 0, 1, 0, 0, 0, 1]);
        assert_eq!(glyph.level(3, 0), 3);
        assert_eq!(glyph.level(4, 0), 3);
        assert_eq!(glyph.level(3, 1), 1);
    }

    #[test]
    fn sixteen_by_sixteen_places_four_blocks() {
        // A 16x16 glyph: 4 source tiles → 4 blocks, row-major
        // (TL, TR, BL, BR). Mark each tile's top-left pixel: u16
        // `level << 14` puts the level at the high byte's bits 7-6 —
        // pixel (0,0) of the block.
        let mut data = font_bytes(1, 2, 2);
        let glyph_at = glyph0_at();
        for (tile, level) in [(0usize, 1u8), (1, 2), (2, 3), (3, 1)] {
            let at = glyph_at + tile * 16;
            data[at..at + 2].copy_from_slice(&(u16::from(level) << 14).to_le_bytes());
        }
        let font = Font::parse(&data).expect("parses");
        let glyph = font.glyph(1);
        assert_eq!(glyph.level(0, 0), 1, "TL");
        assert_eq!(glyph.level(8, 0), 2, "TR");
        assert_eq!(glyph.level(0, 8), 3, "BL");
        assert_eq!(glyph.level(8, 8), 1, "BR");
        assert_eq!(glyph.height, 16);
    }

    #[test]
    fn width_walkers_skip_control_codes() {
        let data = font_bytes(10, 1, 1);
        let font = Font::parse(&data).expect("parses");
        // Three chars, a skipped control block, one char. Chars 1, 2,
        // 3, 4 have widths 8, 9, 10, 11.
        let units = [1u16, 2, 3, 0xFFFE, 0xFF00, 1, 0x0003, 4, EOS];
        assert_eq!(font.string_width(&units, 0), 8 + 9 + 10 + 11);
        // First line stops at LF.
        let units = [1, 2, CHAR_LF, 3, EOS];
        assert_eq!(font.first_line_width(&units, 0), 8 + 9);
        // Multiline takes the widest line.
        let units = [1, 1, CHAR_LF, 1, 2, 3, EOS];
        assert_eq!(font.multiline_width(&units, 0), 8 + 9 + 10);
        // Letter spacing adds per glyph and comes off once at the end.
        let units = [1, 2, EOS];
        assert_eq!(font.string_width(&units, 2), 8 + 2 + 9 + 2 - 2);
    }

    #[test]
    fn rejects_broken_fonts() {
        // Truncated header.
        assert!(Font::parse(&[0u8; 8]).is_err());
        // Glyph shape beyond the game's 2x2 table.
        let mut data = font_bytes(1, 1, 1);
        data[14] = 3;
        assert!(Font::parse(&data).is_err());
        // A width table too short for the fallback glyph read: the
        // fixture pads to 428, so shrink the declared count instead.
        let mut data = font_bytes(3, 1, 1);
        data[8..12].copy_from_slice(&3u32.to_le_bytes());
        assert!(Font::parse(&data).is_err());
        // Truncated glyph data.
        let mut data = font_bytes(2, 1, 1);
        data.truncate(data.len() - 16);
        assert!(Font::parse(&data).is_err());
        // fixedHeight beyond the glyph's tiles.
        let mut data = font_bytes(1, 1, 1);
        data[13] = 16;
        assert!(Font::parse(&data).is_err());
    }

    #[test]
    fn font_info_matches_pret() {
        for id in 0..5 {
            let info = font_info(id);
            assert_eq!((info.max_letter_width, info.max_letter_height), (0x0B, 0x10));
            assert_eq!(
                (info.fg_color, info.bg_color, info.shadow_color),
                (1, 0x0F, 2)
            );
        }
        assert_eq!(font_info(5).max_letter_width, 0x0A);
    }
}