//! NCER — Nitro Cell Resource: a bank of sprite cells. Magic `RECN`.
//!
//! A NCER pairs with a NCGR sheet and a NANR animation bank (see
//! [`crate::formats::nanr`]): each *cell* is one named pose — a set of
//! raw OAM register triples (the GBA/DS sprite hardware's attr0/1/2)
//! that place tiles from the sheet, plus flip and bounding metadata.
//!
//! Beyond the shared Nitro container header (see [`crate::formats`]) every
//! retail HeartGold file carries exactly three sections, in order:
//!
//! - **KBEC** (`CEBK` reversed — Nitro writes section magics backwards)
//!   — the cell bank:
//!
//!   ```text
//!   +0x00 2  nCells
//!   +0x02 2  cellBankAttr   bit 0 = extended records (with bounding
//!                           boxes); all other bits zero on retail
//!   +0x04 4  cellRecordsOffset (always 0x18)
//!   +0x08 4  mapping        NNSG2dCharacterDataMapingType (see CellMapping)
//!   +0x0C 4  vramTransferOffset (0 = none; body-relative)
//!   +0x10 4  reserved (always 0)
//!   +0x14 4  ucatOffset     (0 = none; body-relative)
//!   ```
//!
//!   followed by the cell records, the OAM data, and the optional VRAM
//!   transfer and UCAT blocks, each starting exactly where the previous
//!   ends (see [`Ncer::parse`] for the checks):
//!
//!   - cell records, `nCells` × (8 or 0x10 bytes):
//!     `u16 nOAM, u16 cellAttr, u16 oamDataOffset, u16 reserved`; extended
//!     records add `s16 maxX, s16 maxY, s16 minX, s16 minY`. `cellAttr`
//!     packs bits 0–5 bounding-sphere radius, 8 h-flip, 9 v-flip,
//!     10 hv-flip, 11 bounding-rect flag. `oamDataOffset` chains: cell
//!     *i*'s OAM data sits at `6 × ΣnOAM` of the cells before it.
//!   - OAM data, 6 bytes per entry (raw attr0/attr1/attr2), padded to
//!     4-byte alignment.
//!   - **VRAM transfer** (157 retail files): `u32 szByteMax, u32 reserved
//!     (always 8)`, then one `(u32 srcOffset, u32 size)` pair per cell —
//!     the DMA sources for streaming cell graphics into OBJ VRAM.
//!   - **UCAT** (`TACU`, 146 retail files): per-cell user attributes —
//!     a fixed 0x10-byte header, a fully-derived pointer array, and one
//!     `u32` attribute per cell. See [`Ucat`].
//! - **LBAL** — the label bank (see [`crate::formats`] for its odd
//!   count-less offset table). A NCER's labels name cells, but the
//!   count is independent of `nCells` (e.g. 224 cells, 16 labels).
//! - **UEXT** (`TXEU`) — user-extension marker; always 0x0C bytes of zero.
//!
//! Retail HeartGold (US): 608 members, 11,703 OAM entries in total; see
//! `docs/nitro-sprite.md` for the worked ground truth.

use crate::formats::{nitro_labels, nitro_sections};
use crate::nds::{NdsError, u16le, u32le};

/// How the bank's paired NCGR sheet maps into OBJ VRAM
/// (`NNSG2dCharacterDataMapingType`).
///
/// This is the SDK's *character-data* mapping enum — small values 0–4 —
/// and not the packed `GXOBJVRamModeChar` register bits a NCGR's CHAR
/// section stores (see [`crate::formats::CharMapping`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CellMapping {
    /// 1D mapping, 32 KB boundary (raw 0).
    OneD32K,
    /// 1D mapping, 64 KB boundary (raw 1).
    OneD64K,
    /// 1D mapping, 128 KB boundary (raw 2).
    OneD128K,
    /// 1D mapping, 256 KB boundary (raw 3).
    OneD256K,
    /// 2D mapping (raw 4; never used by HeartGold's cell banks).
    TwoD,
}

impl CellMapping {
    /// Decodes the raw `NNSG2dCharacterDataMapingType` value.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any value outside the enum.
    pub(crate) fn from_raw(raw: u32) -> Result<Self, NdsError> {
        match raw {
            0 => Ok(Self::OneD32K),
            1 => Ok(Self::OneD64K),
            2 => Ok(Self::OneD128K),
            3 => Ok(Self::OneD256K),
            4 => Ok(Self::TwoD),
            _ => Err(NdsError::Invalid {
                what: "unknown NNSG2dCharacterDataMapingType",
            }),
        }
    }

    /// The raw `NNSG2dCharacterDataMapingType` value (the inverse of
    /// [`CellMapping::from_raw`]; used by the round-trip serializer).
    #[must_use]
    pub(crate) fn raw(self) -> u32 {
        match self {
            Self::OneD32K => 0,
            Self::OneD64K => 1,
            Self::OneD128K => 2,
            Self::OneD256K => 3,
            Self::TwoD => 4,
        }
    }
}

/// A cell's bounding box, from an extended cell record.
///
/// The Nitro writers store the corners in `maxX, maxY, minX, minY` order
/// (and `min ≤ max` holds on every retail file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundingBox {
    /// Right edge.
    pub max_x: i16,
    /// Bottom edge.
    pub max_y: i16,
    /// Left edge.
    pub min_x: i16,
    /// Top edge.
    pub min_y: i16,
}

/// One cell: a named pose built from OAM entries.
///
/// Borrows the file bytes; see [`Ncer::parse`].
#[derive(Debug)]
pub struct Cell<'a> {
    /// Number of OAM entries that make up the cell.
    pub oam_count: usize,
    /// Raw `cellAttr`: bits 0–5 bounding-sphere radius, bit 8 h-flip,
    /// bit 9 v-flip, bit 10 hv-flip, bit 11 bounding-rect flag.
    attr: u16,
    /// The cell's OAM entries, 6 bytes each (`attr0, attr1, attr2`).
    oam: &'a [u8],
    bounds: Option<BoundingBox>,
}

impl Cell<'_> {
    /// The bounding-sphere radius (`cellAttr` bits 0–5), in pixels.
    #[must_use]
    pub fn radius(&self) -> u8 {
        (self.attr & 0x3F) as u8
    }

    /// Whether the cell's OAM entries are h-flipped (`cellAttr` bit 8).
    #[must_use]
    pub fn h_flip(&self) -> bool {
        self.attr & 0x100 != 0
    }

    /// Whether the cell's OAM entries are v-flipped (`cellAttr` bit 9).
    #[must_use]
    pub fn v_flip(&self) -> bool {
        self.attr & 0x200 != 0
    }

    /// Whether the cell is flipped across both axes (`cellAttr` bit 10).
    #[must_use]
    pub fn hv_flip(&self) -> bool {
        self.attr & 0x400 != 0
    }

    /// Whether the cell uses rectangular rather than spherical bounds
    /// (`cellAttr` bit 11).
    #[must_use]
    pub fn has_bounding_rect(&self) -> bool {
        self.attr & 0x800 != 0
    }

    /// The OAM entry `i` as its raw hardware registers
    /// `(attr0, attr1, attr2)`.
    #[must_use]
    pub fn oam_attr(&self, i: usize) -> Option<(u16, u16, u16)> {
        let bytes = self.oam.get(6 * i..6 * i + 6)?;
        let half = |n: usize| u16::from_le_bytes([bytes[2 * n], bytes[2 * n + 1]]);
        Some((half(0), half(1), half(2)))
    }

    /// The cell's bounding box, if the bank uses extended cell records.
    #[must_use]
    pub fn bounding_box(&self) -> Option<BoundingBox> {
        self.bounds
    }
}

/// The VRAM transfer block: DMA sources for streaming cell graphics.
#[derive(Debug)]
pub struct VramTransfer {
    /// The maximum size any one cell transfers.
    pub sz_byte_max: u32,
    /// One `(srcOffset, size)` pair per cell.
    pub blocks: Vec<(u32, u32)>,
}

/// The UCAT user-attribute block (magic `TACU`): one `u32` per cell.
///
/// The block's on-disk header declares its layout outright — `u16
/// numCells, u16 attrsPerCell(1), u32 reserved(8)` followed by a `u32`
/// offset per cell and the attributes — and every pointer value on every
/// retail file is fully derived (`8 + 4*nCells + 4*i`, pointing at
/// attribute *i*), so [`Ncer::parse`] validates rather than trusts them.
#[derive(Debug)]
pub struct Ucat {
    attrs: Vec<u32>,
}

impl Ucat {
    /// The user attribute of cell `i`.
    #[must_use]
    pub fn attr(&self, i: usize) -> Option<u32> {
        self.attrs.get(i).copied()
    }

    /// All per-cell user attributes, in cell order.
    #[must_use]
    pub fn attrs(&self) -> &[u32] {
        &self.attrs
    }
}

/// A parsed NCER. Borrows the file bytes; see [`Ncer::parse`].
#[derive(Debug)]
pub struct Ncer<'a> {
    /// Container version (always 0x0100 on retail files).
    version: u16,
    mapping: CellMapping,
    cells: Vec<Cell<'a>>,
    /// Trailing alignment bytes after the OAM data when no VRAM or UCAT
    /// block follows (0–3): a section ending after the OAM data may end
    /// there exactly or 4-aligned, and both shapes ship in retail. A
    /// retained writer artifact, reproduced by [`Ncer::to_bytes`].
    oam_pad: usize,
    vram: Option<VramTransfer>,
    ucat: Option<Ucat>,
    labels: Vec<&'a str>,
}

/// Whether `data` begins with a NCER header.
#[must_use]
pub fn is_ncer(data: &[u8]) -> bool {
    data.get(0..6) == Some(&[b'R', b'E', b'C', b'N', 0xFF, 0xFE])
}

impl<'a> Ncer<'a> {
    /// Parses a complete NCER file.
    ///
    /// Beyond the usual container checks this enforces the full retail
    /// layout: the KBEC header constants, the cell-record fields, the
    /// OAM offset chain, that the optional VRAM and UCAT blocks sit
    /// exactly at (and tile exactly to the end of) the section, and the
    /// UCAT pointer derivation.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any truncated, inconsistent, or
    /// non-retail-shaped file.
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        let (version, sections) = nitro_sections(data, b"RECN", 3)?;
        if version != 0x0100 {
            return Err(NdsError::Invalid {
                what: "NCER version (always 0x0100 on retail files)",
            });
        }
        let [kbec, labl, uext] = &sections[..] else {
            return Err(NdsError::Invalid {
                what: "NCER section count",
            });
        };
        for (sec, magic, what) in [
            (kbec, b"KBEC", "NCER's first section is not the cell bank"),
            (labl, b"LBAL", "NCER's second section is not the label bank"),
            (
                uext,
                b"TXEU",
                "NCER's third section is not the user-extension block",
            ),
        ] {
            if sec.magic != *magic {
                return Err(NdsError::Invalid { what });
            }
        }
        if uext.size != 0xC || u32le(data, uext.offset + 8)? != 0 {
            return Err(NdsError::Invalid {
                what: "NCER UEXT block (always 0x0C bytes of zero)",
            });
        }

        let body = kbec.offset + 8;
        let body_len = kbec.size - 8;
        let body_end = body + body_len;
        if body_end > data.len() {
            return Err(NdsError::Truncated {
                what: "NCER KBEC section",
                need: body_end,
                got: data.len(),
            });
        }

        // --- KBEC header ---
        let n_cells = u16le(data, body)? as usize;
        if n_cells == 0 {
            return Err(NdsError::Invalid {
                what: "NCER cell count",
            });
        }
        let bank_attr = u16le(data, body + 2)?;
        if bank_attr & !1 != 0 {
            return Err(NdsError::Invalid {
                what: "NCER cellBankAttr (only bit 0 exists)",
            });
        }
        let extended = bank_attr & 1 != 0;
        let stride = if extended { 0x10 } else { 0x8 };
        if u32le(data, body + 4)? as usize != 0x18 {
            return Err(NdsError::Invalid {
                what: "NCER cellRecordsOffset (always 0x18)",
            });
        }
        let mapping = CellMapping::from_raw(u32le(data, body + 8)?)?;
        let vram_off = u32le(data, body + 0xC)? as usize;
        if u32le(data, body + 0x10)? != 0 {
            return Err(NdsError::Invalid {
                what: "NCER KBEC reserved field",
            });
        }
        let ucat_off = u32le(data, body + 0x14)? as usize;

        // --- Cell records ---
        let cells_off = body + 0x18;
        let cells_end = cells_off + n_cells * stride;
        if cells_end > body_end {
            return Err(NdsError::Truncated {
                what: "NCER cell records",
                need: cells_end,
                got: data.len(),
            });
        }
        let mut cells: Vec<Cell> = Vec::with_capacity(n_cells);
        let mut total_oam = 0usize;
        for i in 0..n_cells {
            let at = cells_off + i * stride;
            let oam_count = u16le(data, at)? as usize;
            let attr = u16le(data, at + 2)?;
            if attr & !0x3FFF != 0 {
                return Err(NdsError::Invalid {
                    what: "NCER cellAttr (only bits 0-5 and 8-11 exist)",
                });
            }
            let oam_off = u16le(data, at + 4)? as usize;
            if u16le(data, at + 6)? != 0 {
                return Err(NdsError::Invalid {
                    what: "NCER cell record padding",
                });
            }
            if oam_off != total_oam * 6 {
                return Err(NdsError::Invalid {
                    what: "NCER oamDataOffset chain",
                });
            }
            let bounds = extended
                .then(|| -> Result<BoundingBox, NdsError> {
                    let bounds = BoundingBox {
                        max_x: u16le(data, at + 8)? as i16,
                        max_y: u16le(data, at + 10)? as i16,
                        min_x: u16le(data, at + 12)? as i16,
                        min_y: u16le(data, at + 14)? as i16,
                    };
                    // A cell with no OAMs keeps the tool's sentinel box
                    // (max = INT16_MIN, min = INT16_MAX — never updated
                    // per OAM); a non-empty cell must carry a real box.
                    let sentinel = bounds.max_x == i16::MIN
                        && bounds.max_y == i16::MIN
                        && bounds.min_x == i16::MAX
                        && bounds.min_y == i16::MAX;
                    if !sentinel
                        && oam_count > 0
                        && (bounds.min_x > bounds.max_x || bounds.min_y > bounds.max_y)
                    {
                        return Err(NdsError::Invalid {
                            what: "NCER cell bounding box (min > max)",
                        });
                    }
                    Ok(bounds)
                })
                .transpose()?;
            cells.push(Cell {
                oam_count,
                attr,
                oam: &[],
                bounds,
            });
            total_oam += oam_count;
        }

        // --- OAM data (padded to 4-byte alignment relative to the body) ---
        let oam_start = cells_end;
        let oam_end = oam_start + 6 * total_oam;
        if oam_end > body_end {
            return Err(NdsError::Truncated {
                what: "NCER OAM data",
                need: oam_end,
                got: data.len(),
            });
        }
        let mut at = oam_start;
        for cell in &mut cells {
            cell.oam = &data[at..at + 6 * cell.oam_count];
            at += 6 * cell.oam_count;
        }
        // Body-relative alignment, so relative to `body`, not absolute.
        // The padding exists to place a following VRAM transfer block on
        // a 4-byte boundary; a section that simply ends after the OAM
        // data ends there — exactly, or padded, and retail ships both
        // shapes.
        let padded_oam_end = body + (oam_end - body).div_ceil(4) * 4;
        let mut oam_pad = 0;
        if vram_off == 0 && ucat_off == 0 {
            if body_end == padded_oam_end {
                oam_pad = padded_oam_end - oam_end;
            } else if body_end != oam_end {
                return Err(NdsError::Invalid {
                    what: "NCER KBEC section does not end at the OAM data",
                });
            }
        }

        // --- Optional VRAM transfer block, exactly at the padded end ---
        let mut vram = None;
        // A section with no VRAM/UCAT block ends at the OAM data or its
        // padding — both validated above, so `body_end` is where it ends.
        let mut end = if vram_off != 0 || ucat_off != 0 {
            padded_oam_end
        } else {
            body_end
        };
        if vram_off != 0 {
            if vram_off != padded_oam_end - body {
                return Err(NdsError::Invalid {
                    what: "NCER vramTransferOffset (must follow the OAM data exactly)",
                });
            }
            let at = body + vram_off;
            let sz_byte_max = u32le(data, at)?;
            if u32le(data, at + 4)? != 8 {
                return Err(NdsError::Invalid {
                    what: "NCER VRAM transfer reserved field",
                });
            }
            let mut blocks = Vec::with_capacity(n_cells);
            for i in 0..n_cells {
                blocks.push((u32le(data, at + 8 + 8 * i)?, u32le(data, at + 12 + 8 * i)?));
            }
            vram = Some(VramTransfer {
                sz_byte_max,
                blocks,
            });
            end = at + 8 + 8 * n_cells;
        }

        // --- Optional UCAT block, exactly after the VRAM block ---
        let mut ucat = None;
        if ucat_off != 0 {
            if ucat_off != end - body {
                return Err(NdsError::Invalid {
                    what: "NCER ucatOffset (must follow the preceding block exactly)",
                });
            }
            let at = body + ucat_off;
            if &data[at..at + 4] != b"TACU" {
                return Err(NdsError::Invalid {
                    what: "NCER UCAT magic",
                });
            }
            if u32le(data, at + 4)? as usize != 0x10 + 8 * n_cells {
                return Err(NdsError::Invalid {
                    what: "NCER UCAT size",
                });
            }
            if u16le(data, at + 8)? as usize != n_cells {
                return Err(NdsError::Invalid {
                    what: "NCER UCAT numCells (must match the bank)",
                });
            }
            if u16le(data, at + 0xA)? != 1 {
                return Err(NdsError::Invalid {
                    what: "NCER UCAT attrsPerCell (always 1)",
                });
            }
            if u32le(data, at + 0xC)? != 8 {
                return Err(NdsError::Invalid {
                    what: "NCER UCAT reserved field",
                });
            }
            for i in 0..n_cells {
                if u32le(data, at + 0x10 + 4 * i)? as usize != 8 + 4 * n_cells + 4 * i {
                    return Err(NdsError::Invalid {
                        what: "NCER UCAT pointer derivation",
                    });
                }
            }
            let attrs_off = at + 0x10 + 4 * n_cells;
            let mut attrs = Vec::with_capacity(n_cells);
            for i in 0..n_cells {
                attrs.push(u32le(data, attrs_off + 4 * i)?);
            }
            ucat = Some(Ucat { attrs });
            end = at + 0x10 + 8 * n_cells;
        }

        if end != body_end {
            return Err(NdsError::Invalid {
                what: "NCER KBEC blocks do not tile the section",
            });
        }

        let labels = nitro_labels(data, labl.offset, labl.size)?;

        Ok(Self {
            version,
            mapping,
            cells,
            oam_pad,
            vram,
            ucat,
            labels,
        })
    }

    /// The container version (always 0x0100 on retail files).
    #[must_use]
    pub fn version(&self) -> u16 {
        self.version
    }

    /// The OBJ VRAM mapping mode the bank's cells assume.
    #[must_use]
    pub fn mapping(&self) -> CellMapping {
        self.mapping
    }

    /// The number of cells in the bank.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// All cells, in bank order.
    #[must_use]
    pub fn cells(&self) -> &[Cell<'a>] {
        &self.cells
    }

    /// Cell `i`, if it exists.
    #[must_use]
    pub fn cell(&self, i: usize) -> Option<&Cell<'a>> {
        self.cells.get(i)
    }

    /// Whether the bank uses extended cell records (with bounding boxes).
    #[must_use]
    pub fn is_extended(&self) -> bool {
        self.cells[0].bounds.is_some()
    }

    /// The VRAM transfer block, if the bank has one.
    #[must_use]
    pub fn vram_transfer(&self) -> Option<&VramTransfer> {
        self.vram.as_ref()
    }

    /// The UCAT user-attribute block, if the bank has one.
    #[must_use]
    pub fn ucat(&self) -> Option<&Ucat> {
        self.ucat.as_ref()
    }

    /// The bank's labels, in section order.
    ///
    /// The count is independent of the cell count — labels name cells
    /// loosely, not one-to-one.
    #[must_use]
    pub fn labels(&self) -> &[&str] {
        &self.labels
    }

    /// Re-serializes the parsed NCER into its container form.
    ///
    /// Byte-exact on retail-shaped files, with every layout constant
    /// re-derived rather than copied: the KBEC header fields, the
    /// `oamDataOffset` chain, the VRAM/UCAT block offsets, the UCAT
    /// pointer derivation, and the LBAL offset table (a running sum of
    /// the label lengths). Two raw-byte invariants the serializer relies
    /// on were pinned empirically across all 612 retail banks: the OAM
    /// data's 4-byte alignment padding is zero, and every LBAL section
    /// ends exactly at its last label's NUL. (`tests/roundtrip_hg.rs`
    /// re-serializes every NCER in the ROM and byte-compares against the
    /// original — a parser that mislaid any field breaks the comparison.)
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let n_cells = self.cells.len();
        let extended = self.cells[0].bounds.is_some();
        let stride = if extended { 0x10 } else { 8 };

        // --- KBEC body: header, cell records, OAM data, VRAM, UCAT ---
        let mut body = vec![0u8; 0x18];
        body[0..2].copy_from_slice(&(n_cells as u16).to_le_bytes());
        body[2..4].copy_from_slice(&u16::from(extended).to_le_bytes());
        body[4..8].copy_from_slice(&0x18u32.to_le_bytes());
        body[8..12].copy_from_slice(&self.mapping.raw().to_le_bytes());
        let mut oam_off = 0usize;
        for cell in &self.cells {
            let at = body.len();
            body.resize(at + stride, 0);
            body[at..at + 2].copy_from_slice(&(cell.oam_count as u16).to_le_bytes());
            body[at + 2..at + 4].copy_from_slice(&cell.attr.to_le_bytes());
            body[at + 4..at + 6].copy_from_slice(&(oam_off as u16).to_le_bytes());
            if let Some(b) = cell.bounds {
                body[at + 8..at + 10].copy_from_slice(&b.max_x.to_le_bytes());
                body[at + 10..at + 12].copy_from_slice(&b.max_y.to_le_bytes());
                body[at + 12..at + 14].copy_from_slice(&b.min_x.to_le_bytes());
                body[at + 14..at + 16].copy_from_slice(&b.min_y.to_le_bytes());
            }
            oam_off += 6 * cell.oam_count;
        }
        for cell in &self.cells {
            body.extend_from_slice(cell.oam);
        }
        // Alignment padding: a following VRAM/UCAT block sits on a
        // 4-byte boundary (the padding bytes are zero on every padded
        // retail file); a section ending after the OAM data keeps the
        // writer's own padding, retained from the parse.
        if self.vram.is_some() || self.ucat.is_some() {
            body.resize(body.len().div_ceil(4) * 4, 0);
        } else {
            body.resize(body.len() + self.oam_pad, 0);
        }

        let vram_off = self.vram.as_ref().map(|_| body.len());
        if let Some(v) = &self.vram {
            body.extend_from_slice(&v.sz_byte_max.to_le_bytes());
            body.extend_from_slice(&8u32.to_le_bytes());
            for &(src, size) in &v.blocks {
                body.extend_from_slice(&src.to_le_bytes());
                body.extend_from_slice(&size.to_le_bytes());
            }
        }
        let ucat_off = self.ucat.as_ref().map(|_| body.len());
        if let Some(u) = &self.ucat {
            body.extend_from_slice(b"TACU");
            body.extend_from_slice(&((0x10 + 8 * n_cells) as u32).to_le_bytes());
            body.extend_from_slice(&(n_cells as u16).to_le_bytes());
            body.extend_from_slice(&1u16.to_le_bytes());
            body.extend_from_slice(&8u32.to_le_bytes());
            for i in 0..n_cells {
                body.extend_from_slice(&((8 + 4 * n_cells + 4 * i) as u32).to_le_bytes());
            }
            for &attr in &u.attrs {
                body.extend_from_slice(&attr.to_le_bytes());
            }
        }
        body[0xC..0x10].copy_from_slice(&(vram_off.unwrap_or(0) as u32).to_le_bytes());
        body[0x14..0x18].copy_from_slice(&(ucat_off.unwrap_or(0) as u32).to_le_bytes());

        // --- LBAL body: the derived offset table, then the strings ---
        let table = 4 * self.labels.len();
        let mut labl_body = vec![0u8; table];
        for (i, label) in self.labels.iter().enumerate() {
            let strings_off = labl_body.len() - table;
            labl_body[4 * i..4 * i + 4].copy_from_slice(&(strings_off as u32).to_le_bytes());
            labl_body.extend_from_slice(label.as_bytes());
            labl_body.push(0);
        }

        // --- Container ---
        let kbec_size = 8 + body.len();
        let labl_size = 8 + labl_body.len();
        let total = 0x10 + kbec_size + labl_size + 0xC;
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"RECN");
        out.extend_from_slice(&0xFEFFu16.to_le_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&0x10u16.to_le_bytes());
        out.extend_from_slice(&3u16.to_le_bytes());
        out.extend_from_slice(b"KBEC");
        out.extend_from_slice(&(kbec_size as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(b"LBAL");
        out.extend_from_slice(&(labl_size as u32).to_le_bytes());
        out.extend_from_slice(&labl_body);
        out.extend_from_slice(b"TXEU");
        out.extend_from_slice(&0xCu32.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a NCER in memory: `n_cells` cells with `oams_per_cell` OAM
    /// entries each. The payload OAM bytes are `attr0 = i*0x11`,
    /// `attr1 = 0x22`, `attr2 = 0x33`. Optional VRAM transfer and UCAT
    /// blocks are appended when requested; labels name the cells.
    fn build_ncer(extended: bool, with_vram: bool, with_ucat: bool) -> Vec<u8> {
        let n_cells = 2usize;
        let oams_per_cell = 2usize;
        let stride = if extended { 0x10 } else { 0x8 };

        // Cell records.
        let mut kbec_body = vec![0u8; 0x18];
        kbec_body[0..2].copy_from_slice(&(n_cells as u16).to_le_bytes());
        kbec_body[2..4].copy_from_slice(&(extended as u16).to_le_bytes());
        kbec_body[4..8].copy_from_slice(&0x18u32.to_le_bytes());
        kbec_body[8..12].copy_from_slice(&0u32.to_le_bytes()); // 1D_32K mapping
        for i in 0..n_cells {
            let at = 0x18 + i * stride;
            kbec_body.resize(at + stride, 0);
            kbec_body[at..at + 2].copy_from_slice(&(oams_per_cell as u16).to_le_bytes());
            kbec_body[at + 2..at + 4].copy_from_slice(&0x0820u16.to_le_bytes()); // rect flag + radius 0x20
            kbec_body[at + 4..at + 6]
                .copy_from_slice(&((i * oams_per_cell * 6) as u16).to_le_bytes());
            if extended {
                kbec_body[at + 8..at + 10].copy_from_slice(&16i16.to_le_bytes()); // maxX
                kbec_body[at + 10..at + 12].copy_from_slice(&8i16.to_le_bytes()); // maxY
                kbec_body[at + 12..at + 14].copy_from_slice(&(-16i16).to_le_bytes()); // minX
                kbec_body[at + 14..at + 16].copy_from_slice(&(-8i16).to_le_bytes()); // minY
            }
        }

        // OAM data: 2 registers per entry.
        let total_oam = n_cells * oams_per_cell;
        let oam_start = kbec_body.len();
        kbec_body.resize(oam_start + 6 * total_oam, 0);
        for e in 0..total_oam {
            let at = oam_start + 6 * e;
            kbec_body[at..at + 2].copy_from_slice(&((e * 0x11) as u16).to_le_bytes());
            kbec_body[at + 2..at + 4].copy_from_slice(&0x22u16.to_le_bytes());
            kbec_body[at + 4..at + 6].copy_from_slice(&0x33u16.to_le_bytes());
        }
        // 4-byte alignment pad.
        kbec_body.resize((kbec_body.len() + 3) & !3, 0);

        // Optional VRAM transfer: max size, reserved 8, one pair per cell.
        let vram_off = if with_vram {
            let off = kbec_body.len();
            kbec_body.extend_from_slice(&0x100u32.to_le_bytes());
            kbec_body.extend_from_slice(&8u32.to_le_bytes());
            for i in 0..n_cells {
                kbec_body.extend_from_slice(&((i * 0x40) as u32).to_le_bytes());
                kbec_body.extend_from_slice(&0x40u32.to_le_bytes());
            }
            Some(off)
        } else {
            None
        };

        // Optional UCAT: header, derived pointers, then attributes.
        let ucat_off = if with_ucat {
            let off = kbec_body.len();
            let ucat_size = 0x10 + 8 * n_cells;
            kbec_body.extend_from_slice(b"TACU");
            kbec_body.extend_from_slice(&(ucat_size as u32).to_le_bytes());
            kbec_body.extend_from_slice(&(n_cells as u16).to_le_bytes());
            kbec_body.extend_from_slice(&1u16.to_le_bytes());
            kbec_body.extend_from_slice(&8u32.to_le_bytes());
            for i in 0..n_cells {
                kbec_body.extend_from_slice(&((8 + 4 * n_cells + 4 * i) as u32).to_le_bytes());
            }
            for i in 0..n_cells {
                kbec_body.extend_from_slice(&(0x1000 + i as u32).to_le_bytes());
            }
            Some(off)
        } else {
            None
        };

        let kbec_size = 8 + kbec_body.len();

        // LBAL: one label per cell.
        let names: Vec<&[u8]> = vec![b"left", b"right"];
        let mut labl_body = Vec::new();
        let table_size = 4 * names.len();
        let mut pos = table_size;
        let mut offsets = Vec::new();
        for name in &names {
            offsets.push((pos - table_size) as u32);
            pos += name.len() + 1;
        }
        for off in &offsets {
            labl_body.extend_from_slice(&off.to_le_bytes());
        }
        for name in &names {
            labl_body.extend_from_slice(name);
            labl_body.push(0);
        }
        let labl_size = 8 + labl_body.len();

        let total = 0x10 + kbec_size + labl_size + 0xC;
        let mut rom = vec![0u8; total];
        rom[0..4].copy_from_slice(b"RECN");
        rom[4..6].copy_from_slice(&0xFEFFu16.to_le_bytes());
        rom[6..8].copy_from_slice(&0x0100u16.to_le_bytes());
        rom[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        rom[0xC..0xE].copy_from_slice(&0x10u16.to_le_bytes());
        rom[0xE..0x10].copy_from_slice(&3u16.to_le_bytes());

        let mut off = 0x10;
        rom[off..off + 4].copy_from_slice(b"KBEC");
        rom[off + 4..off + 8].copy_from_slice(&(kbec_size as u32).to_le_bytes());
        rom[off + 8..off + 8 + kbec_body.len()].copy_from_slice(&kbec_body);
        let vram_field = vram_off.unwrap_or(0);
        let ucat_field = ucat_off.unwrap_or(0);
        rom[off + 8 + 0xC..off + 8 + 0x10].copy_from_slice(&(vram_field as u32).to_le_bytes());
        rom[off + 8 + 0x14..off + 8 + 0x18].copy_from_slice(&(ucat_field as u32).to_le_bytes());
        off += kbec_size;

        rom[off..off + 4].copy_from_slice(b"LBAL");
        rom[off + 4..off + 8].copy_from_slice(&(labl_size as u32).to_le_bytes());
        rom[off + 8..off + 8 + labl_body.len()].copy_from_slice(&labl_body);
        off += labl_size;

        rom[off..off + 4].copy_from_slice(b"TXEU");
        rom[off + 4..off + 8].copy_from_slice(&0xCu32.to_le_bytes());
        rom
    }

    #[test]
    fn parses_cells_labels_and_blocks() {
        for extended in [false, true] {
            for with_vram in [false, true] {
                for with_ucat in [false, true] {
                    let data = build_ncer(extended, with_vram, with_ucat);
                    assert!(is_ncer(&data));
                    let ncer = Ncer::parse(&data).expect("synthetic NCER must parse");
                    assert_eq!(ncer.version(), 0x0100);
                    assert_eq!(ncer.mapping(), CellMapping::OneD32K);
                    assert_eq!(ncer.cell_count(), 2);
                    assert_eq!(ncer.is_extended(), extended);
                    assert_eq!(ncer.labels(), ["left", "right"]);

                    let cell = ncer.cell(0).unwrap();
                    assert_eq!(cell.oam_count, 2);
                    assert_eq!(cell.radius(), 0x20);
                    assert!(cell.has_bounding_rect());
                    assert!(!cell.h_flip());
                    assert_eq!(cell.oam_attr(0), Some((0x00, 0x22, 0x33)));
                    assert_eq!(cell.oam_attr(1), Some((0x11, 0x22, 0x33)));
                    assert_eq!(ncer.cell(1).unwrap().oam_attr(0), Some((0x22, 0x22, 0x33)));
                    if extended {
                        let b = cell.bounding_box().unwrap();
                        assert_eq!((b.max_x, b.max_y, b.min_x, b.min_y), (16, 8, -16, -8));
                    } else {
                        assert_eq!(cell.bounding_box(), None);
                    }

                    match (with_vram, ncer.vram_transfer()) {
                        (true, Some(v)) => {
                            assert_eq!(v.sz_byte_max, 0x100);
                            assert_eq!(v.blocks, vec![(0, 0x40), (0x40, 0x40)]);
                        }
                        (false, None) => {}
                        _ => panic!("VRAM transfer presence mismatch"),
                    }
                    match (with_ucat, ncer.ucat()) {
                        (true, Some(u)) => {
                            assert_eq!(u.attrs(), &[0x1000, 0x1001]);
                            assert_eq!(u.attr(0), Some(0x1000));
                            assert_eq!(u.attr(2), None);
                        }
                        (false, None) => {}
                        _ => panic!("UCAT presence mismatch"),
                    }

                    assert_eq!(
                        ncer.to_bytes(),
                        data,
                        "byte-exact across every block combination"
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_broken_ncer() {
        let good = build_ncer(true, true, true);

        let mut bad_magic = good.clone();
        bad_magic[0..4].copy_from_slice(b"RECM");
        assert!(Ncer::parse(&bad_magic).is_err());

        assert!(Ncer::parse(&good[..good.len() - 2]).is_err());

        // KBEC body starts at 0x18. Break each header invariant in turn.
        let field = |rom: &mut [u8], at: usize, value: u32| {
            rom[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        let mut bad_attr = good.clone();
        bad_attr[0x18 + 2..0x18 + 4].copy_from_slice(&3u16.to_le_bytes()); // unknown bank bits
        assert!(Ncer::parse(&bad_attr).is_err());

        let mut bad_records_off = good.clone();
        field(&mut bad_records_off, 0x18 + 4, 0x20);
        assert!(Ncer::parse(&bad_records_off).is_err());

        let mut bad_mapping = good.clone();
        field(&mut bad_mapping, 0x18 + 8, 9);
        assert!(Ncer::parse(&bad_mapping).is_err());

        let mut bad_reserved = good.clone();
        field(&mut bad_reserved, 0x18 + 0x10, 1);
        assert!(Ncer::parse(&bad_reserved).is_err());

        // Break the OAM offset chain: second cell's pointer.
        let mut bad_chain = good.clone();
        bad_chain[0x18 + 0x10 + 4..0x18 + 0x10 + 6].copy_from_slice(&6u16.to_le_bytes());
        assert!(Ncer::parse(&bad_chain).is_err());

        // Break the VRAM offset (no longer directly after the OAM data).
        let mut bad_vram = good.clone();
        field(&mut bad_vram, 0x18 + 0xC, 0x40);
        assert!(Ncer::parse(&bad_vram).is_err());

        // Break the UCAT fixed field. With this fixture the blocks run
        // body+0x38 (cells end) +0x18 (OAM) = padded 0x50, VRAM 0x50..0x68,
        // UCAT at body+0x68 (absolute 0x80).
        let mut bad_ucat = good.clone();
        field(&mut bad_ucat, 0x18 + 0x68 + 0xC, 0);
        assert!(Ncer::parse(&bad_ucat).is_err());

        // Break the UCAT pointer derivation.
        let mut bad_ptr = good.clone();
        field(&mut bad_ptr, 0x18 + 0x68 + 0x10, 0);
        assert!(Ncer::parse(&bad_ptr).is_err());

        // UEXT body must be zero.
        let mut bad_uext = good.clone();
        bad_uext[good.len() - 1] = 1;
        assert!(Ncer::parse(&bad_uext).is_err());

        assert!(!is_ncer(b"not an ncer at all"));
        assert!(Ncer::parse(b"RECN").is_err());
    }
}
