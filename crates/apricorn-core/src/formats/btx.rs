//! BTX — NNS-G3D texture archive (`NSBTX`, magic `BTX0`).
//!
//! Unlike the 2D formats (NCGR/NCLR/…, see [`crate::formats`]) a BTX is a
//! *3D* resource file: the magic is the literal `BTX0` (not reversed) and
//! the container follows the NNS-G3D `NNSG3dResFileHeader` layout instead
//! of the Nitro container scheme — after the 0x10-byte header comes a
//! block-offset *table*, not the blocks themselves.
//!
//! Every retail HeartGold file carries exactly one block, `TEX0`, at
//! file offset 0x14, which ends the file:
//!
//! ```text
//! +0x00 4  "BTX0"   +0x04 2  bom 0xFEFF   +0x06 2  version (0x0001)
//! +0x08 4  fileSize (== file length)      +0x0C 2  headerSize 0x10
//! +0x0E 2  block count (1)                +0x10 ..  u32 blockOffset[1] (0x14)
//! ```
//!
//! `TEX0` (`NNSG3dResTex`), offsets relative to the block start:
//!
//! ```text
//! +0x00 4  "TEX0"   +0x04 4  size (block ends the file)
//! +0x08   texInfo:    u32 vramKey(0), u16 sizeTex (texture bytes/8),
//!                     u16 ofsDict (0x3C), u32 flag(0), u32 ofsTex
//! +0x18   tex4x4Info: u32 vramKey(0), u16 sizeTex (always 0 — retail
//!                     ships no COMP4x4 textures), u16 ofsDict (0x3C),
//!                     u32 flag(0), u32 ofsTex (garbage — unvalidated),
//!                     u32 ofsTexPlttIdx (garbage — unvalidated)
//! +0x2C   plttInfo:   u32 vramKey(0), u16 sizePltt (palette bytes/8),
//!                     u16 flag (bit 15 = USEPLTT4), u16 ofsDict,
//!                     u16 dummy(0), u32 ofsPlttData
//! +0x3C   texture dictionary, then the palette dictionary, then the
//!         texture data (sizeTex*8), then the palette data (sizePltt*8)
//!         which ends the file — each region exactly abutting.
//! ```
//!
//! Both dictionaries are [`NNSG3dResDict`]s: a header, a binary-search
//! node array, a single entry header, the packed per-entry payloads, and
//! fixed 16-byte NUL-padded name slots (a name may fill all 16 bytes
//! with no terminator). Everything except the node contents is fully
//! derived on retail files, so [`Btx::parse`] validates all of it.
//!
//! Texture entries carry two words: `TEXIMAGE_PARAM` (image offset/8 in
//! bits 0–15, zero bits 16–19, width/height exponents in bits 20–22 and
//! 23–25 — the side is `8 << exp`, `GXTexFmt` in bits 26–28, color0-
//! transparent in bit 29, zero bits 30–31) and a second word (width in
//! bits 0–10, height in bits 11–21, unknown bits 22–30 — always zero,
//! bit 31 always set). Width and height must equal `8 << exp`.
//!
//! Two deliberate non-validations, both pinned empirically: the 4x4
//! info's data pointers are garbage on every retail file, and in 45 of
//! 1,157 files the last texture entry's declared span overruns the
//! texture data by up to 1 KiB (dummy shadow textures; the file carries
//! only the zero-filled prefix). Texture *offsets* are validated, span
//! ends are not.
//!
//! [`NNSG3dResDict`]: crate::formats::btx
//!
//! Retail HeartGold (US): 1,157 files (11 loose `.nsbtx`, 1,146 inside
//! NARCs), 14,735 texture entries and 7,729 palette entries; see
//! `docs/nitro-btx.md` for the full census.

use crate::nds::{NdsError, u16le, u32le};

/// A texture pixel format, from the SDK's `GXTexFmt` enum.
///
/// Only the five formats HeartGold actually uses are accepted;
/// COMP4x4 (whose data needs the 4x4 palette-index block the game never
/// ships) and DIRECT (16bpp, unused) are rejected at parse time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TexFmt {
    /// 3-bit alpha, 5-bit index, 8 bpp (`GX_TEXFMT_A3I5`, raw 1).
    A3i5,
    /// 4-color indexed, 4 bpp (`GX_TEXFMT_PLTT4`, raw 2).
    Pltt4,
    /// 16-color indexed, 4 bpp (`GX_TEXFMT_PLTT16`, raw 3).
    Pltt16,
    /// 256-color indexed, 8 bpp (`GX_TEXFMT_PLTT256`, raw 4).
    Pltt256,
    /// 5-bit alpha, 3-bit index, 8 bpp (`GX_TEXFMT_A5I3`, raw 6).
    A5i3,
}

impl TexFmt {
    /// Decodes the raw `GXTexFmt` value.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any value HeartGold does not use.
    pub(crate) fn from_raw(raw: u32) -> Result<Self, NdsError> {
        match raw {
            1 => Ok(Self::A3i5),
            2 => Ok(Self::Pltt4),
            3 => Ok(Self::Pltt16),
            4 => Ok(Self::Pltt256),
            6 => Ok(Self::A5i3),
            _ => Err(NdsError::Invalid {
                what: "GXTexFmt not used by HeartGold (no COMP4x4/DIRECT textures)",
            }),
        }
    }

    /// The raw `GXTexFmt` value (the inverse of [`TexFmt::from_raw`];
    /// used by the round-trip serializer).
    #[must_use]
    pub(crate) fn raw(self) -> u32 {
        match self {
            Self::A3i5 => 1,
            Self::Pltt4 => 2,
            Self::Pltt16 => 3,
            Self::Pltt256 => 4,
            Self::A5i3 => 6,
        }
    }

    /// Bits per pixel: 4 or 8.
    #[must_use]
    pub fn bpp(self) -> u8 {
        match self {
            Self::Pltt4 | Self::Pltt16 => 4,
            Self::A3i5 | Self::Pltt256 | Self::A5i3 => 8,
        }
    }
}

/// One texture dictionary entry.
///
/// `data` is clamped to the texture-data area (see the module docs on
/// the 45 retail files whose declared span overruns it).
#[derive(Debug)]
pub struct BtxTexture<'a> {
    name: &'a str,
    /// Offset of the image data within the texture-data area, in bytes.
    offset: usize,
    /// `width * height * bpp / 8`, the entry's declared data size.
    declared_size: usize,
    width: u16,
    height: u16,
    fmt: TexFmt,
    color0_transparent: bool,
    data: &'a [u8],
}

impl BtxTexture<'_> {
    /// The texture's name (16-byte dictionary slot, NUL-padded).
    #[must_use]
    pub fn name(&self) -> &str {
        self.name
    }

    /// Offset of the image data within the texture-data area, in bytes
    /// (the stored `TEXIMAGE_PARAM` field is this value divided by 8).
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The declared data size, `width * height * bpp / 8`. Retail files
    /// may carry less data than declared (see the module docs); use
    /// [`data`](Self::data) for the actual bytes.
    #[must_use]
    pub fn declared_size(&self) -> usize {
        self.declared_size
    }

    /// The texture's width in pixels (always a power-of-two multiple of
    /// 8, 8–256).
    #[must_use]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// The texture's height in pixels.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.height
    }

    /// The pixel format.
    #[must_use]
    pub fn fmt(&self) -> TexFmt {
        self.fmt
    }

    /// Whether index 0 renders as transparent (`TEXIMAGE_PARAM` bit 29).
    #[must_use]
    pub fn color0_transparent(&self) -> bool {
        self.color0_transparent
    }

    /// The image data — the declared span, clamped to the end of the
    /// texture-data area.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        self.data
    }
}

/// One palette dictionary entry.
///
/// The entry's on-disk payload is a `u16` base (the palette's offset
/// into the palette data, in 8-byte units; bits 13–15 are always zero)
/// plus a `u16` word of unknown meaning (GBATEK: "usually 0, sometimes
/// 1") — 0 on 6,117 retail entries, 1 on 1,612.
#[derive(Debug)]
pub struct BtxPalette<'a> {
    name: &'a str,
    /// Offset of the palette within the palette-data area, in bytes.
    offset: usize,
    /// The unknown second `u16` of the entry payload (0 or 1 on retail).
    word1: u16,
    data: &'a [u8],
}

impl BtxPalette<'_> {
    /// The palette's name (16-byte dictionary slot, NUL-padded).
    #[must_use]
    pub fn name(&self) -> &str {
        self.name
    }

    /// Offset of the palette within the palette-data area, in bytes.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The entry's unknown second word (0 or 1 on every retail file).
    #[must_use]
    pub fn word1(&self) -> u16 {
        self.word1
    }

    /// The palette data from this palette's offset to the end of the
    /// palette-data area. Per-palette sizes are not stored in the file;
    /// a 16-color palette occupies 32 bytes, a 4-color one 16 (see
    /// [`Btx::uses_pltt4`]).
    #[must_use]
    pub fn data(&self) -> &[u8] {
        self.data
    }
}

/// A parsed BTX texture archive. Borrows the file bytes; see
/// [`Btx::parse`].
#[derive(Debug)]
pub struct Btx<'a> {
    textures: Vec<BtxTexture<'a>>,
    palettes: Vec<BtxPalette<'a>>,
    tex_data: &'a [u8],
    pltt_data: &'a [u8],
    /// `plttInfo.flag` bit 15 (`NNS_G3D_RESPLTT_USEPLTT4`): the file's
    /// palettes are 16-byte 4-color sets.
    uses_pltt4: bool,
    /// The texture dictionary's trie node array, verbatim. The nodes are
    /// a NitroSDK writer artifact (a binary-search trie over the names)
    /// the engine never needs — lookups are linear — so the round-trip
    /// serializer copies them instead of re-running the SDK's
    /// trie-construction algorithm.
    tex_nodes: &'a [u8],
    /// The palette dictionary's trie node array, verbatim (see
    /// [`Btx::tex_nodes`]).
    pltt_nodes: &'a [u8],
    /// The `tex4x4Info` `ofsTex` garbage pointer (see the module docs:
    /// never validated on any retail file, meaningless, kept verbatim).
    t44_ofs_tex: u32,
    /// The `tex4x4Info` `ofsTexPlttIdx` garbage pointer (see
    /// [`Btx::t44_ofs_tex`]).
    t44_ofs_pltt_idx: u32,
}

/// Whether `data` begins with a BTX0 header.
#[must_use]
pub fn is_btx(data: &[u8]) -> bool {
    data.get(0..6) == Some(&[b'B', b'T', b'X', b'0', 0xFF, 0xFE])
}

/// A dictionary's derived layout (`NNSG3dResDict`), all validated.
struct Dict<'a> {
    /// `numEntry` (1–126 on retail files).
    num: usize,
    /// `sizeDictBlk` — the whole dictionary, header through name slots.
    size_blk: usize,
    /// `ofsEntry`, dictionary-relative: the single entry header.
    ofs_entry: usize,
    /// The `(numEntry + 1)` trie nodes, verbatim (a writer artifact;
    /// contents unvalidated — see [`Btx::tex_nodes`]).
    nodes: &'a [u8],
}

/// Parses and validates one dictionary.
///
/// Layout (all offsets dictionary-relative):
///
/// ```text
/// +0x00 1  revision (0)   +0x01 1  numEntry
/// +0x02 2  sizeDictBlk     +0x04 2  dummy (8 — the node-array offset)
/// +0x06 2  ofsEntry  == 12 + 4*numEntry
/// +0x08    (numEntry+1) trie nodes, 4 bytes each (contents unvalidated)
/// +ofsEntry  one entry header: u16 sizeUnit, u16 ofsName
/// +ofsEntry+4  numEntry packed payloads of sizeUnit bytes each
/// +ofsEntry+ofsName  numEntry 16-byte NUL-padded name slots
/// ```
///
/// `sizeUnit` (8 for textures, 4 for palettes), `ofsEntry`, `ofsName`
/// (`== 4 + sizeUnit*numEntry`) and `sizeDictBlk` (`== ofsEntry + 4 +
/// sizeUnit*numEntry + 16*numEntry`) are all fully derived on retail
/// files, so each is validated rather than trusted.
fn parse_dict<'a>(
    data: &'a [u8],
    off: usize,
    size_unit: usize,
) -> Result<(Dict<'a>, Vec<&'a str>), NdsError> {
    if data.len() < off + 8 {
        return Err(NdsError::Truncated {
            what: "BTX0 dictionary header",
            need: off + 8,
            got: data.len(),
        });
    }
    if data[off] != 0 {
        return Err(NdsError::Invalid {
            what: "BTX0 dictionary revision (always 0)",
        });
    }
    let num = usize::from(data[off + 1]);
    if num == 0 {
        return Err(NdsError::Invalid {
            what: "BTX0 dictionary entry count (retail files carry at least one)",
        });
    }
    let size_blk = u16le(data, off + 2)? as usize;
    if u16le(data, off + 4)? != 8 {
        return Err(NdsError::Invalid {
            what: "BTX0 dictionary dummy field (always 8)",
        });
    }
    let ofs_entry = u16le(data, off + 6)? as usize;
    if ofs_entry != 12 + 4 * num {
        return Err(NdsError::Invalid {
            what: "BTX0 dictionary ofsEntry (must follow the trie nodes)",
        });
    }
    if u16le(data, off + ofs_entry)? as usize != size_unit {
        return Err(NdsError::Invalid {
            what: "BTX0 dictionary sizeUnit",
        });
    }
    let ofs_name = u16le(data, off + ofs_entry + 2)? as usize;
    if ofs_name != 4 + size_unit * num {
        return Err(NdsError::Invalid {
            what: "BTX0 dictionary ofsName (must follow the packed entries)",
        });
    }
    if size_blk != ofs_entry + ofs_name + 16 * num {
        return Err(NdsError::Invalid {
            what: "BTX0 dictionary sizeDictBlk (must end after the name slots)",
        });
    }
    if off + size_blk > data.len() {
        return Err(NdsError::Truncated {
            what: "BTX0 dictionary",
            need: off + size_blk,
            got: data.len(),
        });
    }

    let names_base = off + ofs_entry + ofs_name;
    let mut names = Vec::with_capacity(num);
    for i in 0..num {
        let slot = names_base + 16 * i;
        let name_end = data[slot..slot + 16]
            .iter()
            .position(|&b| b == 0)
            .map_or(slot + 16, |nul| slot + nul);
        let name = data.get(slot..name_end).ok_or(NdsError::Invalid {
            what: "BTX0 dictionary name slot",
        })?;
        if !name.iter().all(|b| b.is_ascii_graphic() || *b == b'_') {
            return Err(NdsError::Invalid {
                what: "BTX0 dictionary name (not printable ASCII)",
            });
        }
        names.push(core::str::from_utf8(name).map_err(|_| NdsError::Invalid {
            what: "BTX0 dictionary name (not ASCII)",
        })?);
    }
    Ok((
        Dict {
            num,
            size_blk,
            ofs_entry,
            nodes: &data[off + 8..off + ofs_entry],
        },
        names,
    ))
}

impl<'a> Btx<'a> {
    /// Parses a complete BTX file.
    ///
    /// This enforces the full retail layout: the NNS-G3D container
    /// constants (bom, version 0x0001, one TEX0 block at 0x14 that ends
    /// the file), the zeroed `vramKey`/flag/dummy fields, the absence of
    /// COMP4x4 textures, every derived dictionary offset, the exact
    /// abutment of the two dictionaries and the two data areas, that
    /// every texture entry's width/height match their exponents, and
    /// that texture and palette offsets land inside their data areas.
    /// See the module docs for the two deliberate non-validations.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any truncated, inconsistent, or
    /// non-retail-shaped file.
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        // --- NNS-G3D container header ---
        if data.len() < 0x18 {
            return Err(NdsError::Truncated {
                what: "BTX0 container header",
                need: 0x18,
                got: data.len(),
            });
        }
        if &data[0..4] != b"BTX0" {
            return Err(NdsError::Invalid {
                what: "not a BTX0 texture archive",
            });
        }
        if u16le(data, 0x04)? != 0xFEFF {
            return Err(NdsError::Invalid {
                what: "BTX0 byte-order mark",
            });
        }
        if u16le(data, 0x06)? != 0x0001 {
            return Err(NdsError::Invalid {
                what: "BTX0 version (always 0x0001 on retail files)",
            });
        }
        if u32le(data, 0x08)? as usize != data.len() {
            return Err(NdsError::Invalid {
                what: "BTX0 file size does not match its data",
            });
        }
        if u16le(data, 0x0C)? != 0x10 {
            return Err(NdsError::Invalid {
                what: "BTX0 header size",
            });
        }
        if u16le(data, 0x0E)? != 1 {
            return Err(NdsError::Invalid {
                what: "BTX0 block count (always one TEX0 block)",
            });
        }
        if u32le(data, 0x10)? as usize != 0x14 {
            return Err(NdsError::Invalid {
                what: "BTX0 block offset (always 0x14)",
            });
        }

        // --- TEX0 block ---
        let t = 0x14;
        if &data[t..t + 4] != b"TEX0" {
            return Err(NdsError::Invalid {
                what: "BTX0 block is not a TEX0 texture block",
            });
        }
        if t + u32le(data, t + 4)? as usize != data.len() {
            return Err(NdsError::Invalid {
                what: "TEX0 block (must end the file)",
            });
        }

        // texInfo
        if u32le(data, t + 0x08)? != 0 {
            return Err(NdsError::Invalid {
                what: "TEX0 texInfo vram key (always zero)",
            });
        }
        let size_tex = u16le(data, t + 0x0C)? as usize;
        if u16le(data, t + 0x0E)? as usize != 0x3C {
            return Err(NdsError::Invalid {
                what: "TEX0 texInfo ofsDict (always 0x3C)",
            });
        }
        if u32le(data, t + 0x10)? != 0 {
            return Err(NdsError::Invalid {
                what: "TEX0 texInfo flag (always zero)",
            });
        }
        let ofs_tex = u32le(data, t + 0x14)? as usize;

        // tex4x4Info: retail ships no COMP4x4 textures (size zero, the
        // same dictionary at 0x3C); its data pointers are garbage on
        // every retail file and are never validated — but they are kept
        // verbatim so the round-trip serializer can reproduce them.
        if u32le(data, t + 0x18)? != 0 {
            return Err(NdsError::Invalid {
                what: "TEX0 tex4x4Info vram key (always zero)",
            });
        }
        if u16le(data, t + 0x1C)? != 0 {
            return Err(NdsError::Invalid {
                what: "TEX0 tex4x4Info sizeTex (HeartGold ships no COMP4x4 textures)",
            });
        }
        if u16le(data, t + 0x1E)? as usize != 0x3C {
            return Err(NdsError::Invalid {
                what: "TEX0 tex4x4Info ofsDict (always 0x3C)",
            });
        }
        if u32le(data, t + 0x20)? != 0 {
            return Err(NdsError::Invalid {
                what: "TEX0 tex4x4Info flag (always zero)",
            });
        }
        let t44_ofs_tex = u32le(data, t + 0x24)?;
        let t44_ofs_pltt_idx = u32le(data, t + 0x28)?;

        // plttInfo
        if u32le(data, t + 0x2C)? != 0 {
            return Err(NdsError::Invalid {
                what: "TEX0 plttInfo vram key (always zero)",
            });
        }
        let size_pltt = u16le(data, t + 0x30)? as usize;
        let flag = u16le(data, t + 0x32)?;
        if flag & 0x7FFF != 0 {
            return Err(NdsError::Invalid {
                what: "TEX0 plttInfo flag (only bit 15, USEPLTT4, is defined)",
            });
        }
        let pltt_dict_off = u16le(data, t + 0x34)? as usize;
        if u16le(data, t + 0x36)? != 0 {
            return Err(NdsError::Invalid {
                what: "TEX0 plttInfo dummy (always zero)",
            });
        }
        let ofs_pltt = u32le(data, t + 0x38)? as usize;

        // --- The two dictionaries, then the two data areas, abutting ---
        const TEX_DICT_OFF: usize = 0x3C;
        let (tex_dict, tex_names) = parse_dict(data, t + TEX_DICT_OFF, 8)?;
        if pltt_dict_off != TEX_DICT_OFF + tex_dict.size_blk {
            return Err(NdsError::Invalid {
                what: "TEX0 plttInfo ofsDict (must follow the texture dictionary)",
            });
        }
        let (pltt_dict, pltt_names) = parse_dict(data, t + pltt_dict_off, 4)?;
        if ofs_tex != pltt_dict_off + pltt_dict.size_blk {
            return Err(NdsError::Invalid {
                what: "TEX0 ofsTex (must follow the palette dictionary)",
            });
        }
        if ofs_pltt != ofs_tex + size_tex * 8 {
            return Err(NdsError::Invalid {
                what: "TEX0 ofsPlttData (must follow the texture data)",
            });
        }
        if t + ofs_pltt + size_pltt * 8 != data.len() {
            return Err(NdsError::Invalid {
                what: "TEX0 palette data (must end the file)",
            });
        }
        let tex_data =
            data.get(t + ofs_tex..t + ofs_tex + size_tex * 8)
                .ok_or(NdsError::Truncated {
                    what: "TEX0 texture data",
                    need: t + ofs_tex + size_tex * 8,
                    got: data.len(),
                })?;
        let pltt_data = data.get(t + ofs_pltt..).ok_or(NdsError::Truncated {
            what: "TEX0 palette data",
            need: t + ofs_pltt + 1,
            got: data.len(),
        })?;

        // --- Texture entries: TEXIMAGE_PARAM + a width/height word ---
        let entries_base = t + TEX_DICT_OFF + tex_dict.ofs_entry + 4;
        let mut textures = Vec::with_capacity(tex_dict.num);
        let mut min_offset = usize::MAX;
        for (i, &name) in tex_names.iter().enumerate() {
            let at = entries_base + 8 * i;
            let param = u32le(data, at)?;
            let word1 = u32le(data, at + 4)?;
            let offset = (param & 0xFFFF) as usize * 8;
            if param & 0x000F_0000 != 0 {
                return Err(NdsError::Invalid {
                    what: "TEXIMAGE_PARAM bits 16-19 (always zero)",
                });
            }
            let fmt = TexFmt::from_raw((param >> 26) & 7)?;
            if param >> 30 != 0 {
                return Err(NdsError::Invalid {
                    what: "TEXIMAGE_PARAM bits 30-31 (always zero)",
                });
            }
            let width_exp = (param >> 20) & 7;
            let height_exp = (param >> 23) & 7;
            let width = (word1 & 0x7FF) as u16;
            let height = ((word1 >> 11) & 0x7FF) as u16;
            if word1 & 0x7FC0_0000 != 0 {
                return Err(NdsError::Invalid {
                    what: "TEXIMAGE_PARAM word1 bits 22-30 (always zero)",
                });
            }
            if word1 >> 31 == 0 {
                return Err(NdsError::Invalid {
                    what: "TEXIMAGE_PARAM word1 bit 31 (always set)",
                });
            }
            if u32::from(width) != 8 << width_exp || u32::from(height) != 8 << height_exp {
                return Err(NdsError::Invalid {
                    what: "TEXIMAGE_PARAM size (width/height must equal 8<<exponent)",
                });
            }
            if offset >= tex_data.len() {
                return Err(NdsError::Invalid {
                    what: "texture offset (outside the texture data)",
                });
            }
            min_offset = min_offset.min(offset);
            let declared_size = width as usize * height as usize * usize::from(fmt.bpp()) / 8;
            let end = (offset + declared_size).min(tex_data.len());
            textures.push(BtxTexture {
                name,
                offset,
                declared_size,
                width,
                height,
                fmt,
                color0_transparent: param & (1 << 29) != 0,
                data: &tex_data[offset..end],
            });
        }
        if min_offset != 0 {
            return Err(NdsError::Invalid {
                what: "texture data (must start with the first texture)",
            });
        }

        // --- Palette entries: u16 base/8 + u16 word1 ---
        let entries_base = t + pltt_dict_off + pltt_dict.ofs_entry + 4;
        let mut palettes = Vec::with_capacity(pltt_dict.num);
        for (i, &name) in pltt_names.iter().enumerate() {
            let at = entries_base + 4 * i;
            let base = u16le(data, at)?;
            if base & 0xE000 != 0 {
                return Err(NdsError::Invalid {
                    what: "palette base bits 13-15 (always zero)",
                });
            }
            let offset = usize::from(base & 0x1FFF) * 8;
            if offset >= pltt_data.len() {
                return Err(NdsError::Invalid {
                    what: "palette offset (outside the palette data)",
                });
            }
            palettes.push(BtxPalette {
                name,
                offset,
                word1: u16le(data, at + 2)?,
                data: &pltt_data[offset..],
            });
        }

        Ok(Self {
            textures,
            palettes,
            tex_data,
            pltt_data,
            uses_pltt4: flag & 0x8000 != 0,
            tex_nodes: tex_dict.nodes,
            pltt_nodes: pltt_dict.nodes,
            t44_ofs_tex,
            t44_ofs_pltt_idx,
        })
    }

    /// All textures, in dictionary order.
    #[must_use]
    pub fn textures(&self) -> &[BtxTexture<'a>] {
        &self.textures
    }

    /// The number of textures.
    #[must_use]
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// All palettes, in dictionary order.
    #[must_use]
    pub fn palettes(&self) -> &[BtxPalette<'a>] {
        &self.palettes
    }

    /// The number of palettes.
    #[must_use]
    pub fn palette_count(&self) -> usize {
        self.palettes.len()
    }

    /// The whole texture-data area (every texture's bytes live here;
    /// see [`BtxTexture::offset`]).
    #[must_use]
    pub fn texture_data(&self) -> &'a [u8] {
        self.tex_data
    }

    /// The whole palette-data area (every palette's bytes live here;
    /// see [`BtxPalette::offset`]).
    #[must_use]
    pub fn palette_data(&self) -> &'a [u8] {
        self.pltt_data
    }

    /// Whether the palettes are 16-byte 4-color sets
    /// (`NNS_G3D_RESPLTT_USEPLTT4`; 216 of 1,157 retail files).
    #[must_use]
    pub fn uses_pltt4(&self) -> bool {
        self.uses_pltt4
    }

    /// The first texture named `name`, if any.
    #[must_use]
    pub fn texture_by_name(&self, name: &str) -> Option<&BtxTexture<'a>> {
        self.textures.iter().find(|tex| tex.name == name)
    }

    /// The first palette named `name`, if any.
    #[must_use]
    pub fn palette_by_name(&self, name: &str) -> Option<&BtxPalette<'a>> {
        self.palettes.iter().find(|pltt| pltt.name == name)
    }

    /// Re-serializes the parsed BTX into its container form.
    ///
    /// Byte-exact: every field the parser validates is re-derived here
    /// (the dictionary offsets, `sizeTex`/`sizePltt`, `ofsTex`/
    /// `ofsPlttData`, the `TEXIMAGE_PARAM`/`word1` bit fields, and both
    /// dictionaries' headers and offsets), so this is the round-trip
    /// half of the parser guard (`tests/roundtrip_hg.rs` re-serializes
    /// every BTX in the ROM and byte-compares against the original).
    ///
    /// The two fields that are *not* derivable are copied verbatim from
    /// the retained parse state: the dictionaries' trie node arrays
    /// (a writer artifact — see [`Btx::tex_nodes`]) and the `tex4x4Info`
    /// garbage pointers (see [`Btx::t44_ofs_tex`]). Name-slot padding
    /// after each dictionary name is re-written as zeros, which the
    /// pinned retail scan confirmed matches every file.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let n_tex = self.textures.len();
        let n_pltt = self.palettes.len();

        // Both dictionaries' layouts are fully derived (and validated at
        // parse time), so re-derive them: ofsEntry == 12 + 4*num, ofsName
        // == 4 + sizeUnit*num, sizeDictBlk == ofsEntry + ofsName + 16*num.
        let tex_ofs_entry = 12 + 4 * n_tex;
        let tex_ofs_name = 4 + 8 * n_tex;
        let tex_blk = tex_ofs_entry + tex_ofs_name + 16 * n_tex;
        let pltt_ofs_entry = 12 + 4 * n_pltt;
        let pltt_ofs_name = 4 + 4 * n_pltt;
        let pltt_blk = pltt_ofs_entry + pltt_ofs_name + 16 * n_pltt;
        let pltt_dict_off = 0x3C + tex_blk;
        let ofs_tex = pltt_dict_off + pltt_blk;
        let ofs_pltt = ofs_tex + self.tex_data.len();
        let total = 0x14 + ofs_pltt + self.pltt_data.len();

        let mut out = Vec::with_capacity(total);

        // NNS-G3D container header: one TEX0 block at 0x14 that ends the
        // file.
        out.extend_from_slice(b"BTX0");
        out.extend_from_slice(&0xFEFFu16.to_le_bytes());
        out.extend_from_slice(&0x0001u16.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&0x10u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&0x14u32.to_le_bytes());

        // TEX0 block header.
        out.extend_from_slice(b"TEX0");
        out.extend_from_slice(&((total - 0x14) as u32).to_le_bytes());

        // texInfo.
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&((self.tex_data.len() / 8) as u16).to_le_bytes());
        out.extend_from_slice(&0x3Cu16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(ofs_tex as u32).to_le_bytes());

        // tex4x4Info: no COMP4x4 data, dictionary mirrored at 0x3C, and
        // the two data pointers are the retail garbage kept verbatim.
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x3Cu16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&self.t44_ofs_tex.to_le_bytes());
        out.extend_from_slice(&self.t44_ofs_pltt_idx.to_le_bytes());

        // plttInfo.
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&((self.pltt_data.len() / 8) as u16).to_le_bytes());
        out.extend_from_slice(&(u16::from(self.uses_pltt4) << 15).to_le_bytes());
        out.extend_from_slice(&(pltt_dict_off as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(ofs_pltt as u32).to_le_bytes());

        // Texture dictionary: header, verbatim trie nodes, entry header,
        // packed TEXIMAGE_PARAM/word1 pairs, then the name slots.
        out.push(0);
        out.push(n_tex as u8);
        out.extend_from_slice(&(tex_blk as u16).to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&(tex_ofs_entry as u16).to_le_bytes());
        out.extend_from_slice(self.tex_nodes);
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&(tex_ofs_name as u16).to_le_bytes());
        for tex in &self.textures {
            // width == 8 << exp, so the exponent is the log2 of width/8
            // (both sides validated at parse time).
            let width_exp = (tex.width / 8).trailing_zeros();
            let height_exp = (tex.height / 8).trailing_zeros();
            let param = (tex.offset / 8) as u32
                | (width_exp << 20)
                | (height_exp << 23)
                | (tex.fmt.raw() << 26)
                | (u32::from(tex.color0_transparent) << 29);
            let word1 = 0x8000_0000 | (u32::from(tex.height) << 11) | u32::from(tex.width);
            out.extend_from_slice(&param.to_le_bytes());
            out.extend_from_slice(&word1.to_le_bytes());
        }
        for tex in &self.textures {
            write_name_slot(&mut out, tex.name);
        }

        // Palette dictionary: header, verbatim trie nodes, entry header,
        // packed base/word1 pairs, then the name slots.
        out.push(0);
        out.push(n_pltt as u8);
        out.extend_from_slice(&(pltt_blk as u16).to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&(pltt_ofs_entry as u16).to_le_bytes());
        out.extend_from_slice(self.pltt_nodes);
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&(pltt_ofs_name as u16).to_le_bytes());
        for pltt in &self.palettes {
            out.extend_from_slice(&((pltt.offset / 8) as u16).to_le_bytes());
            out.extend_from_slice(&pltt.word1.to_le_bytes());
        }
        for pltt in &self.palettes {
            write_name_slot(&mut out, pltt.name);
        }

        // The two data areas, verbatim (each texture's/palette's bytes
        // are slices into them).
        out.extend_from_slice(self.tex_data);
        out.extend_from_slice(self.pltt_data);
        out
    }
}

/// Appends `name` and zero-pads to the dictionary's fixed 16-byte slot.
/// A name may fill all 16 bytes with no terminator; the parse guarantees
/// the length fits, and the retail scan pinned the padding as zeros.
fn write_name_slot(out: &mut Vec<u8>, name: &str) {
    out.extend_from_slice(name.as_bytes());
    out.resize(out.len() + (16 - name.len()), 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two textures (16×16 PLTT16, then 8×8 A5I3 at offset 128) and two
    /// palettes (at 0 and 32), laid out exactly as a retail file.
    ///
    /// Fixed layout the negative tests patch into:
    fn build_btx() -> Vec<u8> {
        const TEX_A_SIZE: usize = 16 * 16 / 2; // PLTT16, 4bpp
        const TEX_B_SIZE: usize = 8 * 8; // A5I3, 8bpp
        const TEX_DATA: usize = TEX_A_SIZE + TEX_B_SIZE; // 192
        const PLTT_DATA: usize = 64; // two 16-color palettes

        // Header + block table (0x18).
        // Dictionary headers sit at 0x50 (texture) and 0x98 (palette);
        // texture entries at 0x68/0x70, palette entries at 0xB0/0xB4;
        // texture data 0xD8..0x198, palette data 0x198..0x1D8 (file end).
        let mut rom = vec![0u8; 0x18];
        rom[0..4].copy_from_slice(b"BTX0");
        rom[4..6].copy_from_slice(&0xFEFFu16.to_le_bytes());
        rom[6..8].copy_from_slice(&0x0001u16.to_le_bytes());
        rom[0x0C..0x0E].copy_from_slice(&0x10u16.to_le_bytes());
        rom[0x0E..0x10].copy_from_slice(&1u16.to_le_bytes());
        rom[0x10..0x14].copy_from_slice(&0x14u32.to_le_bytes());

        // texInfo (0x1C..0x2C): sizeTex 192/8, ofsDict 0x3C.
        let mut tex_info = vec![0u8; 0x10];
        tex_info[4..6].copy_from_slice(&((TEX_DATA / 8) as u16).to_le_bytes());
        tex_info[6..8].copy_from_slice(&0x3Cu16.to_le_bytes());

        // tex4x4Info (0x2C..0x40): zero size, dict mirrors 0x3C.
        let mut t44_info = vec![0u8; 0x14];
        t44_info[6..8].copy_from_slice(&0x3Cu16.to_le_bytes());

        // plttInfo (0x40..0x50): sizePltt 64/8, ofsDict patched below.
        let mut pltt_info = vec![0u8; 0x10];
        pltt_info[4..6].copy_from_slice(&((PLTT_DATA / 8) as u16).to_le_bytes());
        // flag stays 0 (16-color palettes, not PLTT4).

        // Texture dictionary at 0x50: 2 entries, sizeUnit 8.
        let num = 2usize;
        let tex_ofs_entry = 12 + 4 * num; // 20
        let tex_ofs_name = 4 + 8 * num; // 20
        let tex_blk = tex_ofs_entry + tex_ofs_name + 16 * num; // 72
        let mut tex_dict = vec![0u8; 8 + 4 * (num + 1)]; // header + nodes
        tex_dict[1] = num as u8;
        tex_dict[2..4].copy_from_slice(&(tex_blk as u16).to_le_bytes());
        tex_dict[4..6].copy_from_slice(&8u16.to_le_bytes()); // dummy
        tex_dict[6..8].copy_from_slice(&(tex_ofs_entry as u16).to_le_bytes());
        let mut tex_rest = vec![0u8; 4 + 8 * num + 16 * num]; // entry header + entries + names
        tex_rest[0..2].copy_from_slice(&8u16.to_le_bytes()); // sizeUnit
        tex_rest[2..4].copy_from_slice(&(tex_ofs_name as u16).to_le_bytes());
        // Entry 0: off 0, 16x16 PLTT16 (exp 1), color0 transparent.
        let param0: u32 = (1 << 20) | (1 << 23) | (3 << 26) | (1 << 29);
        tex_rest[4..8].copy_from_slice(&param0.to_le_bytes());
        tex_rest[8..12].copy_from_slice(&(0x8000_0000u32 | (16 << 11) | 16).to_le_bytes());
        // Entry 1: off 128, 8x8 A5I3 (fmt 6, exp 0), color0 used.
        let param1: u32 = (128 / 8) | (6 << 26);
        tex_rest[12..16].copy_from_slice(&param1.to_le_bytes());
        tex_rest[16..20].copy_from_slice(&(0x8000_0000u32 | (8 << 11) | 8).to_le_bytes());
        for (i, name) in ["test_a", "shadow"].iter().enumerate() {
            let slot = 4 + 8 * num + 16 * i;
            tex_rest[slot..slot + name.len()].copy_from_slice(name.as_bytes());
        }
        tex_dict.extend(tex_rest);

        // Palette dictionary at 0x98: 2 entries, sizeUnit 4.
        let pltt_ofs_entry = 12 + 4 * num; // 20
        let pltt_ofs_name = 4 + 4 * num; // 12
        let pltt_blk = pltt_ofs_entry + pltt_ofs_name + 16 * num; // 64
        let mut pltt_dict = vec![0u8; 8 + 4 * (num + 1)];
        pltt_dict[1] = num as u8;
        pltt_dict[2..4].copy_from_slice(&(pltt_blk as u16).to_le_bytes());
        pltt_dict[4..6].copy_from_slice(&8u16.to_le_bytes());
        pltt_dict[6..8].copy_from_slice(&(pltt_ofs_entry as u16).to_le_bytes());
        let mut pltt_rest = vec![0u8; 4 + 4 * num + 16 * num];
        pltt_rest[0..2].copy_from_slice(&4u16.to_le_bytes());
        pltt_rest[2..4].copy_from_slice(&(pltt_ofs_name as u16).to_le_bytes());
        // Entry 0: base 0, word1 0. Entry 1: base 32/8=4, word1 1.
        pltt_rest[4..6].copy_from_slice(&0u16.to_le_bytes());
        pltt_rest[6..8].copy_from_slice(&0u16.to_le_bytes());
        pltt_rest[8..10].copy_from_slice(&4u16.to_le_bytes());
        pltt_rest[10..12].copy_from_slice(&1u16.to_le_bytes());
        for (i, name) in ["pal_a", "pal_b"].iter().enumerate() {
            let slot = 4 + 4 * num + 16 * i;
            pltt_rest[slot..slot + name.len()].copy_from_slice(name.as_bytes());
        }
        pltt_dict.extend(pltt_rest);

        // Texture data, then palette data, both recognizable bytes.
        let tex_data: Vec<u8> = (0..TEX_DATA).map(|i| (0xA0 + i) as u8).collect();
        let pltt_data: Vec<u8> = (0..PLTT_DATA).map(|i| (0x10 + i * 3) as u8).collect();

        // Assemble: header(0x10) + block table(4) + TEX0(8) + infos(0x34)
        // + dicts + data.
        let tex_data_start =
            0x14 + 8 + tex_info.len() + t44_info.len() + pltt_info.len() + tex_blk + pltt_blk;
        let pltt_data_start = tex_data_start + TEX_DATA;
        let total = pltt_data_start + PLTT_DATA;
        rom.resize(total, 0);
        rom[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        let mut off = 0x14;
        rom[off..off + 4].copy_from_slice(b"TEX0");
        rom[off + 4..off + 8].copy_from_slice(&((total - 0x14) as u32).to_le_bytes());
        off += 8;
        rom[off..off + tex_info.len()].copy_from_slice(&tex_info);
        let ofs_tex = tex_data_start - 0x14;
        rom[off + 12..off + 16].copy_from_slice(&(ofs_tex as u32).to_le_bytes());
        off += tex_info.len();
        rom[off..off + t44_info.len()].copy_from_slice(&t44_info);
        off += t44_info.len();
        rom[off..off + pltt_info.len()].copy_from_slice(&pltt_info);
        let pltt_dict_off = 0x3C + tex_blk;
        rom[off + 8..off + 10].copy_from_slice(&(pltt_dict_off as u16).to_le_bytes());
        let ofs_pltt = pltt_data_start - 0x14;
        rom[off + 12..off + 16].copy_from_slice(&(ofs_pltt as u32).to_le_bytes());
        off += pltt_info.len();
        rom[off..off + tex_blk].copy_from_slice(&tex_dict);
        off += tex_blk;
        rom[off..off + pltt_blk].copy_from_slice(&pltt_dict);
        off += pltt_blk;
        rom[off..off + TEX_DATA].copy_from_slice(&tex_data);
        off += TEX_DATA;
        rom[off..off + PLTT_DATA].copy_from_slice(&pltt_data);
        rom
    }

    #[test]
    fn parses_container_dicts_entries_and_data() {
        let data = build_btx();
        assert!(is_btx(&data));
        let btx = Btx::parse(&data).expect("synthetic BTX must parse");

        assert!(!btx.uses_pltt4());
        assert_eq!(btx.texture_count(), 2);
        assert_eq!(btx.palette_count(), 2);
        assert_eq!(btx.texture_data().len(), 192);
        assert_eq!(btx.palette_data().len(), 64);

        let a = &btx.textures()[0];
        assert_eq!(a.name(), "test_a");
        assert_eq!((a.width(), a.height()), (16, 16));
        assert_eq!(a.fmt(), TexFmt::Pltt16);
        assert!(a.color0_transparent());
        assert_eq!(a.offset(), 0);
        assert_eq!(a.declared_size(), 128);
        assert_eq!(a.data(), &data[0xD8..0xD8 + 128]);
        assert_eq!(a.data()[0], 0xA0);

        let b = &btx.textures()[1];
        assert_eq!(b.name(), "shadow");
        assert_eq!((b.width(), b.height()), (8, 8));
        assert_eq!(b.fmt(), TexFmt::A5i3);
        assert!(!b.color0_transparent());
        assert_eq!(b.offset(), 128);
        assert_eq!(b.data(), &data[0xD8 + 128..0xD8 + 128 + 64]);

        let pal_a = &btx.palettes()[0];
        assert_eq!(pal_a.name(), "pal_a");
        assert_eq!(pal_a.offset(), 0);
        assert_eq!(pal_a.word1(), 0);
        assert_eq!(pal_a.data().len(), 64);
        assert_eq!(pal_a.data()[0], 0x10);

        let pal_b = &btx.palettes()[1];
        assert_eq!(pal_b.name(), "pal_b");
        assert_eq!(pal_b.offset(), 32);
        assert_eq!(pal_b.word1(), 1);
        assert_eq!(pal_b.data().len(), 32);

        assert_eq!(
            btx.texture_by_name("shadow").map(BtxTexture::name),
            Some("shadow")
        );
        assert_eq!(
            btx.palette_by_name("pal_b").map(BtxPalette::offset),
            Some(32)
        );
        assert!(btx.texture_by_name("missing").is_none());

        assert_eq!(btx.to_bytes(), data, "round-trips byte-exact");
    }

    #[test]
    fn clamps_overrunning_texture_data() {
        // Retail-style overrun: shrink the texture data (sizeTex and
        // ofsPlttData) so entry 1's declared span pokes past its end.
        // The parser must clamp the slice instead of rejecting the file.
        let mut data = build_btx();
        let tex_bytes = 128 + 32; // only 32 of shadow's 64 bytes exist
        data[0x1C + 4..0x1C + 6].copy_from_slice(&((tex_bytes / 8) as u16).to_le_bytes());
        let new_pltt = 0xD8 + tex_bytes;
        data[0x40 + 12..0x40 + 16].copy_from_slice(&((new_pltt - 0x14) as u32).to_le_bytes());
        // Move the palette data up and shrink the file.
        data.truncate(new_pltt);
        data.extend_from_slice(&[0x55u8; 64]);
        let total = data.len();
        data[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        data[0x14 + 4..0x14 + 8].copy_from_slice(&((total - 0x14) as u32).to_le_bytes());

        let btx = Btx::parse(&data).expect("overrunning retail shape must parse");
        let shadow = &btx.textures()[1];
        assert_eq!(shadow.declared_size(), 64);
        assert_eq!(shadow.data().len(), 32);
        assert_eq!(btx.texture_data().len(), 160);

        assert_eq!(btx.to_bytes(), data, "clamped file round-trips byte-exact");
    }

    #[test]
    fn round_trips_verbatim_writer_artifacts() {
        // The two tex4x4Info data pointers are garbage on every retail
        // file — never validated, never derived — so the serializer must
        // copy them verbatim. Patch nonzero values in and confirm they
        // survive the round trip (along with the trie nodes, which
        // build_btx leaves mostly zero).
        let mut data = build_btx();
        data[0x14 + 0x24..0x14 + 0x28].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        data[0x14 + 0x28..0x14 + 0x2C].copy_from_slice(&0x0BAD_F00Du32.to_le_bytes());
        data[0x14 + 0x3C + 8] = 0x7F; // texture trie node byte

        let btx = Btx::parse(&data).expect("garbage-patched BTX must parse");
        assert_eq!(btx.to_bytes(), data, "garbage pointers and nodes survive");
    }

    #[test]
    fn rejects_broken_btx() {
        let good = build_btx();

        let mut bad_magic = good.clone();
        bad_magic[0..4].copy_from_slice(b"XTB0");
        assert!(!is_btx(&bad_magic));
        assert!(Btx::parse(&bad_magic).is_err());

        assert!(Btx::parse(&good[..good.len() - 1]).is_err());

        let u16at = |rom: &mut [u8], at: usize, value: u16| {
            rom[at..at + 2].copy_from_slice(&value.to_le_bytes());
        };
        let u32at = |rom: &mut [u8], at: usize, value: u32| {
            rom[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };

        // Container fields (header is 0x10, block table at 0x10).
        let mut bad = good.clone();
        u16at(&mut bad, 0x06, 0x0100); // wrong version
        assert!(Btx::parse(&bad).is_err());
        u16at(&mut bad, 0x06, 1);
        u16at(&mut bad, 0x0E, 2); // two blocks
        assert!(Btx::parse(&bad).is_err());
        u16at(&mut bad, 0x0E, 1);
        u32at(&mut bad, 0x10, 0x18); // block not at 0x14
        assert!(Btx::parse(&bad).is_err());
        u32at(&mut bad, 0x10, 0x14);
        u32at(&mut bad, 0x14 + 4, 0x100); // TEX0 size wrong
        assert!(Btx::parse(&bad).is_err());

        // TEX0 info fields: texInfo at 0x1C, tex4x4 at 0x2C, pltt at 0x40.
        let mut bad = good.clone();
        u32at(&mut bad, 0x1C, 1); // vram key
        assert!(Btx::parse(&bad).is_err());
        u32at(&mut bad, 0x1C, 0);
        u16at(&mut bad, 0x30, 8); // a 4x4 texture exists
        assert!(Btx::parse(&bad).is_err());
        u16at(&mut bad, 0x30, 0);
        u16at(&mut bad, 0x46, 0x1234); // undefined pltt flag bits
        assert!(Btx::parse(&bad).is_err());
        u16at(&mut bad, 0x46, 0);

        // Region abutment: each pointer off by one must fail.
        let mut bad = good.clone();
        u16at(&mut bad, 0x48, 0x84 + 4); // palette dict not adjacent
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u32at(&mut bad, 0x28, 0xC8 + 4); // ofsTex not adjacent
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u32at(&mut bad, 0x4C, 0x188 - 8); // ofsPlttData not adjacent
        assert!(Btx::parse(&bad).is_err());

        // Dictionary structure (texture dict header at 0x50).
        let mut bad = good.clone();
        bad[0x50] = 1; // revision
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u16at(&mut bad, 0x50 + 4, 4); // dummy not 8
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u16at(&mut bad, 0x50 + 6, 24); // ofsEntry not 12+4*num
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u16at(&mut bad, 0x50 + 20, 16); // sizeUnit not 8
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u16at(&mut bad, 0x50 + 20 + 2, 24); // ofsName not 4+8*num
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u16at(&mut bad, 0x50 + 2, 60); // sizeDictBlk wrong
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        bad[0x78] = 0xFF; // name not printable ASCII
        assert!(Btx::parse(&bad).is_err());

        // Texture entries at 0x68 and 0x70.
        let mut bad = good.clone();
        u32at(&mut bad, 0x68, 0x0001_0000); // bits 16-19 set
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u32at(
            &mut bad,
            0x68,
            (7 << 26) | (1 << 20) | (1 << 23) | (1 << 29),
        ); // DIRECT
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u32at(&mut bad, 0x68, (1 << 29) | (3 << 26) | (1 << 23)); // width exp 0, width 16
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u32at(&mut bad, 0x6C, 0x0000_8010); // bit 31 clear
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u32at(&mut bad, 0x6C, 0x8040_8010); // bits 22-30 set
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u32at(&mut bad, 0x70, 0x0FFF | (6 << 26)); // offset outside the data
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u32at(
            &mut bad,
            0x68,
            8 | (1 << 20) | (1 << 23) | (3 << 26) | (1 << 29),
        );
        u32at(&mut bad, 0x70, 16 | (6 << 26)); // no texture at offset 0
        assert!(Btx::parse(&bad).is_err());

        // Palette entries at 0xB0 and 0xB4.
        let mut bad = good.clone();
        u16at(&mut bad, 0xB0, 0x2000); // base bits 13-15 set
        assert!(Btx::parse(&bad).is_err());
        let mut bad = good.clone();
        u16at(&mut bad, 0xB4, 0x8); // 64 bytes, past the 64-byte area
        assert!(Btx::parse(&bad).is_err());

        assert!(!is_btx(b"not a btx at all"));
        assert!(Btx::parse(b"BTX0").is_err());
    }
}
