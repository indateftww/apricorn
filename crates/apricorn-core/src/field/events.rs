//! Map events — pret `src/map_events.c:154` (`MapEvents_ComputeRamHeader`)
//! over `include/map_events_internal.h`: NARC `a/0/3/2`
//! (`fielddata/eventdata/zone_event`), 491 members, each four
//! count-prefixed arrays in order:
//!
//! ```text
//! u32 numBg;    BgEvent[numBg]       20 B: u16 scriptId, type; s32 x, z, y; u16 dir, pad
//! u32 numObj;   ObjectEvent[numObj]  32 B: u16 id, spriteId, movement, type, eventFlag,
//!                                          scriptId; s16 facingDirection; u16 param[3];
//!                                          s16 xRange, yRange; u16 x, z; s32 y
//! u32 numWarp;  WarpEvent[numWarp]   12 B: u16 x, z, header, anchor; u32 y
//! u32 numCoord; CoordEvent[numCoord] 16 B: u16 scriptId; s16 x, z; u16 w, h, y, val, var
//! ```
//!
//! The game reads a member into a 0x800-byte buffer
//! (`MapEvents::event_data`); every retail member fits.

use crate::assets::{AssetStore, AssetsError};
use crate::nds::{NdsError, u16le, u32le};

/// NitroFS path of the events archive.
pub const EVENTS_NARC: &str = "a/0/3/2";
/// Members in the retail archive.
pub const EVENTS_COUNT: usize = 491;
/// `sizeof(BgEvent)`.
pub const BG_EVENT_SIZE: usize = 20;
/// `sizeof(ObjectEvent)`.
pub const OBJECT_EVENT_SIZE: usize = 32;
/// `sizeof(WarpEvent)`.
pub const WARP_EVENT_SIZE: usize = 12;
/// `sizeof(CoordEvent)`.
pub const COORD_EVENT_SIZE: usize = 16;
/// The game's member buffer (`MapEvents::event_data`).
pub const MAX_BYTES: usize = 0x800;

/// A background (signpost / hidden item) event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BgEvent {
    /// `scriptId`.
    pub script_id: u16,
    /// `type`.
    pub kind: u16,
    /// Tile x.
    pub x: i32,
    /// Tile z.
    pub z: i32,
    /// Height.
    pub y: i32,
    /// `dir`: the facing that triggers it.
    pub dir: u16,
}

/// A map object (NPC / item ball) event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectEvent {
    /// `id` (local object id).
    pub id: u16,
    /// `spriteId` — see [`super::ov01::SpriteModelTable`].
    pub sprite_id: u16,
    /// `movement` type.
    pub movement: u16,
    /// `type`.
    pub kind: u16,
    /// `eventFlag`: the flag hiding the object.
    pub event_flag: u16,
    /// `scriptId`.
    pub script_id: u16,
    /// `facingDirection`.
    pub facing_direction: i16,
    /// `param[3]`.
    pub param: [u16; 3],
    /// `xRange`.
    pub x_range: i16,
    /// `yRange`.
    pub y_range: i16,
    /// Tile x.
    pub x: u16,
    /// Tile z.
    pub z: u16,
    /// Height.
    pub y: i32,
}

/// A warp (door / stairs / edge) event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WarpEvent {
    /// Tile x.
    pub x: u16,
    /// Tile z.
    pub z: u16,
    /// `header`: the destination map id.
    pub header: u16,
    /// `anchor`: the destination warp index.
    pub anchor: u16,
    /// Height.
    pub y: u32,
}

/// A coordinate-trigger event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoordEvent {
    /// `scriptId`.
    pub script_id: u16,
    /// Tile x of the area's corner.
    pub x: i16,
    /// Tile z of the area's corner.
    pub z: i16,
    /// Width in tiles.
    pub w: u16,
    /// Height (depth) in tiles.
    pub h: u16,
    /// Height.
    pub y: u16,
    /// `val`: the value `var` must hold.
    pub val: u16,
    /// `var`: the game variable checked.
    pub var: u16,
}

/// One member's events.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MapEvents {
    /// `bg_events`.
    pub bg: Vec<BgEvent>,
    /// `object_events`.
    pub objects: Vec<ObjectEvent>,
    /// `warp_events`.
    pub warps: Vec<WarpEvent>,
    /// `coord_events`.
    pub coords: Vec<CoordEvent>,
}

impl MapEvents {
    /// Decodes a member.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when a count overruns the member or bytes
    /// remain after the last array.
    pub fn parse(bytes: &[u8]) -> Result<Self, NdsError> {
        let mut p = 0usize;
        let count = |p: &mut usize| -> Result<usize, NdsError> {
            let n = u32le(bytes, *p)? as usize;
            *p += 4;
            Ok(n)
        };
        let n = count(&mut p)?;
        let mut bg = Vec::with_capacity(n);
        for _ in 0..n {
            bg.push(BgEvent {
                script_id: u16le(bytes, p)?,
                kind: u16le(bytes, p + 2)?,
                x: u32le(bytes, p + 4)? as i32,
                z: u32le(bytes, p + 8)? as i32,
                y: u32le(bytes, p + 12)? as i32,
                dir: u16le(bytes, p + 16)?,
            });
            p += BG_EVENT_SIZE;
        }
        let n = count(&mut p)?;
        let mut objects = Vec::with_capacity(n);
        for _ in 0..n {
            objects.push(ObjectEvent {
                id: u16le(bytes, p)?,
                sprite_id: u16le(bytes, p + 2)?,
                movement: u16le(bytes, p + 4)?,
                kind: u16le(bytes, p + 6)?,
                event_flag: u16le(bytes, p + 8)?,
                script_id: u16le(bytes, p + 10)?,
                facing_direction: u16le(bytes, p + 12)? as i16,
                param: [u16le(bytes, p + 14)?, u16le(bytes, p + 16)?, u16le(bytes, p + 18)?],
                x_range: u16le(bytes, p + 20)? as i16,
                y_range: u16le(bytes, p + 22)? as i16,
                x: u16le(bytes, p + 24)?,
                z: u16le(bytes, p + 26)?,
                y: u32le(bytes, p + 28)? as i32,
            });
            p += OBJECT_EVENT_SIZE;
        }
        let n = count(&mut p)?;
        let mut warps = Vec::with_capacity(n);
        for _ in 0..n {
            warps.push(WarpEvent {
                x: u16le(bytes, p)?,
                z: u16le(bytes, p + 2)?,
                header: u16le(bytes, p + 4)?,
                anchor: u16le(bytes, p + 6)?,
                y: u32le(bytes, p + 8)?,
            });
            p += WARP_EVENT_SIZE;
        }
        let n = count(&mut p)?;
        let mut coords = Vec::with_capacity(n);
        for _ in 0..n {
            coords.push(CoordEvent {
                script_id: u16le(bytes, p)?,
                x: u16le(bytes, p + 2)? as i16,
                z: u16le(bytes, p + 4)? as i16,
                w: u16le(bytes, p + 6)?,
                h: u16le(bytes, p + 8)?,
                y: u16le(bytes, p + 10)?,
                val: u16le(bytes, p + 12)?,
                var: u16le(bytes, p + 14)?,
            });
            p += COORD_EVENT_SIZE;
        }
        if p != bytes.len() {
            return Err(NdsError::Invalid {
                what: "map events length",
            });
        }
        Ok(Self {
            bg,
            objects,
            warps,
            coords,
        })
    }

    /// Loads member `bank` (`MapHeader::events_bank`).
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the member is missing or malformed.
    pub fn load(store: &AssetStore, bank: u16) -> Result<Self, AssetsError> {
        let bytes = store.member(EVENTS_NARC, usize::from(bank))?;
        Self::parse(&bytes).map_err(|source| AssetsError::Corrupt {
            what: format!("{EVENTS_NARC}#{bank}"),
            source,
        })
    }

    /// `Field_GetWarpEventAtXYPos`: the first warp on tile `(x, z)`.
    #[must_use]
    pub fn warp_at(&self, x: u16, z: u16) -> Option<usize> {
        self.warps.iter().position(|w| w.x == x && w.z == z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrays_are_count_prefixed_and_exact() {
        let mut b = Vec::new();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        let mut obj = [0u8; OBJECT_EVENT_SIZE];
        obj[2..4].copy_from_slice(&376u16.to_le_bytes());
        obj[12..14].copy_from_slice(&(-1i16).to_le_bytes());
        obj[24..26].copy_from_slice(&8u16.to_le_bytes());
        obj[26..28].copy_from_slice(&10u16.to_le_bytes());
        b.extend_from_slice(&obj);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[3, 0, 4, 0, 63, 0, 1, 0, 0, 0, 0, 0]);
        b.extend_from_slice(&0u32.to_le_bytes());
        let e = MapEvents::parse(&b).unwrap();
        assert_eq!(e.objects[0].sprite_id, 376);
        assert_eq!(e.objects[0].facing_direction, -1);
        assert_eq!((e.objects[0].x, e.objects[0].z), (8, 10));
        assert_eq!(e.warps[0], WarpEvent { x: 3, z: 4, header: 63, anchor: 1, y: 0 });
        assert_eq!(e.warp_at(3, 4), Some(0));
        assert_eq!(e.warp_at(4, 3), None);
        b.push(0);
        assert!(MapEvents::parse(&b).is_err());
        assert!(MapEvents::parse(&[5, 0, 0, 0]).is_err());
    }
}
