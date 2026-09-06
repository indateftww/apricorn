//! Engine cache chunks: the conversion step's output format.
//!
//! The raw Nitro formats are optimized for the NDS's VRAM layouts, not
//! for an engine: 4bpp pixels are nibble-packed, colors are BGR555,
//! message text is XOR-obfuscated, and OAM attributes are three packed
//! hardware registers. `apricorn-tools convert` (see
//! `docs/conversion.md`) translates every graphics and text asset into
//! one of six *cache chunks* — flat, always-valid, engine-ready records
//! — so the runtime loader does no per-frame decoding work beyond a
//! copy.
//!
//! A chunk is a self-contained little-endian file:
//!
//! ```text
//! +0x00 4  b"APCH"
//! +0x04 2  format version
//! +0x06 2  chunk kind (one of the six below)
//! +0x08 .. kind-specific payload (documented on each writer)
//! ```
//!
//! Chunks are *not* a re-serialization of the raw formats: they carry
//! exactly what the engine consumes, decoded once at conversion time —
//! pixels expanded to one byte each, palettes converted to RGBA8, MAT
//! banks decrypted, OAM unpacked into signed coordinates and named
//! fields. Every reader validates the payload structure exactly (the
//! chunk must tile to its last byte), so a truncated or corrupt cache
//! is rejected at load rather than misrendered.
//!
//! What the six kinds preserve, and why:
//!
//! * **Tiles** (NCGR) — pixel bytes expanded 4bpp → 1 byte per pixel
//!   (8bpp passes through), plus the mapping/grid/CPOS metadata the
//!   renderer needs to arrange tiles.
//! * **Palette** (NCLR) — colors converted BGR555 → RGBA8 with the
//!   SDK's 5→8-bit expansion (`v << 3 | v >> 2`). A PMCP table is
//!   *not* a decompression: per pret's `NNS_G2dLoadPaletteEx` it is a
//!   partial-load patch — VRAM sub-palette `i` reuses the stored
//!   16-color sub-palette `indices[i]` of an already-loaded palette —
//!   so the chunk keeps the patch indices verbatim instead of
//!   inventing a materialization.
//! * **Screen** (NSCR) — dimensions plus the raw map entries; a text
//!   screen's u16 entries are already the engine's native format, so
//!   only the dims/mode metadata is added.
//! * **Cells** (NCER) — every OAM entry decoded: signed 9-bit X and
//!   8-bit Y unwrapped, tile/palette/priority/shape/size/flips/mode
//!   extracted (the raw attr triple is kept alongside for the affine
//!   renderer), plus the cell-level flips, radii, bounds, VRAM-transfer
//!   blocks, UCAT attributes, and labels.
//! * **Animation** (NANR) — sequences with their label, loop point,
//!   play mode, and per-frame decoded results (cell, delay, and the
//!   SRT/translate fields where the element carries them). Frames are
//!   fixed-size 20-byte records; fields the element kind does not
//!   carry read zero.
//! * **Text** (MAT) — the bank's decrypted u16 code units, key retained
//!   for provenance. Phase 4 maps these against `charmap.txt`.

use crate::formats::{
    AnimElement, AnimResult, CellMapping, CharMapping, MsgBank, Nanr, Ncer, Ncgr, Nclr, Nscr,
    PixelFmt, PlayMode,
};
use crate::nds::{NdsError, u16le, u32le};

/// The cache chunk magic.
pub const MAGIC: [u8; 4] = *b"APCH";

/// The cache format version. Bump when any chunk layout changes.
pub const VERSION: u16 = 1;

/// The six chunk kinds, by source format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkKind {
    /// Expanded tile pixels (from NCGR).
    Tiles,
    /// RGBA colors (from NCLR).
    Palette,
    /// Screen map entries (from NSCR).
    Screen,
    /// Decoded OAM cell records (from NCER).
    Cells,
    /// Animation sequences (from NANR).
    Animation,
    /// Decrypted message units (from MAT).
    Text,
}

impl ChunkKind {
    /// The kind's payload code.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            Self::Tiles => 0,
            Self::Palette => 1,
            Self::Screen => 2,
            Self::Cells => 3,
            Self::Animation => 4,
            Self::Text => 5,
        }
    }

    /// Decodes a payload code.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for unknown codes.
    pub fn from_code(code: u16) -> Result<Self, NdsError> {
        match code {
            0 => Ok(Self::Tiles),
            1 => Ok(Self::Palette),
            2 => Ok(Self::Screen),
            3 => Ok(Self::Cells),
            4 => Ok(Self::Animation),
            5 => Ok(Self::Text),
            _ => Err(NdsError::Invalid {
                what: "unknown cache chunk kind",
            }),
        }
    }

    /// The conventional file extension for chunks of this kind.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Tiles => "tiles",
            Self::Palette => "pal",
            Self::Screen => "screen",
            Self::Cells => "cells",
            Self::Animation => "anim",
            Self::Text => "text",
        }
    }
}

/// The kind of a chunk, by its header; `None` when `data` is not a cache
/// chunk (too short, wrong magic, wrong version, or unknown kind).
#[must_use]
pub fn kind_of(data: &[u8]) -> Option<ChunkKind> {
    if data.len() < 8 || data[..4] != MAGIC || u16le(data, 4).ok()? != VERSION {
        return None;
    }
    ChunkKind::from_code(u16le(data, 6).ok()?).ok()
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Little-endian chunk builder.
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new(kind: ChunkKind) -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&kind.code().to_le_bytes());
        Self { bytes }
    }

    fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn i16(&mut self, v: i16) {
        self.u16(v as u16);
    }
}

/// The cache's CharMapping code. Enum order, not the packed register
/// bits the NCGR stores — the reader reverses this.
fn char_code(mapping: CharMapping) -> u8 {
    match mapping {
        CharMapping::TwoD => 0,
        CharMapping::OneD32K => 1,
        CharMapping::OneD64K => 2,
        CharMapping::OneD128K => 3,
        CharMapping::OneD256K => 4,
    }
}

/// Reverses [`char_code`].
fn char_from_code(code: u8) -> Result<CharMapping, NdsError> {
    match code {
        0 => Ok(CharMapping::TwoD),
        1 => Ok(CharMapping::OneD32K),
        2 => Ok(CharMapping::OneD64K),
        3 => Ok(CharMapping::OneD128K),
        4 => Ok(CharMapping::OneD256K),
        _ => Err(NdsError::Invalid {
            what: "Tiles mapping code",
        }),
    }
}

/// BGR555 → RGBA8 with the SDK's 5→8-bit expansion (`v << 3 | v >> 2`,
/// mapping 0→0 and 31→255).
fn rgba555(color: u16) -> [u8; 4] {
    let expand = |v: u16| (v << 3 | v >> 2) as u8;
    // BGR555: blue in bits 0..5, green in 5..10, red in 10..15.
    [
        expand(color >> 10 & 0x1F),
        expand(color >> 5 & 0x1F),
        expand(color & 0x1F),
        0xFF,
    ]
}

/// Encodes a NCGR as a Tiles chunk.
///
/// ```text
/// u8  fmt      0 = 4bpp source (pixels expanded), 1 = 8bpp
/// u8  mapping  CharMapping code 0..=4 (TwoD, OneD32K, OneD64K, ...)
/// u8  flags    bit0 grid dims, bit1 CPOS, bit2 VRAM-transfer
/// u8  pad
/// [u16 grid_w, u16 grid_h]   present iff bit0
/// [u16 cpos_w, u16 cpos_h]   present iff bit1
/// u32 tile_count
/// u32 pixel_count
/// [pixels]                    one byte per pixel
/// ```
#[must_use]
pub fn encode_tiles(ncgr: &Ncgr<'_>) -> Vec<u8> {
    let mut w = Writer::new(ChunkKind::Tiles);
    let fmt = u8::from(ncgr.pixel_fmt() == PixelFmt::Pltt256);
    let grid = ncgr.height().zip(ncgr.width());
    let cpos = ncgr.cpos();
    let mut flags = 0u8;
    if grid.is_some() {
        flags |= 1 << 0;
    }
    if cpos.is_some() {
        flags |= 1 << 1;
    }
    if ncgr.has_vram_transfer() {
        flags |= 1 << 2;
    }
    w.u8(fmt);
    w.u8(char_code(ncgr.mapping()));
    w.u8(flags);
    w.u8(0);
    if let Some((h, wd)) = grid {
        w.u16(wd);
        w.u16(h);
    }
    if let Some((wd, h)) = cpos {
        w.u16(wd);
        w.u16(h);
    }
    w.u32(ncgr.tile_count() as u32);
    let pixels: Vec<u8> = match ncgr.pixel_fmt() {
        PixelFmt::Pltt16 => ncgr
            .tile_data()
            .iter()
            .flat_map(|&b| [b & 0x0F, b >> 4])
            .collect(),
        PixelFmt::Pltt256 => ncgr.tile_data().to_vec(),
    };
    w.u32(pixels.len() as u32);
    w.bytes.extend_from_slice(&pixels);
    w.bytes
}

/// Encodes a NCLR as a Palette chunk.
///
/// ```text
/// u8  flags      bit0 4bpp (16-color slots), bit1 extended, bit2 PMCP
/// u8  pad, u16 pad
/// u32 color_count        stored colors, RGBA below
/// u16 pmcp_count, u16 pad
/// [RGBA × color_count]
/// [u16 × pmcp_count]     PMCP patch indices, slot order
/// ```
///
/// The PMCP indices are the partial-load patch table, in slot order:
/// VRAM sub-palette `i` reuses the stored sub-palette `pmcp[i]` of an
/// already-loaded palette (see the module docs). The chunk keeps them
/// verbatim; the engine applies the patch at load time.
#[must_use]
pub fn encode_palette(nclr: &Nclr<'_>) -> Vec<u8> {
    let mut w = Writer::new(ChunkKind::Palette);
    let mut flags = 0u8;
    if nclr.bpp() == 4 {
        flags |= 1 << 0;
    }
    if nclr.is_extended() {
        flags |= 1 << 1;
    }
    let pmcp: Vec<u16> = match nclr.pmcp() {
        Some(pmcp) => {
            flags |= 1 << 2;
            (0..usize::from(pmcp.num_palettes()))
                .map(|slot| pmcp.palette_of(slot).expect("slot < num_palettes"))
                .collect()
        }
        None => Vec::new(),
    };
    w.u8(flags);
    w.u8(0);
    w.u16(0);
    w.u32(nclr.color_count() as u32);
    w.u16(pmcp.len() as u16);
    w.u16(0);
    for chunk in nclr.palette_data().chunks_exact(2) {
        w.bytes
            .extend_from_slice(&rgba555(u16::from_le_bytes([chunk[0], chunk[1]])));
    }
    for index in pmcp {
        w.u16(index);
    }
    w.bytes
}

/// Encodes a NSCR as a Screen chunk.
///
/// ```text
/// u16 width, u16 height          in pixels
/// u16 color_mode, u16 screen_format
/// u32 entry_len
/// [entries]
/// ```
#[must_use]
pub fn encode_screen(nscr: &Nscr<'_>) -> Vec<u8> {
    let mut w = Writer::new(ChunkKind::Screen);
    w.u16(nscr.width());
    w.u16(nscr.height());
    w.u16(nscr.color_mode());
    w.u16(nscr.screen_format());
    w.u32(nscr.entries().len() as u32);
    w.bytes.extend_from_slice(nscr.entries());
    w.bytes
}

/// Encodes a NCER as a Cells chunk.
///
/// ```text
/// u16 cell_count
/// u8  mapping       CellMapping code: 0..=3 one-dimensional, 4 = TwoD
/// u8  flags         bit0 extended, bit1 VRAM transfer, bit2 UCAT
/// [u32 vram_max, (u32 src, u32 size) × cell_count]   iff bit1
/// [u32 count, u32 × count]                          iff bit2
/// per cell:
///   u16 oam_count
///   u8  radius
///   u8  flags       bit0 hflip, bit1 vflip, bit2 hvflip, bit3 bounds
///   [i16 min_x, i16 min_y, i16 max_x, i16 max_y]    iff bit3
///   per OAM (20 bytes):
///     i16 x, i16 y        (x sign-extended from the 9-bit attr1 field)
///     u16 tile, u16 a0, u16 a1, u16 a2 (raw attr triple)
///     u8 palette, u8 priority, u8 shape, u8 size
///     u8 flags (bit0 hflip, bit1 vflip, bit2 mosaic, bit3 affine)
///     u8 mode
/// u16 label_count
/// per label: u8 len, [bytes]
/// ```
#[must_use]
pub fn encode_cells(ncer: &Ncer<'_>) -> Vec<u8> {
    let mut w = Writer::new(ChunkKind::Cells);
    let mapping_code = match ncer.mapping() {
        CellMapping::OneD32K => 0u8,
        CellMapping::OneD64K => 1,
        CellMapping::OneD128K => 2,
        CellMapping::OneD256K => 3,
        CellMapping::TwoD => 4,
    };
    let mut flags = 0u8;
    if ncer.is_extended() {
        flags |= 1 << 0;
    }
    let vram = ncer.vram_transfer();
    if vram.is_some() {
        flags |= 1 << 1;
    }
    let ucat = ncer.ucat();
    if ucat.is_some() {
        flags |= 1 << 2;
    }
    w.u16(ncer.cell_count() as u16);
    w.u8(mapping_code);
    w.u8(flags);
    if let Some(vram) = vram {
        w.u32(vram.sz_byte_max);
        for &(src, size) in &vram.blocks {
            w.u32(src);
            w.u32(size);
        }
    }
    if let Some(ucat) = ucat {
        w.u32(ucat.attrs().len() as u32);
        for &attr in ucat.attrs() {
            w.u32(attr);
        }
    }
    for cell in ncer.cells() {
        let mut cell_flags = 0u8;
        if cell.h_flip() {
            cell_flags |= 1 << 0;
        }
        if cell.v_flip() {
            cell_flags |= 1 << 1;
        }
        if cell.hv_flip() {
            cell_flags |= 1 << 2;
        }
        if cell.has_bounding_rect() {
            cell_flags |= 1 << 3;
        }
        w.u16(cell.oam_count as u16);
        w.u8(cell.radius());
        w.u8(cell_flags);
        if let Some(b) = cell.bounding_box() {
            w.i16(b.min_x);
            w.i16(b.min_y);
            w.i16(b.max_x);
            w.i16(b.max_y);
        }
        for j in 0..cell.oam_count {
            let (a0, a1, a2) = cell.oam_attr(j).expect("j < oam_count");
            // X is a 9-bit two's-complement field; Y is 8-bit unsigned.
            let x_raw = a1 & 0x01FF;
            let x = if x_raw & 0x100 != 0 {
                x_raw as i16 - 0x200
            } else {
                x_raw as i16
            };
            w.i16(x);
            w.i16((a0 & 0xFF) as i16);
            w.u16(a2 & 0x03FF); // tile
            w.u16(a0);
            w.u16(a1);
            w.u16(a2);
            w.u8((a2 >> 12) as u8); // palette
            w.u8((a2 >> 10 & 3) as u8); // priority
            w.u8((a0 >> 14) as u8); // shape
            w.u8((a1 >> 14) as u8); // size
            let mut oam_flags = 0u8;
            if a1 & 0x1000 != 0 {
                oam_flags |= 1 << 0; // horizontal flip
            }
            if a1 & 0x2000 != 0 {
                oam_flags |= 1 << 1; // vertical flip
            }
            if a0 & 0x1000 != 0 {
                oam_flags |= 1 << 2; // mosaic
            }
            if a0 & 0x0100 != 0 {
                oam_flags |= 1 << 3; // rotation/scaling
            }
            w.u8(oam_flags);
            w.u8((a0 >> 10 & 3) as u8); // mode
        }
    }
    w.u16(ncer.labels().len() as u16);
    for label in ncer.labels() {
        w.u8(label.len() as u8);
        w.bytes.extend_from_slice(label.as_bytes());
    }
    w.bytes
}

/// Encodes a NANR as an Animation chunk.
///
/// ```text
/// u16 sequence_count
/// per sequence:
///   u16 frame_count, u16 loop_start
///   u8 play_mode, u8 element, u8 label_len, u8 pad
///   [label]
///   per frame (20 bytes; fields the element kind lacks are zero):
///     u16 cell, u16 delay, i16 x, i16 y,
///     u16 rotation, u16 pad, u32 scale_x, u32 scale_y
/// ```
///
/// # Errors
/// Returns an [`NdsError`] if a label exceeds 255 bytes or a frame's
/// result is missing (the NANR parser guarantees neither happens on
/// well-formed files).
pub fn encode_animation(nanr: &Nanr<'_>) -> Result<Vec<u8>, NdsError> {
    let mut w = Writer::new(ChunkKind::Animation);
    w.u16(nanr.sequence_count() as u16);
    for seq in nanr.sequences() {
        let label = seq.label();
        if label.len() > 0xFF {
            return Err(NdsError::Invalid {
                what: "NANR sequence label exceeds 255 bytes",
            });
        }
        // The raw on-disk encodings: play mode 1..=4, element 0..=2.
        let play_mode = match seq.play_mode() {
            PlayMode::Forward => 1u8,
            PlayMode::ForwardLoop => 2,
            PlayMode::Reverse => 3,
            PlayMode::ReverseLoop => 4,
        };
        let element = match seq.element() {
            AnimElement::Cell => 0u8,
            AnimElement::Srt => 1,
            AnimElement::Translate => 2,
        };
        w.u16(seq.frame_count() as u16);
        w.u16(seq.loop_start());
        w.u8(play_mode);
        w.u8(element);
        w.u8(label.len() as u8);
        w.u8(0);
        w.bytes.extend_from_slice(label.as_bytes());
        for j in 0..seq.frame_count() {
            let delay = seq.frames()[j].delay;
            let mut frame = [0u8; 20];
            let mut put = |off: usize, bytes: &[u8]| {
                frame[off..off + bytes.len()].copy_from_slice(bytes);
            };
            match seq.result(j) {
                Some(AnimResult::Cell { cell }) => put(0, &cell.to_le_bytes()),
                Some(AnimResult::Translate { cell, x, y }) => {
                    put(0, &cell.to_le_bytes());
                    put(4, &x.to_le_bytes());
                    put(6, &y.to_le_bytes());
                }
                Some(AnimResult::Srt {
                    cell,
                    rotation,
                    scale_x,
                    scale_y,
                    x,
                    y,
                }) => {
                    put(0, &cell.to_le_bytes());
                    put(4, &x.to_le_bytes());
                    put(6, &y.to_le_bytes());
                    put(8, &rotation.to_le_bytes());
                    put(12, &scale_x.to_le_bytes());
                    put(16, &scale_y.to_le_bytes());
                }
                None => {
                    return Err(NdsError::Invalid {
                        what: "NANR frame result is missing",
                    });
                }
            }
            put(2, &delay.to_le_bytes());
            w.bytes.extend_from_slice(&frame);
        }
    }
    Ok(w.bytes)
}

/// Encodes a MAT bank as a Text chunk.
///
/// ```text
/// u32 message_count
/// u16 key, u16 pad
/// per message: u32 unit_count, [u16 units]
/// ```
#[must_use]
pub fn encode_text(bank: &MsgBank<'_>) -> Vec<u8> {
    let mut w = Writer::new(ChunkKind::Text);
    w.u32(bank.message_count() as u32);
    w.u16(bank.key());
    w.u16(0);
    for message in bank.messages() {
        w.u32(message.len() as u32);
        for &unit in message {
            w.u16(unit);
        }
    }
    w.bytes
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// A decoded Tiles chunk. See [`encode_tiles`] for the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tiles {
    fmt: u8,
    mapping: CharMapping,
    grid: Option<(u16, u16)>,
    cpos: Option<(u16, u16)>,
    vram_transfer: bool,
    tile_count: u32,
    pixels: Vec<u8>,
}

impl Tiles {
    /// Parses a Tiles chunk.
    ///
    /// # Errors
    /// Returns an [`NdsError`] if the header, flags, counts, or pixel
    /// data are inconsistent — including trailing bytes.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        let mut c = Cursor::new(data, ChunkKind::Tiles)?;
        let fmt = c.u8()?;
        if fmt > 1 {
            return Err(NdsError::Invalid {
                what: "Tiles pixel format",
            });
        }
        let mapping = char_from_code(c.u8()?)?;
        let flags = c.u8()?;
        c.u8()?; // pad
        if flags & !0b111 != 0 {
            return Err(NdsError::Invalid {
                what: "Tiles flags",
            });
        }
        let grid = flags & 1 << 0 != 0;
        let cpos = flags & 1 << 1 != 0;
        let vram_transfer = flags & 1 << 2 != 0;
        let grid = grid.then(|| Ok((c.u16()?, c.u16()?))).transpose()?;
        let cpos = cpos.then(|| Ok((c.u16()?, c.u16()?))).transpose()?;
        let tile_count = c.u32()?;
        let pixel_count = c.u32()?;
        // One byte per pixel, 64 pixels per 8×8 tile regardless of the
        // source's packing.
        if u64::from(pixel_count) != u64::from(tile_count) * 64 {
            return Err(NdsError::Invalid {
                what: "Tiles pixel count does not match the tile count",
            });
        }
        let pixels = c.take(pixel_count as usize)?.to_vec();
        c.done()?;
        Ok(Self {
            fmt,
            mapping,
            grid,
            cpos,
            vram_transfer,
            tile_count,
            pixels,
        })
    }

    /// Whether the source NCGR was 4bpp (pixels hold 0..=15).
    #[must_use]
    pub fn is_4bpp(&self) -> bool {
        self.fmt == 0
    }

    /// The OBJ VRAM mapping mode.
    #[must_use]
    pub fn mapping(&self) -> CharMapping {
        self.mapping
    }

    /// The sheet's tile grid `(width, height)`, if the source had one.
    #[must_use]
    pub fn grid(&self) -> Option<(u16, u16)> {
        self.grid
    }

    /// The CPOS sheet size `(width, height)` in tiles, if present.
    #[must_use]
    pub fn cpos(&self) -> Option<(u16, u16)> {
        self.cpos
    }

    /// Whether the source NCGR carries a VRAM-transfer area.
    #[must_use]
    pub fn has_vram_transfer(&self) -> bool {
        self.vram_transfer
    }

    /// The number of 8×8 tiles.
    #[must_use]
    pub fn tile_count(&self) -> u32 {
        self.tile_count
    }

    /// The expanded pixels, one byte each, in tile order.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// A decoded Palette chunk. See [`encode_palette`] for the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    bpp4: bool,
    extended: bool,
    colors: Vec<[u8; 4]>,
    pmcp: Vec<u16>,
}

impl Palette {
    /// Parses a Palette chunk.
    ///
    /// # Errors
    /// Returns an [`NdsError`] on inconsistent flags, counts, or a
    /// payload that does not tile exactly.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        let mut c = Cursor::new(data, ChunkKind::Palette)?;
        let flags = c.u8()?;
        if flags & !0b111 != 0 {
            return Err(NdsError::Invalid {
                what: "Palette flags",
            });
        }
        let bpp4 = flags & 1 << 0 != 0;
        let extended = flags & 1 << 1 != 0;
        let has_pmcp = flags & 1 << 2 != 0;
        c.u8()?;
        c.u16()?;
        let color_count = c.u32()? as usize;
        // The count is always present; the flag only says whether the
        // indices follow the colors.
        let pmcp_count = usize::from(c.u16()?);
        if !has_pmcp && pmcp_count != 0 {
            return Err(NdsError::Invalid {
                what: "Palette PMCP count without the PMCP flag",
            });
        }
        c.u16()?;
        let mut colors = Vec::with_capacity(color_count);
        for _ in 0..color_count {
            colors.push([c.u8()?, c.u8()?, c.u8()?, c.u8()?]);
        }
        let mut pmcp = Vec::with_capacity(pmcp_count);
        for _ in 0..pmcp_count {
            pmcp.push(c.u16()?);
        }
        c.done()?;
        Ok(Self {
            bpp4,
            extended,
            colors,
            pmcp,
        })
    }

    /// Whether the slots are 16-color (32-byte) — 4bpp source.
    #[must_use]
    pub fn is_16_color(&self) -> bool {
        self.bpp4
    }

    /// Whether the source was an extended-palette bank.
    #[must_use]
    pub fn is_extended(&self) -> bool {
        self.extended
    }

    /// The colors as RGBA8, in stored order.
    #[must_use]
    pub fn rgba(&self) -> &[[u8; 4]] {
        &self.colors
    }

    /// The PMCP patch table, in slot order: VRAM sub-palette `i`
    /// reuses the stored sub-palette `pmcp()[i]` (empty when the
    /// source had no PMCP).
    #[must_use]
    pub fn pmcp(&self) -> &[u16] {
        &self.pmcp
    }
}

/// A decoded Screen chunk. See [`encode_screen`] for the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    width: u16,
    height: u16,
    color_mode: u16,
    screen_format: u16,
    entries: Vec<u8>,
}

impl Screen {
    /// Parses a Screen chunk.
    ///
    /// # Errors
    /// Returns an [`NdsError`] on truncated data or an entry area that
    /// does not tile the chunk.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        let mut c = Cursor::new(data, ChunkKind::Screen)?;
        let width = c.u16()?;
        let height = c.u16()?;
        let color_mode = c.u16()?;
        let screen_format = c.u16()?;
        let entry_len = c.u32()? as usize;
        let entries = c.take(entry_len)?.to_vec();
        // Text screens carry one u16 entry per 8×8 tile; the
        // rotation-screen variants (formats 1 and 2) are not pinned yet,
        // so their entry areas pass through unvalidated.
        if screen_format == 0 {
            let tiles = usize::from(width / 8) * usize::from(height / 8);
            if entry_len != tiles * 2 {
                return Err(NdsError::Invalid {
                    what: "Screen entry area does not cover the screen",
                });
            }
        }
        c.done()?;
        Ok(Self {
            width,
            height,
            color_mode,
            screen_format,
            entries,
        })
    }

    /// The screen width in pixels.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// The screen height in pixels.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.height
    }

    /// The NSCR color mode (`GX_BG_COLORMODE_16/256`).
    #[must_use]
    pub fn color_mode(&self) -> u16 {
        self.color_mode
    }

    /// The NSCR screen format (0 = text BG).
    #[must_use]
    pub fn screen_format(&self) -> u16 {
        self.screen_format
    }

    /// The raw map entries.
    #[must_use]
    pub fn entries(&self) -> &[u8] {
        &self.entries
    }
}

/// One decoded OAM entry: the named fields plus the raw attribute triple
/// (kept for the affine renderer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OamRecord {
    /// Screen X, sign-extended from the attr1 9-bit field.
    pub x: i16,
    /// Screen Y (the attr0 8-bit field, 0..=255).
    pub y: i16,
    /// The OBJ tile (`charName`), attr2 bits 0..10.
    pub tile: u16,
    /// The raw attribute triple as stored in the NCER.
    pub a0: u16,
    /// The raw attribute triple as stored in the NCER.
    pub a1: u16,
    /// The raw attribute triple as stored in the NCER.
    pub a2: u16,
    /// The 16-color palette slot, attr2 bits 12..16.
    pub palette: u8,
    /// The BG priority, attr2 bits 10..12.
    pub priority: u8,
    /// The shape, attr0 bits 14..16 (0 square, 1 wide, 2 tall).
    pub shape: u8,
    /// The size class, attr1 bits 14..16 (meaning depends on shape).
    pub size: u8,
    /// Whether the sprite is horizontally flipped (attr1 bit 12).
    pub h_flip: bool,
    /// Whether the sprite is vertically flipped (attr1 bit 13).
    pub v_flip: bool,
    /// Whether mosaic is on (attr0 bit 12).
    pub mosaic: bool,
    /// Whether the sprite uses rotation/scaling (attr0 bit 8).
    pub affine: bool,
    /// The blend mode (attr0 bits 10..12: 0 normal, 1 semi-transparent,
    /// 2 OBJ window).
    pub mode: u8,
}

/// One decoded cell: its OAM records plus the cell-level fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRecord {
    /// The cell's OAM entries.
    pub oam: Vec<OamRecord>,
    /// The cell's bounding-sphere radius.
    pub radius: u8,
    /// Cell-level horizontal flip.
    pub h_flip: bool,
    /// Cell-level vertical flip.
    pub v_flip: bool,
    /// Cell-level combined HV flip.
    pub hv_flip: bool,
    /// The bounding box `(min_x, min_y, max_x, max_y)`, if stored.
    pub bounds: Option<(i16, i16, i16, i16)>,
}

/// The VRAM-transfer block, as preserved from the NCER.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VramTransferData {
    /// The maximum size any one cell transfers.
    pub sz_byte_max: u32,
    /// One `(src_offset, size)` pair per cell.
    pub blocks: Vec<(u32, u32)>,
}

/// A decoded Cells chunk. See [`encode_cells`] for the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cells {
    mapping: CellMapping,
    vram: Option<VramTransferData>,
    ucat: Vec<u32>,
    cells: Vec<CellRecord>,
    labels: Vec<String>,
}

impl Cells {
    /// Parses a Cells chunk.
    ///
    /// # Errors
    /// Returns an [`NdsError`] on any inconsistent count, flag, or
    /// label, or when the payload does not tile exactly.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        let mut c = Cursor::new(data, ChunkKind::Cells)?;
        let cell_count = c.u16()? as usize;
        let mapping = match c.u8()? {
            0 => CellMapping::OneD32K,
            1 => CellMapping::OneD64K,
            2 => CellMapping::OneD128K,
            3 => CellMapping::OneD256K,
            4 => CellMapping::TwoD,
            _ => {
                return Err(NdsError::Invalid {
                    what: "Cells mapping code",
                });
            }
        };
        let flags = c.u8()?;
        if flags & !0b111 != 0 {
            return Err(NdsError::Invalid {
                what: "Cells flags",
            });
        }
        let vram = if flags & 1 << 1 != 0 {
            let sz_byte_max = c.u32()?;
            let mut blocks = Vec::with_capacity(cell_count);
            for _ in 0..cell_count {
                blocks.push((c.u32()?, c.u32()?));
            }
            Some(VramTransferData {
                sz_byte_max,
                blocks,
            })
        } else {
            None
        };
        let ucat = if flags & 1 << 2 != 0 {
            let count = c.u32()? as usize;
            if count != cell_count {
                return Err(NdsError::Invalid {
                    what: "Cells UCAT count (one attribute per cell)",
                });
            }
            let mut attrs = Vec::with_capacity(count);
            for _ in 0..count {
                attrs.push(c.u32()?);
            }
            attrs
        } else {
            Vec::new()
        };
        let mut cells = Vec::with_capacity(cell_count);
        for _ in 0..cell_count {
            let oam_count = c.u16()? as usize;
            let radius = c.u8()?;
            let cell_flags = c.u8()?;
            if cell_flags & !0b1111 != 0 {
                return Err(NdsError::Invalid {
                    what: "Cells cell flags",
                });
            }
            let bounds = if cell_flags & 1 << 3 != 0 {
                Some((c.i16()?, c.i16()?, c.i16()?, c.i16()?))
            } else {
                None
            };
            let mut oam = Vec::with_capacity(oam_count);
            for _ in 0..oam_count {
                let x = c.i16()?;
                let y = c.i16()?;
                let tile = c.u16()?;
                let a0 = c.u16()?;
                let a1 = c.u16()?;
                let a2 = c.u16()?;
                let palette = c.u8()?;
                let priority = c.u8()?;
                let shape = c.u8()?;
                let size = c.u8()?;
                let oam_flags = c.u8()?;
                let mode = c.u8()?;
                if oam_flags & !0b1111 != 0 || mode > 2 {
                    return Err(NdsError::Invalid {
                        what: "Cells OAM flags or mode",
                    });
                }
                oam.push(OamRecord {
                    x,
                    y,
                    tile,
                    a0,
                    a1,
                    a2,
                    palette,
                    priority,
                    shape,
                    size,
                    h_flip: oam_flags & 1 << 0 != 0,
                    v_flip: oam_flags & 1 << 1 != 0,
                    mosaic: oam_flags & 1 << 2 != 0,
                    affine: oam_flags & 1 << 3 != 0,
                    mode,
                });
            }
            cells.push(CellRecord {
                oam,
                radius,
                h_flip: cell_flags & 1 << 0 != 0,
                v_flip: cell_flags & 1 << 1 != 0,
                hv_flip: cell_flags & 1 << 2 != 0,
                bounds,
            });
        }
        let label_count = c.u16()? as usize;
        let mut labels = Vec::with_capacity(label_count);
        for _ in 0..label_count {
            let len = c.u8()? as usize;
            let bytes = c.take(len)?;
            labels.push(
                core::str::from_utf8(bytes)
                    .map_err(|_| NdsError::Invalid {
                        what: "Cells label is not UTF-8",
                    })?
                    .to_owned(),
            );
        }
        c.done()?;
        Ok(Self {
            mapping,
            vram,
            ucat,
            cells,
            labels,
        })
    }

    /// The character-data mapping mode.
    #[must_use]
    pub fn mapping(&self) -> CellMapping {
        self.mapping
    }

    /// The VRAM-transfer block, if the source had one.
    #[must_use]
    pub fn vram_transfer(&self) -> Option<&VramTransferData> {
        self.vram.as_ref()
    }

    /// The per-cell UCAT attributes (empty when the source had none).
    #[must_use]
    pub fn ucat(&self) -> &[u32] {
        &self.ucat
    }

    /// The decoded cells.
    #[must_use]
    pub fn cells(&self) -> &[CellRecord] {
        &self.cells
    }

    /// The bank's labels (count is independent of the cell count).
    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }
}

/// One animation frame, fully decoded; fields the element kind does not
/// carry read zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimFrame {
    /// The NCER cell this frame shows.
    pub cell: u16,
    /// How many ticks the frame holds before the sequence advances.
    pub delay: u16,
    /// X translation (SRT and translate elements).
    pub x: i16,
    /// Y translation (SRT and translate elements).
    pub y: i16,
    /// Rotation (`rotZ`, SRT only).
    pub rotation: u16,
    /// X scale, fx32 (SRT only).
    pub scale_x: u32,
    /// Y scale, fx32 (SRT only).
    pub scale_y: u32,
}

/// One animation sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimSequence {
    /// The sequence's label.
    pub label: String,
    /// The frame index the sequence restarts from when looping.
    pub loop_start: u16,
    /// The raw play mode (1 forward, 2 forward-loop, 3 reverse,
    /// 4 reverse-loop).
    pub play_mode: u8,
    /// The raw element (0 cell, 1 SRT, 2 translate).
    pub element: u8,
    /// The sequence's frames.
    pub frames: Vec<AnimFrame>,
}

/// A decoded Animation chunk. See [`encode_animation`] for the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    sequences: Vec<AnimSequence>,
}

impl Animation {
    /// Parses an Animation chunk.
    ///
    /// # Errors
    /// Returns an [`NdsError`] on truncated data, an unknown mode or
    /// element, or a payload that does not tile exactly.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        let mut c = Cursor::new(data, ChunkKind::Animation)?;
        let sequence_count = c.u16()? as usize;
        let mut sequences = Vec::with_capacity(sequence_count);
        for _ in 0..sequence_count {
            let frame_count = c.u16()? as usize;
            let loop_start = c.u16()?;
            let play_mode = c.u8()?;
            let element = c.u8()?;
            if !(1..=4).contains(&play_mode) || element > 2 || loop_start as usize >= frame_count {
                return Err(NdsError::Invalid {
                    what: "Animation sequence mode, element, or loop point",
                });
            }
            let label_len = c.u8()? as usize;
            c.u8()?; // pad
            let label = core::str::from_utf8(c.take(label_len)?)
                .map_err(|_| NdsError::Invalid {
                    what: "Animation label is not UTF-8",
                })?
                .to_owned();
            let mut frames = Vec::with_capacity(frame_count);
            for _ in 0..frame_count {
                let cell = c.u16()?;
                let delay = c.u16()?;
                let x = c.i16()?;
                let y = c.i16()?;
                let rotation = c.u16()?;
                c.u16()?; // pad
                let scale_x = c.u32()?;
                let scale_y = c.u32()?;
                frames.push(AnimFrame {
                    cell,
                    delay,
                    x,
                    y,
                    rotation,
                    scale_x,
                    scale_y,
                });
            }
            sequences.push(AnimSequence {
                label,
                loop_start,
                play_mode,
                element,
                frames,
            });
        }
        c.done()?;
        Ok(Self { sequences })
    }

    /// The animation's sequences.
    #[must_use]
    pub fn sequences(&self) -> &[AnimSequence] {
        &self.sequences
    }
}

/// A decoded Text chunk. See [`encode_text`] for the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    key: u16,
    units: Vec<u16>,
    starts: Vec<usize>,
}

impl Text {
    /// Parses a Text chunk.
    ///
    /// # Errors
    /// Returns an [`NdsError`] on truncated data, an empty message, or
    /// a payload that does not tile exactly.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        let mut c = Cursor::new(data, ChunkKind::Text)?;
        let message_count = c.u32()? as usize;
        let key = c.u16()?;
        c.u16()?; // pad
        let mut units = Vec::new();
        let mut starts = Vec::with_capacity(message_count + 1);
        for _ in 0..message_count {
            starts.push(units.len());
            let unit_count = c.u32()? as usize;
            if unit_count == 0 {
                return Err(NdsError::Invalid {
                    what: "Text message has zero units",
                });
            }
            for _ in 0..unit_count {
                units.push(c.u16()?);
            }
        }
        starts.push(units.len());
        c.done()?;
        Ok(Self { key, units, starts })
    }

    /// The bank's stored obfuscation key, for provenance.
    #[must_use]
    pub fn key(&self) -> u16 {
        self.key
    }

    /// The number of messages.
    #[must_use]
    pub fn message_count(&self) -> usize {
        self.starts.len() - 1
    }

    /// Message `id`'s decrypted code units (EOS included).
    #[must_use]
    pub fn message(&self, id: usize) -> Option<&[u16]> {
        (id + 1 < self.starts.len()).then(|| &self.units[self.starts[id]..self.starts[id + 1]])
    }

    /// Every message's units, in order.
    pub fn messages(&self) -> impl Iterator<Item = &[u16]> {
        self.starts
            .iter()
            .zip(self.starts.iter().skip(1))
            .map(|(&a, &b)| &self.units[a..b])
    }
}

/// Bounds-checked chunk reader. Enforces the chunk header and that the
/// payload tiles exactly.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn new(data: &[u8], kind: ChunkKind) -> Result<Cursor<'_>, NdsError> {
        if kind_of(data) != Some(kind) {
            return Err(NdsError::Invalid {
                what: "cache chunk header (magic, version, or kind)",
            });
        }
        Ok(Cursor { data, pos: 8 })
    }

    fn u8(&mut self) -> Result<u8, NdsError> {
        let b = *self.data.get(self.pos).ok_or_else(|| NdsError::Truncated {
            what: "cache chunk",
            need: self.pos + 1,
            got: self.data.len(),
        })?;
        self.pos += 1;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16, NdsError> {
        let v = u16le(self.data, self.pos).map_err(|_| NdsError::Truncated {
            what: "cache chunk",
            need: self.pos + 2,
            got: self.data.len(),
        })?;
        self.pos += 2;
        Ok(v)
    }

    fn u32(&mut self) -> Result<u32, NdsError> {
        let v = u32le(self.data, self.pos).map_err(|_| NdsError::Truncated {
            what: "cache chunk",
            need: self.pos + 4,
            got: self.data.len(),
        })?;
        self.pos += 4;
        Ok(v)
    }

    fn i16(&mut self) -> Result<i16, NdsError> {
        Ok(self.u16()? as i16)
    }

    fn take(&mut self, len: usize) -> Result<&[u8], NdsError> {
        let end = self.pos.checked_add(len).ok_or(NdsError::Invalid {
            what: "cache chunk length overflows",
        })?;
        let slice = self.data.get(self.pos..end).ok_or(NdsError::Truncated {
            what: "cache chunk",
            need: end,
            got: self.data.len(),
        })?;
        self.pos = end;
        Ok(slice)
    }

    /// The payload must be consumed exactly.
    fn done(&self) -> Result<(), NdsError> {
        if self.pos != self.data.len() {
            return Err(NdsError::Invalid {
                what: "cache chunk has trailing bytes",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a NCGR: one CHAR section with a 4bpp 2×1-tile sheet,
    /// optionally followed by a CPOS section (same layout as the
    /// format module's fixture).
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
        rom[0xE..0x10].copy_from_slice(&if with_cpos { 2u16 } else { 1u16 }.to_le_bytes());

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

    /// Builds a NCLR with two stored 16-color palettes and a PMCP
    /// mapping three slots onto them (same layout as the format
    /// module's fixture), with BGR555 colors chosen to pin the 5→8-bit
    /// expansion: white, full red, 1/32 green, white.
    fn build_compressed_nclr() -> Vec<u8> {
        let colors: Vec<u8> = [0xFFFFu16, 0x7C00, 0x0020, 0xFFFF]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        let pltt_size = 8 + 0x10 + colors.len();
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
        rom[0x20..0x24].copy_from_slice(&((colors.len() * 3) as u32).to_le_bytes());
        rom[0x24..0x28].copy_from_slice(&0x10u32.to_le_bytes()); // data offset
        rom[0x28..0x28 + colors.len()].copy_from_slice(&colors);

        let off = 0x10 + pltt_size;
        rom[off..off + 4].copy_from_slice(b"PMCP");
        rom[off + 4..off + 8].copy_from_slice(&(pmcp_size as u32).to_le_bytes());
        rom[off + 8..off + 10].copy_from_slice(&3u16.to_le_bytes()); // numPalette
        rom[off + 0xA..off + 0xC].copy_from_slice(&0xBEEFu16.to_le_bytes()); // pad
        rom[off + 0xC..off + 0x10].copy_from_slice(&8u32.to_le_bytes()); // table offset
        for (i, v) in pmcp_indices.iter().enumerate() {
            rom[off + 0x10 + 2 * i..off + 0x12 + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        rom
    }

    /// Builds a NCLR with a single stored 16-color palette and no PMCP
    /// section — the common case, which must round-trip without the
    /// patch table.
    fn build_plain_nclr() -> Vec<u8> {
        let colors: Vec<u8> = [0xFFFFu16, 0x7C00, 0x0020, 0xFFFF]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        let pltt_size = 8 + 0x10 + colors.len();
        let total = 0x10 + pltt_size;

        let mut rom = vec![0u8; total];
        rom[0..4].copy_from_slice(b"RLCN");
        rom[4..6].copy_from_slice(&0xFEFFu16.to_le_bytes());
        rom[6..8].copy_from_slice(&0x0100u16.to_le_bytes());
        rom[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        rom[0xC..0xE].copy_from_slice(&0x10u16.to_le_bytes());
        rom[0xE..0x10].copy_from_slice(&1u16.to_le_bytes());

        rom[0x10..0x14].copy_from_slice(b"TTLP");
        rom[0x14..0x18].copy_from_slice(&(pltt_size as u32).to_le_bytes());
        rom[0x18..0x1C].copy_from_slice(&3u32.to_le_bytes()); // PLTT16
        rom[0x1C..0x20].copy_from_slice(&0u32.to_le_bytes()); // not extended
        rom[0x20..0x24].copy_from_slice(&(colors.len() as u32).to_le_bytes());
        rom[0x24..0x28].copy_from_slice(&0x10u32.to_le_bytes()); // data offset
        rom[0x28..0x28 + colors.len()].copy_from_slice(&colors);
        rom
    }

    /// Builds a NSCR: a 32×16-px text screen with 8 u16 entries (same
    /// layout as the format module's fixture).
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

    /// Encrypts entry `n` in place, exactly as the MAT Decrypt1 undoes
    /// (the same fixture logic the msg module's tests use).
    fn encrypt_entry(data: &mut [u8], key: u16, n: usize, offset: u32, length: u32) {
        let seed = (u64::from(key) * 765 * (n as u64 + 1)) & 0xFFFF;
        let seed = seed as u32 | (seed as u32) << 16;
        data[4 + 8 * n..8 + 8 * n].copy_from_slice(&(offset ^ seed).to_le_bytes());
        data[8 + 8 * n..12 + 8 * n].copy_from_slice(&(length ^ seed).to_le_bytes());
    }

    /// Builds a well-formed MAT (both encryption passes, EOS appended).
    fn build_mat(key: u16, messages: &[&[u16]]) -> Vec<u8> {
        const EOS: u16 = 0xFFFF;
        let mut data = vec![0u8; 4 + 8 * messages.len()];
        data[0..2].copy_from_slice(&(messages.len() as u16).to_le_bytes());
        data[2..4].copy_from_slice(&key.to_le_bytes());
        let mut cursor = data.len();
        for (n, &units) in messages.iter().enumerate() {
            let full: Vec<u16> = units.iter().copied().chain([EOS]).collect();
            encrypt_entry(&mut data, key, n, cursor as u32, full.len() as u32);
            let mut seed = ((n as u64 + 1) * 596_947) as u16;
            for &u in &full {
                data.extend_from_slice(&(u ^ seed).to_le_bytes());
                seed = seed.wrapping_add(18_749);
            }
            cursor += 2 * full.len();
        }
        data
    }

    #[test]
    fn tiles_roundtrip() {
        let data = build_ncgr(false);
        let ncgr = Ncgr::parse(&data).expect("fixture parses");
        let chunk = encode_tiles(&ncgr);
        assert_eq!(kind_of(&chunk), Some(ChunkKind::Tiles));

        let tiles = Tiles::parse(&chunk).expect("chunk parses");
        assert!(tiles.is_4bpp());
        assert_eq!(tiles.mapping(), CharMapping::TwoD);
        assert_eq!(tiles.grid(), Some((2, 1)));
        assert_eq!(tiles.cpos(), None);
        assert!(!tiles.has_vram_transfer());
        assert_eq!(tiles.tile_count(), 2);
        // 0xAB nibbles expand to alternating 11, 10.
        assert_eq!(tiles.pixels(), &[0x0B, 0x0A].repeat(64));

        // The CPOS variant round-trips its sheet size too.
        let data = build_ncgr(true);
        let ncgr = Ncgr::parse(&data).expect("CPOS fixture parses");
        let tiles = Tiles::parse(&encode_tiles(&ncgr)).expect("chunk parses");
        assert_eq!(tiles.cpos(), Some((2, 1)));
    }

    #[test]
    fn palette_roundtrip() {
        let data = build_compressed_nclr();
        let nclr = Nclr::parse(&data).expect("fixture parses");
        let chunk = encode_palette(&nclr);
        assert_eq!(kind_of(&chunk), Some(ChunkKind::Palette));

        let pal = Palette::parse(&chunk).expect("chunk parses");
        assert!(pal.is_16_color());
        assert!(!pal.is_extended());
        // White, full red, 1/32 green, white: 31 -> 0xFF, 1 -> 0x08.
        assert_eq!(
            pal.rgba(),
            &[
                [0xFF, 0xFF, 0xFF, 0xFF],
                [0xFF, 0x00, 0x00, 0xFF],
                [0x00, 0x08, 0x00, 0xFF],
                [0xFF, 0xFF, 0xFF, 0xFF],
            ]
        );
        // The PMCP patch table passes through verbatim.
        assert_eq!(pal.pmcp(), &[0, 0, 1]);

        // The no-PMCP common case must round-trip too (the count field
        // is always present; only the indices are conditional).
        let data = build_plain_nclr();
        let nclr = Nclr::parse(&data).expect("plain fixture parses");
        let pal = Palette::parse(&encode_palette(&nclr)).expect("chunk parses");
        assert_eq!(pal.rgba().len(), 4);
        assert!(pal.pmcp().is_empty());
    }

    #[test]
    fn screen_roundtrip() {
        let data = build_nscr();
        let nscr = Nscr::parse(&data).expect("fixture parses");
        let chunk = encode_screen(&nscr);
        assert_eq!(kind_of(&chunk), Some(ChunkKind::Screen));

        let screen = Screen::parse(&chunk).expect("chunk parses");
        assert_eq!((screen.width(), screen.height()), (32, 16));
        assert_eq!((screen.color_mode(), screen.screen_format()), (0, 0));
        assert_eq!(screen.entries().len(), 16); // 4×2 tiles × 2 bytes
        assert_eq!(&screen.entries()[..2], &[0x4A, 0x00]);
    }

    #[test]
    fn text_roundtrip() {
        let data = build_mat(0xFEE8, &[&[0x1be, 0x1be], &[0x12b]]);
        let bank = MsgBank::parse(&data).expect("fixture parses");
        let chunk = encode_text(&bank);
        assert_eq!(kind_of(&chunk), Some(ChunkKind::Text));

        let text = Text::parse(&chunk).expect("chunk parses");
        assert_eq!(text.key(), 0xFEE8);
        assert_eq!(text.message_count(), 2);
        assert_eq!(text.message(0), Some(&[0x1be, 0x1be, 0xFFFF][..]));
        assert_eq!(text.message(1), Some(&[0x12b, 0xFFFF][..]));
        assert_eq!(text.message(2), None);
        assert_eq!(text.messages().count(), 2);

        // An empty bank round-trips too (message_count 0).
        let data = build_mat(0x1234, &[]);
        let empty = MsgBank::parse(&data).expect("empty parses");
        let text = Text::parse(&encode_text(&empty)).expect("chunk parses");
        assert_eq!(text.message_count(), 0);
    }

    #[test]
    fn corrupt_chunks_are_rejected() {
        let data = build_ncgr(false);
        let chunk = encode_tiles(&Ncgr::parse(&data).unwrap());
        // Truncation and trailing garbage.
        assert!(Tiles::parse(&chunk[..chunk.len() - 1]).is_err());
        let mut trailing = chunk.clone();
        trailing.push(0);
        assert!(Tiles::parse(&trailing).is_err());
        // Wrong kind: the Text reader must refuse a Tiles chunk.
        assert!(Text::parse(&chunk).is_err());
        // A tile count that disagrees with the pixel count (tile_count
        // sits after the header + grid dims: 8 + 4 + 4 bytes).
        let mut bad = chunk.clone();
        bad[16..20].copy_from_slice(&99u32.to_le_bytes());
        assert!(Tiles::parse(&bad).is_err());
        // Not a chunk at all, and a bad version.
        assert_eq!(kind_of(b"not a chunk"), None);
        let mut bad_version = chunk.clone();
        bad_version[4] = 2;
        assert_eq!(kind_of(&bad_version), None);
    }
}
