//! Land data — one 32 × 32-tile cell of the overworld: terrain
//! attributes, prop placements, the cell's NSBMD and its BDHC height
//! data. NARC `a/0/6/5` (`fielddata/landdata/land_data`), 676 members.
//!
//! Header (0x14 bytes, `TERRAIN_ATTRIBUTES_OFFSET` in pret
//! `include/terrain_attributes.h:11`): `u32 attrSize (0x800), propSize
//! (0x30 × n), modelSize, bdhcSize; u16 magic 0x1234; u16 extraSize`.
//! The sections follow contiguously in this order — measured on the
//! retail members with an extra section (0, 4, 5: `BMD0` sits at
//! `0x14 + attr + extra + prop`):
//!
//! ```text
//! attrs  0x800   u16 per tile, index (x % 32) + (z % 32) * 32
//! extra  extraSize   u16 words that look like attributes (0x8006 runs);
//!                    undocumented in pret, kept raw
//! props  0x30 each   MapPropArcData (src/field/map_prop_manager.c:13)
//! model  modelSize   BMD0 (the cell's ground/walls)
//! bdhc   bdhcSize    "BDHC" height data (undocumented in pret)
//! ```
//!
//! Attribute words: bits 0–7 the metatile behavior
//! (`include/constants/metatile_behavior.h`), bit 15 impassable
//! (`0x8000`); the prop manager stamps `0x8023` over Safari objects.
//!
//! BDHC: `"BDHC"`, six `u16` counts, then six arrays whose element sizes
//! (8, 12, 4, 8, 8, 2 bytes: points, slopes, heights, plates, strips,
//! access list) were inferred from the section arithmetic holding on all
//! 676 retail members. Kept raw until the height solver ports it.

use crate::assets::{AssetStore, AssetsError};
use crate::nds::{NdsError, u16le, u32le};

/// NitroFS path of the land-data archive.
pub const LAND_NARC: &str = "a/0/6/5";
/// Members in the retail archive.
pub const LAND_COUNT: usize = 676;
/// Header bytes before the attribute section.
pub const HEADER_SIZE: usize = 0x14;
/// Bytes in the attribute section (`TERRAIN_ATTRIBUTES_SIZE`).
pub const ATTRIBUTE_BYTES: usize = 0x800;
/// Attributes per cell (32 × 32).
pub const ATTRIBUTE_COUNT: usize = ATTRIBUTE_BYTES / 2;
/// `sizeof(MapPropArcData)`.
pub const PROP_RECORD_SIZE: usize = 0x30;
/// The header's magic word.
pub const MAGIC: u16 = 0x1234;
/// `MAP_PROP_MAX`: props the prop manager holds per cell.
pub const MAX_PROPS: usize = 32;
/// Attribute bit 15: the tile cannot be entered.
pub const IMPASSABLE: u16 = 0x8000;
/// Element sizes of the six BDHC arrays, in count order.
pub const BDHC_ELEMENT_SIZES: [usize; 6] = [8, 12, 4, 8, 8, 2];

/// Metatile behavior (bits 0–7) of an attribute word.
#[must_use]
pub const fn behavior(attribute: u16) -> u8 {
    attribute as u8
}

/// Whether an attribute word marks the tile impassable.
#[must_use]
pub const fn impassable(attribute: u16) -> bool {
    attribute & IMPASSABLE != 0
}

/// The header's section sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionSizes {
    /// `attrSize`.
    pub attributes: u32,
    /// `propSize`.
    pub props: u32,
    /// `modelSize`.
    pub model: u32,
    /// `bdhcSize`.
    pub bdhc: u32,
    /// `extraSize`.
    pub extra: u16,
}

impl SectionSizes {
    /// Reads the header.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the member is shorter than the header
    /// or the magic is not [`MAGIC`].
    pub fn parse(bytes: &[u8]) -> Result<Self, NdsError> {
        if bytes.len() < HEADER_SIZE {
            return Err(NdsError::Truncated {
                what: "land data header",
                need: HEADER_SIZE,
                got: bytes.len(),
            });
        }
        if u16le(bytes, 16)? != MAGIC {
            return Err(NdsError::Invalid {
                what: "land data magic",
            });
        }
        Ok(Self {
            attributes: u32le(bytes, 0)?,
            props: u32le(bytes, 4)?,
            model: u32le(bytes, 8)?,
            bdhc: u32le(bytes, 12)?,
            extra: u16le(bytes, 18)?,
        })
    }
}

/// `MapPropArcData`: one prop placed in a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropPlacement {
    /// `buildModel`: member of the area's prop archive.
    pub model_id: i32,
    /// `translation`, fx32, relative to the cell origin.
    pub translation: [i32; 3],
    /// `rotation` as stored. The retail per-cell draw
    /// (`ov01_021F3A3C`) uses an identity rotation and never reads this;
    /// no retail placement is non-zero.
    pub rotation: [i32; 3],
    /// `scale`, fx32 per axis (unit on every retail placement).
    pub scale: [i32; 3],
}

impl PropPlacement {
    /// Decodes one 0x30-byte record.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the record is short.
    pub fn parse(record: &[u8]) -> Result<Self, NdsError> {
        let word = |i: usize| u32le(record, i * 4).map(|v| v as i32);
        Ok(Self {
            model_id: word(0)?,
            translation: [word(1)?, word(2)?, word(3)?],
            rotation: [word(4)?, word(5)?, word(6)?],
            scale: [word(7)?, word(8)?, word(9)?],
        })
    }
}

/// The cell's BDHC block, raw.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bdhc {
    /// The six header counts: points, slopes, heights, plates, strips,
    /// access-list entries.
    pub counts: [u16; 6],
    /// The six arrays, raw, in the same order.
    pub sections: [Vec<u8>; 6],
}

impl Bdhc {
    /// Parses a BDHC block (an empty slice is an empty block).
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the magic is wrong or the counts do
    /// not account for exactly the block's bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, NdsError> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        if bytes.get(..4) != Some(b"BDHC") {
            return Err(NdsError::Invalid { what: "BDHC magic" });
        }
        let mut counts = [0u16; 6];
        for (i, c) in counts.iter_mut().enumerate() {
            *c = u16le(bytes, 4 + i * 2)?;
        }
        let mut p = 16;
        let mut sections: [Vec<u8>; 6] = Default::default();
        for (i, section) in sections.iter_mut().enumerate() {
            let n = usize::from(counts[i]) * BDHC_ELEMENT_SIZES[i];
            *section = bytes
                .get(p..p + n)
                .ok_or(NdsError::Truncated {
                    what: "BDHC section",
                    need: p + n,
                    got: bytes.len(),
                })?
                .to_vec();
            p += n;
        }
        if p != bytes.len() {
            return Err(NdsError::Invalid {
                what: "BDHC section sizes",
            });
        }
        Ok(Self { counts, sections })
    }

    /// The points array (8-byte elements).
    #[must_use]
    pub fn points(&self) -> &[u8] {
        &self.sections[0]
    }
    /// The slopes array (12-byte elements).
    #[must_use]
    pub fn slopes(&self) -> &[u8] {
        &self.sections[1]
    }
    /// The heights array (4-byte elements).
    #[must_use]
    pub fn heights(&self) -> &[u8] {
        &self.sections[2]
    }
    /// The plates array (8-byte elements).
    #[must_use]
    pub fn plates(&self) -> &[u8] {
        &self.sections[3]
    }
    /// The strips array (8-byte elements).
    #[must_use]
    pub fn strips(&self) -> &[u8] {
        &self.sections[4]
    }
    /// The access list (`u16` elements).
    #[must_use]
    pub fn access_list(&self) -> &[u8] {
        &self.sections[5]
    }
}

/// One parsed land-data member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandData {
    /// The section sizes from the header.
    pub sizes: SectionSizes,
    /// Terrain attributes, index `(x % 32) + (z % 32) * 32`.
    pub attributes: [u16; ATTRIBUTE_COUNT],
    /// The extra section, raw.
    pub extra: Vec<u8>,
    /// Prop placements, in archive order.
    pub props: Vec<PropPlacement>,
    /// The cell's NSBMD, raw (`BMD0`).
    pub model: Vec<u8>,
    /// The BDHC height data.
    pub bdhc: Bdhc,
}

impl LandData {
    /// Parses a member.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the header is malformed, the sections
    /// do not add up to the member length, the attribute section is not
    /// 0x800 bytes, the prop section is not a whole number of records, or
    /// the BDHC block is inconsistent.
    pub fn parse(bytes: &[u8]) -> Result<Self, NdsError> {
        let sizes = SectionSizes::parse(bytes)?;
        if sizes.attributes as usize != ATTRIBUTE_BYTES {
            return Err(NdsError::Invalid {
                what: "land attribute section size",
            });
        }
        if !(sizes.props as usize).is_multiple_of(PROP_RECORD_SIZE) {
            return Err(NdsError::Invalid {
                what: "land prop section size",
            });
        }
        let total = HEADER_SIZE
            + sizes.attributes as usize
            + usize::from(sizes.extra)
            + sizes.props as usize
            + sizes.model as usize
            + sizes.bdhc as usize;
        if total != bytes.len() {
            return Err(NdsError::Invalid {
                what: "land data section sizes",
            });
        }
        let mut p = HEADER_SIZE;
        let mut take = |n: usize| {
            let s = &bytes[p..p + n];
            p += n;
            s
        };
        let attr_bytes = take(ATTRIBUTE_BYTES);
        let mut attributes = [0u16; ATTRIBUTE_COUNT];
        for (i, a) in attributes.iter_mut().enumerate() {
            *a = u16le(attr_bytes, i * 2)?;
        }
        let extra = take(usize::from(sizes.extra)).to_vec();
        let props = take(sizes.props as usize)
            .chunks_exact(PROP_RECORD_SIZE)
            .map(PropPlacement::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let model = take(sizes.model as usize).to_vec();
        let bdhc = Bdhc::parse(take(sizes.bdhc as usize))?;
        Ok(Self {
            sizes,
            attributes,
            extra,
            props,
            model,
            bdhc,
        })
    }

    /// Loads member `id`.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the member is missing or malformed.
    pub fn load(store: &AssetStore, id: u16) -> Result<Self, AssetsError> {
        let bytes = store.member(LAND_NARC, usize::from(id))?;
        Self::parse(&bytes).map_err(|source| AssetsError::Corrupt {
            what: format!("{LAND_NARC}#{id}"),
            source,
        })
    }

    /// The attribute of tile `(x, z)` within the cell (0..32 each).
    #[must_use]
    pub fn attribute(&self, x: usize, z: usize) -> Option<u16> {
        if x >= 32 || z >= 32 {
            return None;
        }
        Some(self.attributes[x + z * 32])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(extra: usize, props: usize, bdhc: &[u8]) -> Vec<u8> {
        let model = b"BMD0";
        let mut b = Vec::new();
        b.extend_from_slice(&(ATTRIBUTE_BYTES as u32).to_le_bytes());
        b.extend_from_slice(&((props * PROP_RECORD_SIZE) as u32).to_le_bytes());
        b.extend_from_slice(&(model.len() as u32).to_le_bytes());
        b.extend_from_slice(&(bdhc.len() as u32).to_le_bytes());
        b.extend_from_slice(&MAGIC.to_le_bytes());
        b.extend_from_slice(&(extra as u16).to_le_bytes());
        for i in 0..ATTRIBUTE_COUNT {
            b.extend_from_slice(&(i as u16).to_le_bytes());
        }
        b.extend(std::iter::repeat_n(0xEE, extra));
        for i in 0..props {
            let mut r = [0u8; PROP_RECORD_SIZE];
            r[..4].copy_from_slice(&(i as u32 + 1).to_le_bytes());
            r[4..8].copy_from_slice(&(-24 * 4096i32).to_le_bytes());
            for k in 7..10 {
                r[k * 4..k * 4 + 4].copy_from_slice(&4096u32.to_le_bytes());
            }
            b.extend_from_slice(&r);
        }
        b.extend_from_slice(model);
        b.extend_from_slice(bdhc);
        b
    }

    fn bdhc(counts: [u16; 6]) -> Vec<u8> {
        let mut b = b"BDHC".to_vec();
        for c in counts {
            b.extend_from_slice(&c.to_le_bytes());
        }
        for (i, c) in counts.iter().enumerate() {
            b.extend(std::iter::repeat_n(i as u8, usize::from(*c) * BDHC_ELEMENT_SIZES[i]));
        }
        b
    }

    #[test]
    fn sections_are_laid_out_attrs_extra_props_model_bdhc() {
        let block = bdhc([2, 1, 3, 1, 1, 4]);
        let land = LandData::parse(&member(8, 2, &block)).unwrap();
        assert_eq!(land.sizes.extra, 8);
        assert_eq!(land.extra, vec![0xEE; 8]);
        assert_eq!(land.props.len(), 2);
        assert_eq!(land.props[1].model_id, 2);
        assert_eq!(land.props[1].translation, [-24 * 4096, 0, 0]);
        assert_eq!(land.props[1].scale, [4096; 3]);
        assert_eq!(land.model, b"BMD0");
        assert_eq!(land.bdhc.counts, [2, 1, 3, 1, 1, 4]);
        assert_eq!(land.bdhc.points().len(), 16);
        assert_eq!(land.bdhc.slopes().len(), 12);
        assert_eq!(land.bdhc.heights().len(), 12);
        assert_eq!(land.bdhc.plates().len(), 8);
        assert_eq!(land.bdhc.strips().len(), 8);
        assert_eq!(land.bdhc.access_list().len(), 8);
        assert_eq!(land.attribute(1, 2), Some(65));
        assert_eq!(land.attribute(32, 0), None);
        assert_eq!(behavior(0x8086), 0x86);
        assert!(impassable(0x8000) && !impassable(0x005F));
    }

    #[test]
    fn inconsistent_members_are_rejected() {
        let block = bdhc([1, 0, 0, 0, 0, 0]);
        let mut bad = member(0, 1, &block);
        bad[16] = 0;
        assert!(LandData::parse(&bad).is_err(), "magic");
        let mut bad = member(0, 1, &block);
        bad.push(0);
        assert!(LandData::parse(&bad).is_err(), "trailing byte");
        let mut short = block.clone();
        short.pop();
        assert!(Bdhc::parse(&short).is_err());
        assert!(Bdhc::parse(b"XXXX").is_err());
        assert_eq!(Bdhc::parse(&[]).unwrap(), Bdhc::default());
    }
}
