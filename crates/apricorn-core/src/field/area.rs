//! Area data — pret `AreaDataManager_Alloc` / `AreaDataManager_Load`
//! (`asm/overlay_01_021FB878.s:29`, `:276`): NARC `a/0/4/2`
//! (`fielddata/areadata/area_data`), 106 members of 8 bytes:
//! `u16 buildingSet; u16 mapTexture; u16 unknown; u8 outdoor; u8
//! lightSelector`.
//!
//! `buildingSet` names the `a/0/4/3` id list (`u16 count; u16 ids[]`)
//! and the `a/0/7/0` building NSBTX; `mapTexture` the `a/0/4/4` NSBTX.
//! Byte 6 selects the prop model archive and its animation lists:
//! non-zero → outdoor, `a/0/4/0` (bm_field) + `a/1/0/7`; zero → indoor,
//! `a/1/4/8` (bm_room) + `a/1/0/8` (the `NARC_New` pairs 0x28/0x6b and
//! 0x94/0x6c in the allocator).

use crate::assets::{AssetStore, AssetsError};
use crate::nds::{NdsError, u16le};

/// NitroFS path of the area-data archive.
pub const AREA_NARC: &str = "a/0/4/2";
/// Members in the retail archive.
pub const AREA_COUNT: usize = 106;
/// Bytes per record.
pub const RECORD_SIZE: usize = 8;
/// Building-set id lists (`u16 count; u16 ids[count]`), by `building_set`.
pub const BUILDING_SET_NARC: &str = "a/0/4/3";
/// Map (land model) NSBTX archives, by `map_texture`.
pub const MAP_TEXTURE_NARC: &str = "a/0/4/4";
/// Building (prop) NSBTX archives, by `building_set`.
pub const BUILDING_TEXTURE_NARC: &str = "a/0/7/0";
/// Outdoor prop models (`bm_field`).
pub const BM_FIELD_NARC: &str = "a/0/4/0";
/// Indoor prop models (`bm_room`).
pub const BM_ROOM_NARC: &str = "a/1/4/8";
/// Outdoor prop animation lists.
pub const BM_FIELD_ANIM_NARC: &str = "a/1/0/7";
/// Indoor prop animation lists.
pub const BM_ROOM_ANIM_NARC: &str = "a/1/0/8";

/// One area-data record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AreaData {
    /// Member index (`MapHeader::area_data_bank`).
    pub bank: u8,
    /// `buildingSet`.
    pub building_set: u16,
    /// `mapTexture`.
    pub map_texture: u16,
    /// The third word (0xFFFF on the bedroom's area 25).
    pub unknown: u16,
    /// Byte 6 as stored: non-zero selects the outdoor archives.
    pub outdoor: u8,
    /// `lightSelector` (`AreaDataManager_GetAreaLightArchiveID`).
    pub light_selector: u8,
}

impl AreaData {
    /// Decodes record `bank`.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the record is shorter than 8 bytes.
    pub fn parse(bank: u8, record: &[u8]) -> Result<Self, NdsError> {
        if record.len() < RECORD_SIZE {
            return Err(NdsError::Truncated {
                what: "area data",
                need: RECORD_SIZE,
                got: record.len(),
            });
        }
        Ok(Self {
            bank,
            building_set: u16le(record, 0)?,
            map_texture: u16le(record, 2)?,
            unknown: u16le(record, 4)?,
            outdoor: record[6],
            light_selector: record[7],
        })
    }

    /// Loads record `bank` (`AreaDataManager_Alloc`).
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the member is missing or short.
    pub fn load(store: &AssetStore, bank: u8) -> Result<Self, AssetsError> {
        let bytes = store.member(AREA_NARC, usize::from(bank))?;
        Self::parse(bank, &bytes).map_err(|source| AssetsError::Corrupt {
            what: format!("{AREA_NARC}#{bank}"),
            source,
        })
    }

    /// Whether the area uses the outdoor archives.
    #[must_use]
    pub fn is_outdoor(&self) -> bool {
        self.outdoor != 0
    }

    /// The prop model archive this area's placements index.
    #[must_use]
    pub fn prop_model_narc(&self) -> &'static str {
        if self.is_outdoor() {
            BM_FIELD_NARC
        } else {
            BM_ROOM_NARC
        }
    }

    /// The prop animation-list archive paired with
    /// [`Self::prop_model_narc`].
    #[must_use]
    pub fn prop_animation_narc(&self) -> &'static str {
        if self.is_outdoor() {
            BM_FIELD_ANIM_NARC
        } else {
            BM_ROOM_ANIM_NARC
        }
    }

    /// The building ids of this area's set (`a/0/4/3`).
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the list member is missing or its
    /// count overruns it.
    pub fn building_ids(&self, store: &AssetStore) -> Result<Vec<u16>, AssetsError> {
        let bytes = store.member(BUILDING_SET_NARC, usize::from(self.building_set))?;
        parse_building_ids(&bytes).map_err(|source| AssetsError::Corrupt {
            what: format!("{BUILDING_SET_NARC}#{}", self.building_set),
            source,
        })
    }
}

/// Decodes a building-set id list.
///
/// # Errors
/// Returns an [`NdsError`] when the count overruns the list.
pub fn parse_building_ids(bytes: &[u8]) -> Result<Vec<u16>, NdsError> {
    let count = usize::from(u16le(bytes, 0)?);
    (0..count).map(|i| u16le(bytes, 2 + i * 2)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_fields_and_archive_selection() {
        let indoor = AreaData::parse(25, &[1, 0, 25, 0, 0xFF, 0xFF, 0, 0]).unwrap();
        assert_eq!((indoor.building_set, indoor.map_texture, indoor.unknown), (1, 25, 0xFFFF));
        assert!(!indoor.is_outdoor());
        assert_eq!(indoor.prop_model_narc(), BM_ROOM_NARC);
        assert_eq!(indoor.prop_animation_narc(), BM_ROOM_ANIM_NARC);
        let outdoor = AreaData::parse(2, &[0, 0, 2, 0, 0, 0, 1, 1]).unwrap();
        assert!(outdoor.is_outdoor());
        assert_eq!(outdoor.prop_model_narc(), BM_FIELD_NARC);
        assert_eq!(outdoor.prop_animation_narc(), BM_FIELD_ANIM_NARC);
        assert_eq!(outdoor.light_selector, 1);
        assert!(AreaData::parse(0, &[0; 7]).is_err());
    }

    #[test]
    fn building_lists_are_count_prefixed() {
        assert_eq!(parse_building_ids(&[2, 0, 5, 0, 9, 1]).unwrap(), vec![5, 265]);
        assert!(parse_building_ids(&[3, 0, 5, 0]).is_err());
    }
}
