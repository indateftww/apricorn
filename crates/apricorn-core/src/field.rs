//! ROM-backed field landing. The first map is T20R0202 (New Bark 2F).
//! Geometry stays fixed-point in the logical frame; only the presenter
//! projects it. This is the static field entry, not the Phase 5 script VM.
pub mod model;

use crate::{
    assets::{AssetStore, AssetsError},
    formats::Btx,
    nds::{NdsError, u16le, u32le},
};
use model::{Mesh, Texture};

/// The room's map geometry, props and player billboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldScene {
    /// Map-header ID (64 for the player's bedroom).
    pub map_id: u16,
    /// Ground, walls and furniture in world coordinates.
    pub meshes: Vec<Mesh>,
    /// The south-facing player map-object image.
    pub player: Texture,
    /// Position in tile coordinates; one tile is 16 world units.
    pub position: [i32; 2],
}

impl FieldScene {
    /// `map_headers.h[64]`, matrix 72, area bank 25; the matrix chooses
    /// the land member, and area data chooses both texture archives.
    pub fn bedroom(store: &AssetStore, gender: u8) -> Result<Self, AssetsError> {
        let parse = |what: &str, source| AssetsError::Corrupt {
            what: what.into(),
            source,
        };
        let matrix = store.member("a/0/4/1", 72)?;
        let area = store.member("a/0/4/2", 25)?;
        let result = (|| -> Result<usize, NdsError> {
            if matrix.get(..4) != Some(&[1, 1, 0, 0]) {
                return Err(NdsError::Invalid {
                    what: "bedroom matrix",
                });
            }
            Ok(u16le(&matrix, 5 + matrix[4] as usize)? as usize)
        })()
        .map_err(|e| parse("bedroom matrix", e))?;
        let land = store.member("a/0/6/5", result)?;
        let texture_id = u16le(&area, 2).map_err(|e| parse("area texture", e))? as usize;
        let prop_id = u16le(&area, 0).map_err(|e| parse("area props", e))? as usize;
        let texture_bytes = store.member("a/0/4/4", texture_id)?;
        let prop_bytes = store.member("a/0/7/0", prop_id)?;
        let tex = Btx::parse(&texture_bytes).map_err(|e| parse("room textures", e))?;
        let props = Btx::parse(&prop_bytes).map_err(|e| parse("prop textures", e))?;
        let attr_size = u32le(&land, 0).map_err(|e| parse("land header", e))? as usize;
        let prop_size = u32le(&land, 4).map_err(|e| parse("land header", e))? as usize;
        let mdl_size = u32le(&land, 8).map_err(|e| parse("land header", e))? as usize;
        let start = 20 + attr_size + prop_size;
        let raw = land
            .get(start..start + mdl_size)
            .ok_or_else(|| AssetsError::Missing("land model".into()))?;
        // Land geometry is centered on its 32x32-tile matrix cell.
        let origin = [256 * 4096, 0, 256 * 4096];
        let mut meshes = model::parse(raw, &tex, origin).map_err(|e| parse("room model", e))?;
        let records = land
            .get(20 + attr_size..start)
            .ok_or_else(|| AssetsError::Missing("land props".into()))?;
        for record in records.chunks_exact(48) {
            let id = u32le(record, 0).map_err(|e| parse("prop ID", e))? as usize;
            let mut translation = origin;
            for axis in 0..3 {
                translation[axis] += u32le(record, 4 + axis * 4).unwrap() as i32;
            }
            // All eight bedroom prop placements are unrotated with unit scale.
            if record[16..28] != [0; 12]
                || (0..3).any(|i| u32le(record, 28 + i * 4).unwrap() != 4096)
            {
                return Err(parse(
                    "bedroom prop transform",
                    NdsError::Invalid {
                        what: "nonidentity placement",
                    },
                ));
            }
            let bytes = store.member("a/1/4/8", id)?;
            meshes.extend(
                model::parse(&bytes, &props, translation)
                    .map_err(|e| parse(&format!("prop {id}"), e))?,
            );
        }
        let player_bytes = store.member("a/0/8/1", 69 + usize::from(gender))?;
        let btx = Btx::parse(&player_bytes).map_err(|e| parse("player texture", e))?;
        let player =
            model::decode_texture(&btx, btx.textures()[0].name(), btx.palettes()[0].name())
                .map_err(|e| parse("player pixels", e))?;
        Ok(Self {
            map_id: 64,
            meshes,
            player,
            position: [6, 6],
        })
    }
}
