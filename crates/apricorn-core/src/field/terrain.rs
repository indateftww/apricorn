//! Terrain attributes over a window of matrix cells — the counterpart of
//! pret's `TerrainAttributes` cache (`src/terrain_attributes.c:31`, up to
//! `TERRAIN_ATTRIBUTES_MAX_BLOCK_COUNT` = 16 distinct land blocks of
//! 32 × 32 words) and of the ov01 lookup behind `GetMetatileBehavior`
//! (`asm/unk_02054648.s:431`): a tile whose cell is not resident answers
//! `TILE_BEHAVIOR_NONE` (0xFF), and here also counts as impassable.
//!
//! Coordinates are matrix-wide tile coordinates: cell `(x / 32, z / 32)`,
//! word `(x % 32) + (z % 32) * 32` (see [`super::land`]).

use super::land::{self, ATTRIBUTE_COUNT};
use super::matrix::CELL_TILES;

/// `TILE_BEHAVIOR_NONE`: the behavior of a tile with no loaded cell.
pub const TILE_BEHAVIOR_NONE: u8 = 0xFF;
/// `TERRAIN_ATTRIBUTES_MAX_BLOCK_COUNT`: distinct blocks pret's cache holds.
pub const MAX_BLOCKS: usize = 16;

/// Attribute blocks for a rectangle of cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainAttributes {
    x0: i32,
    z0: i32,
    width: usize,
    height: usize,
    blocks: Vec<Option<Box<[u16; ATTRIBUTE_COUNT]>>>,
}

impl TerrainAttributes {
    /// An empty window covering cells `x0..x0+width`, `z0..z0+height`.
    #[must_use]
    pub fn new(x0: i32, z0: i32, width: usize, height: usize) -> Self {
        Self {
            x0,
            z0,
            width,
            height,
            blocks: vec![None; width * height],
        }
    }

    /// The window as `(x0, z0, width, height)` in cells.
    #[must_use]
    pub fn window(&self) -> (i32, i32, usize, usize) {
        (self.x0, self.z0, self.width, self.height)
    }

    fn slot(&self, cell_x: i32, cell_z: i32) -> Option<usize> {
        let (dx, dz) = (cell_x - self.x0, cell_z - self.z0);
        if dx < 0 || dz < 0 || dx as usize >= self.width || dz as usize >= self.height {
            return None;
        }
        Some(dz as usize * self.width + dx as usize)
    }

    /// Stores a cell's attributes; `false` when the cell lies outside the
    /// window (nothing is stored).
    pub fn insert(&mut self, cell_x: i32, cell_z: i32, attributes: &[u16; ATTRIBUTE_COUNT]) -> bool {
        match self.slot(cell_x, cell_z) {
            Some(i) => {
                self.blocks[i] = Some(Box::new(*attributes));
                true
            }
            None => false,
        }
    }

    /// A resident cell's block.
    #[must_use]
    pub fn cell(&self, cell_x: i32, cell_z: i32) -> Option<&[u16; ATTRIBUTE_COUNT]> {
        self.blocks.get(self.slot(cell_x, cell_z)?)?.as_deref()
    }

    /// Resident cells in the window.
    #[must_use]
    pub fn loaded_cells(&self) -> usize {
        self.blocks.iter().filter(|b| b.is_some()).count()
    }

    /// The attribute word of tile `(x, z)`, when its cell is resident.
    #[must_use]
    pub fn attr(&self, x: i32, z: i32) -> Option<u16> {
        let block = self.cell(x.div_euclid(CELL_TILES), z.div_euclid(CELL_TILES))?;
        let (tx, tz) = (x.rem_euclid(CELL_TILES) as usize, z.rem_euclid(CELL_TILES) as usize);
        Some(block[tx + tz * CELL_TILES as usize])
    }

    /// `GetMetatileBehavior`: bits 0–7, or [`TILE_BEHAVIOR_NONE`] outside
    /// the resident cells.
    #[must_use]
    pub fn behavior(&self, x: i32, z: i32) -> u8 {
        self.attr(x, z).map_or(TILE_BEHAVIOR_NONE, land::behavior)
    }

    /// Whether tile `(x, z)` is impassable; tiles outside the resident
    /// cells cannot be entered.
    #[must_use]
    pub fn impassable(&self, x: i32, z: i32) -> bool {
        self.attr(x, z).is_none_or(land::impassable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_lookups_use_cell_then_word_indexing() {
        let mut t = TerrainAttributes::new(20, 11, 3, 3);
        let mut block = [0u16; ATTRIBUTE_COUNT];
        block[5 + 7 * 32] = 0x8086;
        block[0] = 0x005F;
        assert!(t.insert(21, 12, &block));
        assert!(!t.insert(23, 12, &block));
        assert_eq!(t.loaded_cells(), 1);
        assert_eq!(t.attr(21 * 32 + 5, 12 * 32 + 7), Some(0x8086));
        assert_eq!(t.behavior(21 * 32 + 5, 12 * 32 + 7), 0x86);
        assert!(t.impassable(21 * 32 + 5, 12 * 32 + 7));
        assert_eq!(t.behavior(21 * 32, 12 * 32), 0x5F);
        assert!(!t.impassable(21 * 32, 12 * 32));
        // Inside the window but not resident, and outside the window.
        assert_eq!(t.attr(20 * 32, 11 * 32), None);
        assert_eq!(t.behavior(20 * 32, 11 * 32), TILE_BEHAVIOR_NONE);
        assert!(t.impassable(-1, -1));
        assert_eq!(t.window(), (20, 11, 3, 3));
        assert!(t.cell(21, 12).is_some() && t.cell(22, 12).is_none());
    }
}
