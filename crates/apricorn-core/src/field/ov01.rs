//! Tables read from the field overlay (ARM9 overlay 1, `OVY_1`, loaded at
//! 0x021E5900 per the ROM overlay table; BLZ-compressed in the cart).
//!
//! * Camera presets `ov01_02206478` (`asm/overlay_01_021EABA8.s:465`):
//!   17 records of 0x24 bytes indexed by `MapHeader::camera_type` in
//!   `FieldCamera_Create` (`:17`, asserts `cameraType < 0x11`):
//!   `fx32 distance; u16 angle[3], pad; u16 perspectiveType; u16
//!   fovyAngle; fx32 near, far; VecFx32 lookAtOffset`.
//! * Sprite → move-model table `ov01_022074A8`
//!   (`asm/overlay_01_sprite_data.s:436`, walked by
//!   `ObjectEvent_GetGraphicsInfo` at `asm/overlay_01_021F8D80.s:764`):
//!   6-byte `{u16 spriteId, mmodelId, packed}` rows terminated by
//!   `spriteId == 0xFFFF`; `mmodelId` is the `a/0/8/1` member.
//! * The land model's base transform, `ov01_02206BD8` (`VecFx32` scale)
//!   and `ov01_02206BE4` (`MtxFx33` rotation) at
//!   `asm/overlay_01_021F4704.s:4507`, which
//!   `MapLoadManager_RenderLoadedMap` (`:2424`) copies onto the stack
//!   and hands `GF3dRender_DrawModel` with the cell origin from
//!   `ov01_021F5FB8` (`:3320`).
//!
//! Nothing here is copied: both tables are read from the overlay image
//! at run time.

use super::model::{FX32_ONE, Placement};
use crate::assets::{AssetStore, AssetsError};
use crate::nds::{NdsError, u16le, u32le};

/// The field overlay's id.
pub const OVERLAY_ID: u32 = 1;
/// Where the retail overlay table loads overlay 1.
pub const LOAD_ADDRESS: u32 = 0x021E_5900;
/// `ov01_02206478`.
pub const CAMERA_PRESETS_ADDRESS: u32 = 0x0220_6478;
/// Presets in the table (`FieldCamera_Create`'s bound).
pub const CAMERA_PRESET_COUNT: usize = 17;
/// Bytes per preset.
pub const CAMERA_PRESET_SIZE: usize = 0x24;
/// `ov01_022074A8`.
pub const SPRITE_MODEL_TABLE_ADDRESS: u32 = 0x0220_74A8;
/// Bytes per sprite-table row.
pub const SPRITE_MODEL_ENTRY_SIZE: usize = 6;
/// The sprite id ending the table.
pub const SPRITE_MODEL_END: u16 = 0xFFFF;
/// `SPRITE_HERO` (`include/constants/sprites.h:4`).
pub const SPRITE_HERO: u16 = 0;
/// `SPRITE_HEROINE` (`include/constants/sprites.h:65`).
pub const SPRITE_HEROINE: u16 = 97;
/// NitroFS path of the move-model (map object texture) archive.
pub const MMODEL_NARC: &str = "a/0/8/1";
/// `ov01_02206BD8`: the land model's base scale (`VecFx32`).
pub const LAND_BASE_SCALE_ADDRESS: u32 = 0x0220_6BD8;
/// `ov01_02206BE4`: the land model's base rotation (`MtxFx33`, NNS
/// row-vector layout).
pub const LAND_BASE_ROTATION_ADDRESS: u32 = 0x0220_6BE4;

/// The player's sprite id for a save-file gender (0 male, else female).
#[must_use]
pub fn player_sprite(gender: u8) -> u16 {
    if gender == 0 { SPRITE_HERO } else { SPRITE_HEROINE }
}

/// One camera preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CameraPreset {
    /// Distance from the target, fx32.
    pub distance: i32,
    /// Rotation about x, y, z as u16 angles (65536 = a turn).
    pub angle: [u16; 3],
    /// The padding word after the angles (0 in every retail preset).
    pub padding: u16,
    /// `perspectiveType`: 0 perspective, 1 orthographic.
    pub perspective_type: u16,
    /// `fovyAngle` (u16 angle).
    pub fovy_angle: u16,
    /// Near clipping plane, fx32.
    pub near: i32,
    /// Far clipping plane, fx32.
    pub far: i32,
    /// `lookAtOffset`, fx32.
    pub look_at_offset: [i32; 3],
}

impl CameraPreset {
    /// Decodes one 0x24-byte record.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the record is short.
    pub fn parse(record: &[u8]) -> Result<Self, NdsError> {
        Ok(Self {
            distance: u32le(record, 0)? as i32,
            angle: [u16le(record, 4)?, u16le(record, 6)?, u16le(record, 8)?],
            padding: u16le(record, 10)?,
            perspective_type: u16le(record, 12)?,
            fovy_angle: u16le(record, 14)?,
            near: u32le(record, 16)? as i32,
            far: u32le(record, 20)? as i32,
            look_at_offset: [
                u32le(record, 24)? as i32,
                u32le(record, 28)? as i32,
                u32le(record, 32)? as i32,
            ],
        })
    }
}

/// The 17 camera presets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraPresets {
    presets: Vec<CameraPreset>,
}

impl CameraPresets {
    /// Reads the table from the overlay.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the overlay is missing or corrupt.
    pub fn load(store: &AssetStore) -> Result<Self, AssetsError> {
        Self::from_overlay(&Ov01::load(store)?)
    }

    /// Reads the table from an already-loaded overlay image.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the table lies outside the image.
    pub fn from_overlay(ov: &Ov01) -> Result<Self, AssetsError> {
        let bytes = ov.bytes(
            CAMERA_PRESETS_ADDRESS,
            CAMERA_PRESET_COUNT * CAMERA_PRESET_SIZE,
            "camera presets",
        )?;
        let presets = bytes
            .chunks_exact(CAMERA_PRESET_SIZE)
            .map(CameraPreset::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| AssetsError::Corrupt {
                what: "camera presets".into(),
                source,
            })?;
        Ok(Self { presets })
    }

    /// Preset `camera_type`, when below [`CAMERA_PRESET_COUNT`].
    #[must_use]
    pub fn get(&self, camera_type: u8) -> Option<&CameraPreset> {
        self.presets.get(usize::from(camera_type))
    }

    /// All presets in index order.
    #[must_use]
    pub fn presets(&self) -> &[CameraPreset] {
        &self.presets
    }
}

/// One sprite-table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpriteModelEntry {
    /// `spriteId` (`SPRITE_*`).
    pub sprite_id: u16,
    /// `mmodelId` (`MMODEL_*`, the `a/0/8/1` member).
    pub mmodel_id: u16,
    /// The packed third word (size / flags; bits 10+ a category).
    pub packed: u16,
}

/// The sprite → move-model table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteModelTable {
    entries: Vec<SpriteModelEntry>,
}

impl SpriteModelTable {
    /// Reads the table from the overlay.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the overlay is missing or corrupt.
    pub fn load(store: &AssetStore) -> Result<Self, AssetsError> {
        Self::from_overlay(&Ov01::load(store)?)
    }

    /// Reads the table from an already-loaded overlay image, walking
    /// rows until the terminator.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the table runs off the image
    /// without a terminator.
    pub fn from_overlay(ov: &Ov01) -> Result<Self, AssetsError> {
        let mut entries = Vec::new();
        let mut address = SPRITE_MODEL_TABLE_ADDRESS;
        loop {
            let row = ov.bytes(address, SPRITE_MODEL_ENTRY_SIZE, "sprite model table")?;
            let sprite_id = u16::from_le_bytes([row[0], row[1]]);
            if sprite_id == SPRITE_MODEL_END {
                break;
            }
            entries.push(SpriteModelEntry {
                sprite_id,
                mmodel_id: u16::from_le_bytes([row[2], row[3]]),
                packed: u16::from_le_bytes([row[4], row[5]]),
            });
            address += SPRITE_MODEL_ENTRY_SIZE as u32;
        }
        Ok(Self { entries })
    }

    /// `GetMoveModelNoBySpriteId`: the first row for `sprite_id`.
    #[must_use]
    pub fn model_for_sprite(&self, sprite_id: u16) -> Option<u16> {
        self.entry(sprite_id).map(|e| e.mmodel_id)
    }

    /// `ObjectEvent_GetGraphicsInfo`: the first row for `sprite_id`.
    #[must_use]
    pub fn entry(&self, sprite_id: u16) -> Option<&SpriteModelEntry> {
        self.entries.iter().find(|e| e.sprite_id == sprite_id)
    }

    /// All rows before the terminator.
    #[must_use]
    pub fn entries(&self) -> &[SpriteModelEntry] {
        &self.entries
    }
}

/// The base rotation and scale every land model is drawn with
/// (`MapLoadManager_RenderLoadedMap`); the translation is the cell
/// origin. Identity and unit in retail, but read from the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LandBase {
    /// Rotation in column-vector form (`v' = R · v`), fx32.
    pub rotation: [[i32; 3]; 3],
    /// Per-axis scale, fx32.
    pub scale: [i32; 3],
}

impl LandBase {
    /// Identity rotation, unit scale — what retail stores.
    pub const IDENTITY: Self = Self {
        rotation: Placement::IDENTITY.rotation,
        scale: [FX32_ONE; 3],
    };

    /// Decodes the 12-byte scale vector and the 36-byte row-vector
    /// matrix (`_00 _01 _02 _10 … _22`), transposing the latter into
    /// column-vector form.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when either record is short.
    pub fn parse(scale: &[u8], rotation: &[u8]) -> Result<Self, NdsError> {
        let word = |b: &[u8], i: usize| u32le(b, i * 4).map(|v| v as i32);
        let mut out = Self::IDENTITY;
        for (axis, s) in out.scale.iter_mut().enumerate() {
            *s = word(scale, axis)?;
        }
        for r in 0..3 {
            for c in 0..3 {
                out.rotation[r][c] = word(rotation, c * 3 + r)?;
            }
        }
        Ok(out)
    }

    /// The placement of a land model whose cell origin is `origin`.
    #[must_use]
    pub const fn at(&self, origin: [i32; 3]) -> Placement {
        Placement {
            translation: origin,
            rotation: self.rotation,
            scale: self.scale,
        }
    }
}

/// The field overlay's image in RAM order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ov01 {
    /// The load address from the overlay table.
    pub ram_address: u32,
    image: Vec<u8>,
}

impl Ov01 {
    /// Loads and decompresses overlay 1.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the overlay is missing or corrupt,
    /// or loads somewhere other than [`LOAD_ADDRESS`].
    pub fn load(store: &AssetStore) -> Result<Self, AssetsError> {
        let (entry, image) = store.overlay(OVERLAY_ID)?;
        if entry.ram_address != LOAD_ADDRESS {
            return Err(AssetsError::Corrupt {
                what: "ARM9 overlay 1 load address".into(),
                source: NdsError::Invalid {
                    what: "overlay 1 load address",
                },
            });
        }
        Ok(Self {
            ram_address: entry.ram_address,
            image,
        })
    }

    /// The land model's base transform (`ov01_02206BD8` / `ov01_02206BE4`).
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when either record lies outside the
    /// image.
    pub fn land_base(&self) -> Result<LandBase, AssetsError> {
        let scale = self.bytes(LAND_BASE_SCALE_ADDRESS, 12, "land base scale")?;
        let rotation = self.bytes(LAND_BASE_ROTATION_ADDRESS, 36, "land base rotation")?;
        LandBase::parse(scale, rotation).map_err(|source| AssetsError::Corrupt {
            what: "land base transform".into(),
            source,
        })
    }

    /// `len` bytes at RAM `address`.
    ///
    /// # Errors
    /// Returns [`AssetsError::Missing`] naming `what` when the range lies
    /// outside the image.
    pub fn bytes(&self, address: u32, len: usize, what: &str) -> Result<&[u8], AssetsError> {
        let start = address
            .checked_sub(self.ram_address)
            .map(|o| o as usize)
            .ok_or_else(|| AssetsError::Missing(format!("{what} below overlay 1")))?;
        self.image
            .get(start..start + len)
            .ok_or_else(|| AssetsError::Missing(format!("{what} past overlay 1")))
    }

    /// The image length (`.text` + `.rodata` + `.data`; no `.bss`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.image.len()
    }

    /// Whether the image is empty (never, for a loaded overlay).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.image.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_record_layout() {
        let mut r = [0u8; CAMERA_PRESET_SIZE];
        r[..4].copy_from_slice(&0x0061_B89Bu32.to_le_bytes());
        r[4..6].copy_from_slice(&0xDC82u16.to_le_bytes());
        r[12] = 1;
        r[14..16].copy_from_slice(&0x0281u16.to_le_bytes());
        r[16..20].copy_from_slice(&0x0009_6000u32.to_le_bytes());
        r[20..24].copy_from_slice(&0x006C_7000u32.to_le_bytes());
        r[28..32].copy_from_slice(&(-4096i32).to_le_bytes());
        let p = CameraPreset::parse(&r).unwrap();
        assert_eq!(p.distance, 0x0061_B89B);
        assert_eq!(p.angle, [0xDC82, 0, 0]);
        assert_eq!((p.perspective_type, p.fovy_angle), (1, 0x0281));
        assert_eq!((p.near, p.far), (0x0009_6000, 0x006C_7000));
        assert_eq!(p.look_at_offset, [0, -4096, 0]);
    }

    #[test]
    fn overlay_tables_walk_the_image() {
        let mut image = vec![0u8; 64];
        // Sprite table at offset 8: (0 → 69), (97 → 70), terminator.
        image[8..14].copy_from_slice(&[0, 0, 69, 0, 0x60, 0x1C]);
        image[14..20].copy_from_slice(&[97, 0, 70, 0, 0x60, 0x1C]);
        image[20..22].copy_from_slice(&0xFFFFu16.to_le_bytes());
        let ov = Ov01 {
            ram_address: SPRITE_MODEL_TABLE_ADDRESS - 8,
            image,
        };
        let t = SpriteModelTable::from_overlay(&ov).unwrap();
        assert_eq!(t.entries().len(), 2);
        assert_eq!(t.model_for_sprite(SPRITE_HERO), Some(69));
        assert_eq!(t.model_for_sprite(SPRITE_HEROINE), Some(70));
        assert_eq!(t.model_for_sprite(5), None);
        assert!(ov.bytes(ov.ram_address - 1, 1, "x").is_err());
        assert!(ov.bytes(ov.ram_address + 60, 8, "x").is_err());
        assert!(CameraPresets::from_overlay(&ov).is_err());
        assert_eq!(player_sprite(0), SPRITE_HERO);
        assert_eq!(player_sprite(1), SPRITE_HEROINE);
    }

    #[test]
    fn land_base_transposes_the_row_vector_matrix() {
        let mut scale = Vec::new();
        for s in [FX32_ONE, 2 * FX32_ONE, 3 * FX32_ONE] {
            scale.extend_from_slice(&s.to_le_bytes());
        }
        // Row-vector 90° about Y: rows [[0,0,-1],[0,1,0],[1,0,0]].
        let rows = [[0, 0, -FX32_ONE], [0, FX32_ONE, 0], [FX32_ONE, 0, 0]];
        let mut rotation = Vec::new();
        for row in rows {
            for v in row {
                rotation.extend_from_slice(&v.to_le_bytes());
            }
        }
        let base = LandBase::parse(&scale, &rotation).unwrap();
        assert_eq!(base.scale, [FX32_ONE, 2 * FX32_ONE, 3 * FX32_ONE]);
        assert_eq!(base.rotation, [[0, 0, FX32_ONE], [0, FX32_ONE, 0], [-FX32_ONE, 0, 0]]);
        let p = base.at([10, 20, 30]);
        // (1, 1, 1) → scale (1, 2, 3) → rotate: x' = z, z' = -x.
        assert_eq!(p.apply([FX32_ONE; 3]), [10 + 3 * FX32_ONE, 20 + 2 * FX32_ONE, 30 - FX32_ONE]);
        assert!(LandBase::parse(&scale[..8], &rotation).is_err());
        let mut image = vec![0u8; 0x40];
        image[..12].copy_from_slice(&scale);
        image[12..48].copy_from_slice(&rotation);
        let ov = Ov01 {
            ram_address: LAND_BASE_SCALE_ADDRESS,
            image,
        };
        assert_eq!(ov.land_base().unwrap(), base);
    }
}
