//! Map matrices — pret `src/map_matrix.c:15` (`MapMatrix_MapMatrixData_Load`)
//! and `include/map_matrix.h` (`MAP_MATRIX_MAX_SIZE` 799, name length 16).
//! NARC `a/0/4/1` (`fielddata/mapmatrix/map_matrix`), 288 members.
//!
//! Member layout: `u8 width, height, hasHeaders, hasAltitudes, nameLength;
//! u8 name[nameLength]; [u16 headers[w*h]]; [u8 altitudes[w*h]];
//! u16 landDataIds[w*h]` — cell index `z * width + x`. Without a headers
//! section every cell reports the loading map (`MIi_CpuClear16(map_no)`);
//! without altitudes the game leaves its zero-filled buffer.
//!
//! Cell origins follow `ov01_021F5FB8` (`asm/overlay_01_021F4704.s:3320`):
//! `x = (cellX << 21) + (1 << 20)`, `y = altitude << 15`,
//! `z = (cellZ << 21) + (1 << 20)` — a cell is 32 tiles of 16 units
//! (512 units) with its model centred, and one altitude step is 8 units.

use crate::assets::{AssetStore, AssetsError};
use crate::nds::{NdsError, u16le};

/// NitroFS path of the matrix archive.
pub const MATRIX_NARC: &str = "a/0/4/1";
/// Members in the retail archive.
pub const MATRIX_COUNT: usize = 288;
/// `MAP_MATRIX_MAX_SIZE`: the largest cell count (matrix 0 is 47 × 17).
pub const MAX_CELLS: usize = 799;
/// `MAP_MATRIX_MAX_NAME_LENGTH`.
pub const MAX_NAME_LENGTH: usize = 16;
/// Tiles along each edge of a cell (`MAP_TILES_COUNT_X/Z`).
pub const CELL_TILES: i32 = 32;
/// World units per tile (fx32 integer part).
pub const TILE_UNITS: i32 = 16;
/// World units per cell edge.
pub const CELL_UNITS: i32 = CELL_TILES * TILE_UNITS;
/// World units per altitude step (`altitude << 15` in fx32).
pub const ALTITUDE_UNITS: i32 = 8;
/// A land-data id marking a cell with no map (matrix 0's void).
pub const NO_LAND: u16 = 0xFFFF;

/// One parsed matrix member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapMatrix {
    /// Member index in [`MATRIX_NARC`].
    pub id: u16,
    /// Cells across.
    pub width: u8,
    /// Cells down.
    pub height: u8,
    /// The embedded name (`m_hh0102_` for the bedroom matrix).
    pub name: String,
    /// Whether the member carried a headers section.
    pub has_headers: bool,
    /// Whether the member carried an altitudes section.
    pub has_altitudes: bool,
    /// Map header id per cell (`z * width + x`); the loading map's id
    /// everywhere when [`Self::has_headers`] is false.
    pub headers: Vec<u16>,
    /// Altitude per cell; zeros when [`Self::has_altitudes`] is false.
    pub altitudes: Vec<u8>,
    /// Land-data member (`a/0/6/5`) per cell, [`NO_LAND`] for none.
    pub land_ids: Vec<u16>,
}

impl MapMatrix {
    /// Decodes member `id`'s bytes; `map_id` fills the headers of a
    /// member without a headers section.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the member is truncated, its name is
    /// longer than [`MAX_NAME_LENGTH`], or it has more than [`MAX_CELLS`]
    /// cells.
    pub fn parse(id: u16, bytes: &[u8], map_id: u16) -> Result<Self, NdsError> {
        let truncated = |need: usize| NdsError::Truncated {
            what: "map matrix",
            need,
            got: bytes.len(),
        };
        if bytes.len() < 5 {
            return Err(truncated(5));
        }
        let (width, height) = (bytes[0], bytes[1]);
        let cells = usize::from(width) * usize::from(height);
        if cells == 0 || cells > MAX_CELLS {
            return Err(NdsError::Invalid {
                what: "map matrix size",
            });
        }
        let name_length = usize::from(bytes[4]);
        if name_length > MAX_NAME_LENGTH {
            return Err(NdsError::Invalid {
                what: "map matrix name length",
            });
        }
        let mut p = 5;
        let name = bytes.get(p..p + name_length).ok_or(truncated(p + name_length))?;
        let name = std::str::from_utf8(name)
            .map_err(|_| NdsError::Invalid {
                what: "map matrix name",
            })?
            .trim_end_matches('\0')
            .to_owned();
        p += name_length;
        let has_headers = bytes[2] != 0;
        let has_altitudes = bytes[3] != 0;
        let read_u16s = |p: &mut usize| -> Result<Vec<u16>, NdsError> {
            let v = (0..cells)
                .map(|i| u16le(bytes, *p + i * 2))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| truncated(*p + cells * 2))?;
            *p += cells * 2;
            Ok(v)
        };
        let headers = if has_headers {
            read_u16s(&mut p)?
        } else {
            vec![map_id; cells]
        };
        let altitudes = if has_altitudes {
            let a = bytes.get(p..p + cells).ok_or(truncated(p + cells))?.to_vec();
            p += cells;
            a
        } else {
            vec![0; cells]
        };
        let land_ids = read_u16s(&mut p)?;
        Ok(Self {
            id,
            width,
            height,
            name,
            has_headers,
            has_altitudes,
            headers,
            altitudes,
            land_ids,
        })
    }

    /// Loads matrix `id` for map `map_id` (`MapMatrix_Load`).
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the member is missing or malformed.
    pub fn load(store: &AssetStore, id: u16, map_id: u16) -> Result<Self, AssetsError> {
        let bytes = store.member(MATRIX_NARC, usize::from(id))?;
        Self::parse(id, &bytes, map_id).map_err(|source| AssetsError::Corrupt {
            what: format!("{MATRIX_NARC}#{id}"),
            source,
        })
    }

    /// `width * height`.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        usize::from(self.width) * usize::from(self.height)
    }

    /// Cell index of `(x, z)`, when inside the matrix.
    #[must_use]
    pub fn index(&self, x: i32, z: i32) -> Option<usize> {
        if x < 0 || z < 0 || x >= i32::from(self.width) || z >= i32::from(self.height) {
            return None;
        }
        Some(z as usize * usize::from(self.width) + x as usize)
    }

    /// `MapMatrix_GetMapHeader`.
    #[must_use]
    pub fn header(&self, x: i32, z: i32) -> Option<u16> {
        self.index(x, z).map(|i| self.headers[i])
    }

    /// `MapMatrix_GetMapAltitude`.
    #[must_use]
    pub fn altitude(&self, x: i32, z: i32) -> Option<u8> {
        self.index(x, z).map(|i| self.altitudes[i])
    }

    /// `MapMatrix_GetMapModelNo` by cell coordinates.
    #[must_use]
    pub fn land_id(&self, x: i32, z: i32) -> Option<u16> {
        self.index(x, z).map(|i| self.land_ids[i])
    }

    /// World origin (fx32) of cell `(x, z)` — `ov01_021F5FB8`.
    #[must_use]
    pub fn cell_origin(&self, x: i32, z: i32) -> Option<[i32; 3]> {
        let altitude = self.altitude(x, z)?;
        Some([
            (x << 21) + (1 << 20),
            i32::from(altitude) << 15,
            (z << 21) + (1 << 20),
        ])
    }

    /// The cell holding tile `(x, z)` in matrix-wide tile coordinates.
    #[must_use]
    pub fn cell_of_tile(x: i32, z: i32) -> (i32, i32) {
        (x.div_euclid(CELL_TILES), z.div_euclid(CELL_TILES))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(w: u8, h: u8, headers: bool, alts: bool) -> Vec<u8> {
        let n = usize::from(w) * usize::from(h);
        let mut b = vec![w, h, u8::from(headers), u8::from(alts), 3, b'a', b'b', b'c'];
        if headers {
            for i in 0..n {
                b.extend_from_slice(&(100 + i as u16).to_le_bytes());
            }
        }
        if alts {
            b.extend((0..n).map(|i| i as u8 + 1));
        }
        for i in 0..n {
            b.extend_from_slice(&(200 + i as u16).to_le_bytes());
        }
        b
    }

    #[test]
    fn full_member_parses_and_indexes_row_major() {
        let m = MapMatrix::parse(7, &member(3, 2, true, true), 5).unwrap();
        assert_eq!((m.width, m.height, m.name.as_str()), (3, 2, "abc"));
        assert_eq!(m.cell_count(), 6);
        assert_eq!(m.header(2, 1), Some(105));
        assert_eq!(m.altitude(2, 1), Some(6));
        assert_eq!(m.land_id(0, 1), Some(203));
        assert_eq!(m.index(3, 0), None);
        assert_eq!(m.index(0, -1), None);
        assert_eq!(m.cell_origin(1, 1), Some([768 << 12, 5 << 15, 768 << 12]));
    }

    #[test]
    fn missing_sections_fill_like_the_game() {
        let m = MapMatrix::parse(1, &member(1, 1, false, false), 64).unwrap();
        assert_eq!(m.headers, vec![64]);
        assert_eq!(m.altitudes, vec![0]);
        assert_eq!(m.land_ids, vec![200]);
        assert!(!m.has_headers && !m.has_altitudes);
        assert_eq!(m.cell_origin(0, 0), Some([256 << 12, 0, 256 << 12]));
    }

    #[test]
    fn malformed_members_are_rejected() {
        assert!(MapMatrix::parse(0, &[1, 1, 0, 0], 0).is_err());
        assert!(MapMatrix::parse(0, &[0, 1, 0, 0, 0], 0).is_err());
        assert!(MapMatrix::parse(0, &[1, 1, 0, 0, 17], 0).is_err());
        assert!(MapMatrix::parse(0, &member(2, 2, true, true)[..10], 0).is_err());
    }

    #[test]
    fn tile_to_cell_floors() {
        assert_eq!(MapMatrix::cell_of_tile(31, 32), (0, 1));
        assert_eq!(MapMatrix::cell_of_tile(-1, 0), (-1, 0));
    }
}
