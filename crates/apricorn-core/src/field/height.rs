//! The BDHC height solver: the field overlay's `ov01_021FAE50`
//! (`asm/overlay_01_021FAD1C.s:207`) behind `sub_02054654`
//! (`asm/unk_02054648.s:37`), the matrix-mode entry of the
//! `fieldSystem->unk60` table that `sub_02054940` dispatches to. The map
//! object's height refresh (`sub_02061070` through `sub_02061248`)
//! calls it when the height is stale and once per step of a walk.
//!
//! A cell's BDHC block ([`Bdhc`]) is a set of *plates*, each the
//! rectangle spanned by two corner *points* on the plane a *slope*
//! (a normal) and a *height* (the plane offset) define; z *strips*
//! index the plates, each strip naming a run of the *access list*. For
//! a position the solver binary-searches the strip, tests the plates
//! the strip lists for containment and evaluates each hit's plane as
//! `y = -(nx * x + nz * z + h) / ny`; with several hits the caller's
//! [`HeightMode`] picks the one nearest the current y (the movement
//! code's mode), the highest or the lowest.
//!
//! Positions are fx32 world units relative to the **cell centre**
//! (`sub_02054654` subtracts `(cell * 32 + 16) << 16` from the world
//! position; one tile is 16 units) and the heights come back absolute,
//! so New Bark Town's land surface answers `16 << 12` where the cell's
//! ground plane is.
//!
//! Element layouts, checked against the retail members in
//! `tests/field_system_hg.rs` (New Bark's ground plate at 16 units, the
//! bedroom's at 0):
//!
//! | array | element |
//! |---|---|
//! | points | `{fx32 x; fx32 z}` |
//! | slopes | `{fx32 nx; fx32 ny; fx32 nz}` |
//! | heights | `fx32 h` |
//! | plates | `{u16 point1; u16 point2; u16 slope; u16 height}` |
//! | strips | `{fx32 z; u16 count; u16 first}` |
//! | access list | `u16 plate` |
//!
//! Not ported: the dynamic terrain heights `sub_02054654` consults
//! after the plates (`fieldSystem->dynamicTerrainHeightManager`,
//! `ov01_021FB42C`), which scripts register for bridges and the like
//! (kind 2 of the lookup, gated by the object's `UNK29`); nothing on
//! the maps the field reaches registers one.

use super::LandCell;
use super::land::Bdhc;
use super::map_object::TILE_FX32;

/// How `ov01_021FAE50` picks among several plates under one position —
/// its first argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HeightMode {
    /// Mode 0: the plate whose height is nearest the current y (the
    /// map object's per-step refresh, `sub_02054774`).
    #[default]
    Nearest,
    /// Mode 1: the highest plate.
    Highest,
    /// Mode 2: the lowest plate.
    Lowest,
}

/// The most plates one lookup collects (the routine's stack array).
pub const MAX_HITS: usize = 10;

/// Bytes per point element.
const POINT: usize = 8;
/// Bytes per slope element.
const SLOPE: usize = 12;
/// Bytes per height element.
const HEIGHT: usize = 4;
/// Bytes per plate element.
const PLATE: usize = 8;
/// Bytes per strip element.
const STRIP: usize = 8;

fn i32le(bytes: &[u8], at: usize) -> Option<i32> {
    bytes
        .get(at..at + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u16le(bytes: &[u8], at: usize) -> Option<u16> {
    bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

/// `ov01_021FADEC`: the strip whose plates may cover `z` — the retail
/// binary search over the strips' z boundaries, ported step for step
/// (its answer on a boundary is the search's, not a textbook's).
/// `None` when there are no strips.
#[must_use]
pub fn find_strip(strips: &[u8], count: usize, z: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    if count == 1 {
        return Some(0);
    }
    let boundary = |i: usize| i32le(strips, i * STRIP);
    let mut hi = count - 1;
    let mut mid = hi / 2;
    let mut lo = 0usize;
    loop {
        let boundary_z = boundary(mid)?;
        if boundary_z > z {
            hi -= 1;
            if hi <= lo {
                return Some(mid);
            }
            let sum = lo + mid;
            hi = mid;
            mid = sum / 2;
        } else {
            lo += 1;
            if lo >= hi {
                return Some(mid + 1);
            }
            let sum = mid + hi;
            lo = mid;
            mid = sum / 2;
        }
    }
}

/// `_ll_mul` then `(product + 0x800) >> 12`, truncated to 32 bits.
fn mul_fx(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b) + 0x800) >> 12) as i32
}

/// `FX_Div`: `(a << 12) / b`, truncating toward zero; `None` for a
/// zero divisor (a degenerate slope).
fn fx_div(a: i32, b: i32) -> Option<i32> {
    if b == 0 {
        return None;
    }
    Some(((i64::from(a) << 12) / i64::from(b)) as i32)
}

/// `ov01_021FAD1C`: whether `(x, z)` lies within the rectangle the two
/// corners span, inclusive on all sides, corners in either order.
fn inside(corner_a: (i32, i32), corner_b: (i32, i32), x: i32, z: i32) -> bool {
    let (min_x, max_x) = if corner_a.0 > corner_b.0 {
        (corner_b.0, corner_a.0)
    } else {
        (corner_a.0, corner_b.0)
    };
    let (min_z, max_z) = if corner_a.1 > corner_b.1 {
        (corner_b.1, corner_a.1)
    } else {
        (corner_a.1, corner_b.1)
    };
    min_x <= x && x <= max_x && min_z <= z && z <= max_z
}

/// The plane height of plate `plate` at `(x, z)` when the plate covers
/// the position (`ov01_021FAD6C`, `ov01_021FAD1C`, `ov01_021FAD9C`,
/// `ov01_021FADBC` and the arithmetic of `ov01_021FAE50`'s loop body).
/// `None` when the plate does not cover it or the data is short.
fn plate_height_at(bdhc: &Bdhc, plate: usize, x: i32, z: i32) -> Option<i32> {
    let plates = bdhc.plates();
    let points = bdhc.points();
    let point1 = usize::from(u16le(plates, plate * PLATE)?);
    let point2 = usize::from(u16le(plates, plate * PLATE + 2)?);
    let corner_a = (
        i32le(points, point1 * POINT)?,
        i32le(points, point1 * POINT + 4)?,
    );
    let corner_b = (
        i32le(points, point2 * POINT)?,
        i32le(points, point2 * POINT + 4)?,
    );
    if !inside(corner_a, corner_b, x, z) {
        return None;
    }
    let slope = usize::from(u16le(plates, plate * PLATE + 4)?);
    let height = usize::from(u16le(plates, plate * PLATE + 6)?);
    let slopes = bdhc.slopes();
    let nx = i32le(slopes, slope * SLOPE)?;
    let ny = i32le(slopes, slope * SLOPE + 4)?;
    let nz = i32le(slopes, slope * SLOPE + 8)?;
    let h = i32le(bdhc.heights(), height * HEIGHT)?;
    let sum = mul_fx(nx, x).wrapping_add(mul_fx(nz, z)).wrapping_add(h);
    fx_div(sum.wrapping_neg(), ny)
}

/// `ov01_021FAE50(mode, y, x, z, bdhc, &out)`: the ground height under
/// `(x, z)` (fx32, relative to the cell centre) or `None` when no
/// plate covers it; `y` is the current height the nearest mode
/// measures from.
#[must_use]
pub fn plate_height(bdhc: &Bdhc, mode: HeightMode, y: i32, x: i32, z: i32) -> Option<i32> {
    let strips = bdhc.strips();
    let strip = find_strip(strips, usize::from(bdhc.counts[4]), z)?;
    let count = usize::from(u16le(strips, strip * STRIP + 4)?);
    let first = usize::from(u16le(strips, strip * STRIP + 6)?);
    let access = bdhc.access_list();
    let mut hits: Vec<i32> = Vec::with_capacity(MAX_HITS);
    for i in 0..count {
        let plate = usize::from(u16le(access, (first + i) * 2)?);
        if let Some(height) = plate_height_at(bdhc, plate, x, z) {
            hits.push(height);
            if hits.len() >= MAX_HITS {
                break;
            }
        }
    }
    match hits.len() {
        0 => None,
        1 => Some(hits[0]),
        _ => Some(hits[select(&hits, mode, y)]),
    }
}

/// The routine's choice among several hits, sentinels and strictness
/// as in the asm (`_021FAF6E`, `_021FAF96`, `_021FAFBE`).
fn select(hits: &[i32], mode: HeightMode, y: i32) -> usize {
    let mut best = 0usize;
    match mode {
        HeightMode::Highest => {
            let mut best_height = 0xFF00_0000_u32 as i32;
            for (i, &h) in hits.iter().enumerate() {
                if h > best_height {
                    best_height = h;
                    best = i;
                }
            }
        }
        HeightMode::Lowest => {
            let mut best_height = 0x0100_0000;
            for (i, &h) in hits.iter().enumerate() {
                if h < best_height {
                    best_height = h;
                    best = i;
                }
            }
        }
        HeightMode::Nearest => {
            let distance = |h: i32| y.max(h).wrapping_sub(y.min(h));
            let mut best_distance = distance(hits[0]);
            for (i, &h) in hits.iter().enumerate().skip(1) {
                let d = distance(h);
                if d < best_distance {
                    best_distance = d;
                    best = i;
                }
            }
        }
    }
    best
}

/// `sub_02054654`'s addressing for a resident cell set: the cell
/// holding world position `(x, z)` (fx32) and the position relative to
/// that cell's centre, then [`plate_height`] on the cell's block.
/// `None` when the cell is not resident or no plate covers the
/// position.
#[must_use]
pub fn scene_height(cells: &[LandCell], mode: HeightMode, x: i32, y: i32, z: i32) -> Option<i32> {
    // `(v + (v < 0 ? 0xFFFF : 0)) >> 16`: the tile, truncating toward zero.
    let tile_x = x / TILE_FX32;
    let tile_z = z / TILE_FX32;
    let cell_x = tile_x >> 5;
    let cell_z = tile_z >> 5;
    let rel_x = x.wrapping_sub((cell_x * 32 + 16) << 16);
    let rel_z = z.wrapping_sub((cell_z * 32 + 16) << 16);
    let cell = cells
        .iter()
        .find(|c| i32::from(c.cell_x) == cell_x && i32::from(c.cell_z) == cell_z)?;
    plate_height(&cell.bdhc, mode, y, rel_x, rel_z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::map_object::FX32_ONE;

    struct Plate {
        corners: [(i32, i32); 2],
        normal: [i32; 3],
        height: i32,
    }

    /// A block of `plates` under `strips` z boundaries, every strip
    /// listing every plate.
    fn block(plates: &[Plate], strips: &[i32]) -> Bdhc {
        let mut b = b"BDHC".to_vec();
        let n = plates.len() as u16;
        for c in [n * 2, n, n, n, strips.len() as u16, n * strips.len() as u16] {
            b.extend_from_slice(&c.to_le_bytes());
        }
        for p in plates {
            for (x, z) in p.corners {
                b.extend_from_slice(&x.to_le_bytes());
                b.extend_from_slice(&z.to_le_bytes());
            }
        }
        for p in plates {
            for v in p.normal {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        for p in plates {
            b.extend_from_slice(&p.height.to_le_bytes());
        }
        for (i, _) in plates.iter().enumerate() {
            let i = i as u16;
            for v in [i * 2, i * 2 + 1, i, i] {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        for (s, z) in strips.iter().enumerate() {
            b.extend_from_slice(&z.to_le_bytes());
            b.extend_from_slice(&n.to_le_bytes());
            b.extend_from_slice(&((s as u16) * n).to_le_bytes());
        }
        for _ in strips {
            for i in 0..n {
                b.extend_from_slice(&i.to_le_bytes());
            }
        }
        Bdhc::parse(&b).expect("well-formed block")
    }

    fn flat(y_units: i32, corners: [(i32, i32); 2]) -> Plate {
        Plate {
            corners,
            normal: [0, FX32_ONE, 0],
            height: -y_units * FX32_ONE,
        }
    }

    const HALF_CELL: i32 = 16 * TILE_FX32;
    const WHOLE_CELL: [(i32, i32); 2] = [(-HALF_CELL, -HALF_CELL), (HALF_CELL, HALF_CELL)];

    #[test]
    fn a_flat_plate_answers_its_height_inside_and_nothing_outside() {
        let b = block(&[flat(16, WHOLE_CELL)], &[HALF_CELL]);
        assert_eq!(plate_height(&b, HeightMode::Nearest, 0, 0, 0), Some(16 * FX32_ONE));
        assert_eq!(
            plate_height(&b, HeightMode::Nearest, 0, HALF_CELL, -HALF_CELL),
            Some(16 * FX32_ONE),
            "corners are inclusive"
        );
        assert_eq!(plate_height(&b, HeightMode::Nearest, 0, HALF_CELL + 1, 0), None);
        assert_eq!(plate_height(&Bdhc::default(), HeightMode::Nearest, 0, 0, 0), None);
    }

    #[test]
    fn a_sloped_plane_is_evaluated_with_the_rounded_fixed_point_products() {
        // Plane x + 2y = 0 (normal (1, 2, 0) in fx32): y = -x / 2.
        let b = block(
            &[Plate {
                corners: WHOLE_CELL,
                normal: [FX32_ONE, 2 * FX32_ONE, 0],
                height: 0,
            }],
            &[HALF_CELL],
        );
        let x = 12 * FX32_ONE + 3;
        let want = fx_div(mul_fx(FX32_ONE, x).wrapping_neg(), 2 * FX32_ONE).unwrap();
        assert_eq!(plate_height(&b, HeightMode::Nearest, 0, x, 0), Some(want));
        assert_eq!(want, -(6 * FX32_ONE + 1));
    }

    #[test]
    fn modes_pick_among_overlapping_plates_as_the_asm_does() {
        let b = block(
            &[flat(16, WHOLE_CELL), flat(40, WHOLE_CELL), flat(8, WHOLE_CELL)],
            &[HALF_CELL],
        );
        assert_eq!(plate_height(&b, HeightMode::Highest, 0, 0, 0), Some(40 * FX32_ONE));
        assert_eq!(plate_height(&b, HeightMode::Lowest, 0, 0, 0), Some(8 * FX32_ONE));
        assert_eq!(
            plate_height(&b, HeightMode::Nearest, 30 * FX32_ONE, 0, 0),
            Some(40 * FX32_ONE)
        );
        assert_eq!(
            plate_height(&b, HeightMode::Nearest, 12 * FX32_ONE, 0, 0),
            Some(16 * FX32_ONE),
            "equidistant hits keep the first"
        );
    }

    #[test]
    fn the_strip_search_is_the_retail_one() {
        let mut strips = Vec::new();
        for z in [-8, 0, 8, 16] {
            strips.extend_from_slice(&(z * FX32_ONE).to_le_bytes());
            strips.extend_from_slice(&[0; 4]);
        }
        assert_eq!(find_strip(&strips, 0, 0), None);
        assert_eq!(find_strip(&strips, 1, 100), Some(0));
        // The first boundary above z, as the walk of lo/hi/mid lands it.
        assert_eq!(find_strip(&strips, 4, -20 * FX32_ONE), Some(0));
        assert_eq!(find_strip(&strips, 4, 4 * FX32_ONE), Some(2));
        assert_eq!(find_strip(&strips, 4, 12 * FX32_ONE), Some(3));
        assert_eq!(find_strip(&strips, 4, 40 * FX32_ONE), Some(3));
    }

    #[test]
    fn scene_height_addresses_the_cell_and_its_centre() {
        let cell = LandCell {
            cell_x: 21,
            cell_z: 12,
            map_id: 60,
            land_id: 0,
            origin: [0; 3],
            meshes: Vec::new(),
            attrs: [0; crate::field::land::ATTRIBUTE_COUNT],
            bdhc: block(&[flat(16, WHOLE_CELL)], &[HALF_CELL]),
        };
        let cells = [cell];
        // Tile (695, 397) is cell (21, 12).
        let x = 695 * TILE_FX32 + TILE_FX32 / 2;
        let z = 397 * TILE_FX32 + TILE_FX32 / 2;
        assert_eq!(scene_height(&cells, HeightMode::Nearest, x, 0, z), Some(16 * FX32_ONE));
        assert_eq!(
            scene_height(&cells, HeightMode::Nearest, x + 32 * TILE_FX32, 0, z),
            None,
            "a tile in a cell that is not resident has no height"
        );
    }
}
