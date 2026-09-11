//! The field map's data layer (Phase 5): map headers, matrices, land
//! data, area data, terrain attributes, map events, script headers, the
//! ov01 tables, and [`FieldScene`], which loads a map the way pret's
//! field system does at map entry — `FieldSystem_CreateMap`
//! (`src/field/fieldmap.c:658`: area data, prop models), `MapMatrix_Load`,
//! `MapLoadManager_InitialLoad`, `Field_InitMapEvents`,
//! `MapScriptHeader_ReadFromNarc`, `FieldCamera_Create` — into plain
//! integer geometry the presenter projects. Nothing here ticks; the
//! script VM, movement and the map-load manager's streaming are later
//! steps (`docs/field-data.md`).
//!
//! Coordinates: a tile is 16 world units (fx32 `<< 12`), a cell 32 tiles;
//! the player stands at the centre of tile `(x, z)`, world
//! `((x * 16 + 8) << 12, y, (z * 16 + 8) << 12)`; cell origins are at
//! each cell's centre (`matrix::cell_origin`) and land geometry is
//! authored around that origin.

pub mod area;
pub mod events;
pub mod land;
pub mod map_header;
pub mod matrix;
pub mod avatar;
pub mod input;
pub mod map_object;
pub mod model;
pub mod ov01;
pub mod script_header;
pub mod terrain;

use std::collections::HashMap;

use crate::{
    assets::{AssetStore, AssetsError, narc_member},
    formats::Btx,
    nds::NdsError,
};
use area::AreaData;
use events::MapEvents;
use land::{ATTRIBUTE_COUNT, LandData, PropPlacement};
use map_header::{MapHeader, MapHeaders};
use matrix::{MapMatrix, NO_LAND};
use model::{Mesh, Placement, Texture};
use ov01::{CameraPreset, CameraPresets, Ov01, SpriteModelTable};
use script_header::InitScripts;
use terrain::TerrainAttributes;

/// Cells loaded on each side of the player's cell by [`FieldScene::load`]
/// — a 3 × 3 window. Retail's map-load manager keeps the player's cell
/// plus the neighbours toward the player's quadrant (2 × 2); the 3 × 3
/// window is its superset for every quadrant, so nothing retail draws
/// or collides against is missing, while a full overworld matrix (799
/// cells, 205 distinct land members, 19.5 MB of source) stays out of
/// memory. [`FieldScene::load_all`] loads every cell.
pub const LOAD_RADIUS: i32 = 1;

/// A rectangle of matrix cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellWindow {
    /// First cell column.
    pub x0: i32,
    /// First cell row.
    pub z0: i32,
    /// Columns.
    pub width: usize,
    /// Rows.
    pub height: usize,
}

impl CellWindow {
    /// The whole matrix.
    #[must_use]
    pub fn all(matrix: &MapMatrix) -> Self {
        Self {
            x0: 0,
            z0: 0,
            width: usize::from(matrix.width),
            height: usize::from(matrix.height),
        }
    }

    /// The cells within `radius` of cell `(cx, cz)`, clamped to the
    /// matrix.
    #[must_use]
    pub fn around(matrix: &MapMatrix, cx: i32, cz: i32, radius: i32) -> Self {
        let x0 = (cx - radius).clamp(0, i32::from(matrix.width) - 1);
        let z0 = (cz - radius).clamp(0, i32::from(matrix.height) - 1);
        let x1 = (cx + radius).clamp(0, i32::from(matrix.width) - 1);
        let z1 = (cz + radius).clamp(0, i32::from(matrix.height) - 1);
        Self {
            x0,
            z0,
            width: (x1 - x0 + 1) as usize,
            height: (z1 - z0 + 1) as usize,
        }
    }

    /// Whether cell `(x, z)` lies in the window.
    #[must_use]
    pub fn contains(&self, x: i32, z: i32) -> bool {
        x >= self.x0
            && z >= self.z0
            && ((x - self.x0) as usize) < self.width
            && ((z - self.z0) as usize) < self.height
    }
}

/// One resident matrix cell: its land geometry in world space (the
/// member's model under [`ov01::LandBase`] at the cell origin, as
/// `MapLoadManager_RenderLoadedMap` draws it) and its terrain attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandCell {
    /// Cell column in the matrix.
    pub cell_x: u8,
    /// Cell row in the matrix.
    pub cell_z: u8,
    /// The matrix's map header id for the cell.
    pub map_id: u16,
    /// The `a/0/6/5` member.
    pub land_id: u16,
    /// World origin (fx32) the cell's model and props are placed at.
    pub origin: [i32; 3],
    /// The cell's model in world space.
    pub meshes: Vec<Mesh>,
    /// Terrain attributes, index `(x % 32) + (z % 32) * 32`.
    pub attrs: [u16; ATTRIBUTE_COUNT],
}

/// One placed prop, transformed as `ov01_021F3A3C`
/// (`src/field/map_prop_manager.c:186`) draws the props of a loaded
/// cell: the archive translation plus the cell origin's x and z (never
/// its y), the function's local identity `MtxFx33`, the archive scale.
/// `MapPropManager_LoadFromNARC` (`:100`) keeps the archive rotation but
/// leaves `overrideRotation` clear, so no draw path reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropInstance {
    /// The model actually drawn (the placement's, or 0 when the area's
    /// building set does not contain it — `MapPropManager_LoadFromNARC`).
    pub model_id: i32,
    /// The cell the placement came from.
    pub cell: [u8; 2],
    /// The archive record.
    pub placement: PropPlacement,
    /// World translation (fx32).
    pub translation: [i32; 3],
    /// The model in world space.
    pub meshes: Vec<Mesh>,
}

/// A loaded map: header, matrix, resident cells and props, camera,
/// player image, terrain, events and init scripts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldScene {
    /// Map-header id (64 for the player's bedroom).
    pub map_id: u16,
    /// The map's internal name (`T20R0202`).
    pub name: String,
    /// The map header.
    pub header: MapHeader,
    /// The map's matrix.
    pub matrix: MapMatrix,
    /// The map's area data.
    pub area: AreaData,
    /// The window of cells that were loaded.
    pub window: CellWindow,
    /// Resident cells (matrix cells without land data are skipped).
    pub cells: Vec<LandCell>,
    /// Props of the resident cells, in cell then archive order.
    pub props: Vec<PropInstance>,
    /// Every cell's and prop's meshes concatenated in draw order — the
    /// pre-Phase-5 presenter interface, kept until the rasterizer moves
    /// to [`Self::cells`] / [`Self::props`] (it duplicates their
    /// triangles; textures are shared).
    pub meshes: Vec<Mesh>,
    /// The camera preset `MapHeader::camera_type` selects.
    pub camera: CameraPreset,
    /// The south-facing player map-object image.
    pub player: Texture,
    /// Position in matrix-wide tile coordinates; one tile is 16 world
    /// units.
    pub position: [i32; 2],
    /// Terrain attributes over [`Self::window`].
    pub terrain: TerrainAttributes,
    /// The map's events.
    pub events: MapEvents,
    /// The map's init scripts.
    pub init_scripts: InitScripts,
}

fn corrupt(what: impl Into<String>, source: NdsError) -> AssetsError {
    AssetsError::Corrupt {
        what: what.into(),
        source,
    }
}

impl FieldScene {
    /// A minimal scene for renderer tests: no map data, only the given
    /// world-space `meshes`, the player billboard image, the player's
    /// tile and a camera preset. Everything else is empty (a 0 × 0
    /// window, no cells, props, events or scripts).
    #[must_use]
    pub fn synthetic(
        map_id: u16,
        meshes: Vec<Mesh>,
        player: Texture,
        position: [i32; 2],
        camera: CameraPreset,
    ) -> Self {
        Self {
            map_id,
            name: String::new(),
            header: MapHeader::default(),
            matrix: MapMatrix::default(),
            area: AreaData::default(),
            window: CellWindow {
                x0: 0,
                z0: 0,
                width: 0,
                height: 0,
            },
            cells: Vec::new(),
            props: Vec::new(),
            meshes,
            camera,
            player,
            position,
            terrain: TerrainAttributes::new(0, 0, 0, 0),
            events: MapEvents::default(),
            init_scripts: InitScripts::default(),
        }
    }

    /// Loads map `map_id` with the player on tile `(x, z)`, resident cells
    /// within [`LOAD_RADIUS`] of the player's cell.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when any member the map references is
    /// missing or malformed.
    pub fn load(
        store: &AssetStore,
        map_id: u16,
        x: i32,
        z: i32,
        gender: u8,
    ) -> Result<Self, AssetsError> {
        Self::load_window(store, map_id, x, z, gender, None)
    }

    /// Loads map `map_id` with every cell of its matrix resident. On the
    /// overworld matrix that is all 799 cells (205 distinct land members,
    /// 19.5 MB of source, several hundred megabytes of placed triangles)
    /// — meant for sub-matrices and tooling, not the runtime.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when any member the map references is
    /// missing or malformed.
    pub fn load_all(
        store: &AssetStore,
        map_id: u16,
        x: i32,
        z: i32,
        gender: u8,
    ) -> Result<Self, AssetsError> {
        Self::load_window(store, map_id, x, z, gender, Some(usize::MAX))
    }

    /// `map_headers.h[64]`, matrix 72, area bank 25, the new game's tile
    /// (6, 6): the player's bedroom.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when a bedroom member is missing or
    /// malformed.
    pub fn bedroom(store: &AssetStore, gender: u8) -> Result<Self, AssetsError> {
        Self::load(store, 64, 6, 6, gender)
    }

    /// The loader behind [`Self::load`] (`radius == None`) and
    /// [`Self::load_all`] (`Some(usize::MAX)`); any other radius loads
    /// that many cells on each side of the player's cell.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when any member the map references is
    /// missing or malformed.
    pub fn load_window(
        store: &AssetStore,
        map_id: u16,
        x: i32,
        z: i32,
        gender: u8,
        radius: Option<usize>,
    ) -> Result<Self, AssetsError> {
        let headers = MapHeaders::load(store)?;
        let header = *headers
            .get(map_id)
            .ok_or_else(|| AssetsError::Missing(format!("map header {map_id}")))?;
        let name = headers.name(map_id).unwrap_or_default().to_owned();
        let matrix = MapMatrix::load(store, header.matrix_id, map_id)?;
        let area = AreaData::load(store, header.area_data_bank)?;
        let (cx, cz) = MapMatrix::cell_of_tile(x, z);
        let window = match radius {
            None => CellWindow::around(&matrix, cx, cz, LOAD_RADIUS),
            Some(usize::MAX) => CellWindow::all(&matrix),
            Some(r) => CellWindow::around(&matrix, cx, cz, r.min(i32::MAX as usize) as i32),
        };

        let map_texture_bytes = store.member(area::MAP_TEXTURE_NARC, usize::from(area.map_texture))?;
        let map_texture = Btx::parse(&map_texture_bytes)
            .map_err(|e| corrupt(format!("{}#{}", area::MAP_TEXTURE_NARC, area.map_texture), e))?;
        let prop_texture_bytes =
            store.member(area::BUILDING_TEXTURE_NARC, usize::from(area.building_set))?;
        let prop_texture = Btx::parse(&prop_texture_bytes).map_err(|e| {
            corrupt(
                format!("{}#{}", area::BUILDING_TEXTURE_NARC, area.building_set),
                e,
            )
        })?;
        let building_ids = area.building_ids(store)?;
        let land_narc = store.narc(land::LAND_NARC)?;
        let prop_narc_path = area.prop_model_narc();
        let prop_narc = store.narc(prop_narc_path)?;

        let ov01 = Ov01::load(store)?;
        let land_base = ov01.land_base()?;
        let mut cells = Vec::new();
        let mut props = Vec::new();
        let mut terrain = TerrainAttributes::new(window.x0, window.z0, window.width, window.height);
        let mut prop_models: HashMap<i32, Vec<Mesh>> = HashMap::new();
        for cz in window.z0..window.z0 + window.height as i32 {
            for cx in window.x0..window.x0 + window.width as i32 {
                let Some(land_id) = matrix.land_id(cx, cz) else {
                    continue;
                };
                if land_id == NO_LAND {
                    continue;
                }
                let bytes = narc_member(&land_narc, land::LAND_NARC, usize::from(land_id))?;
                let what = format!("{}#{land_id}", land::LAND_NARC);
                let land = LandData::parse(&bytes).map_err(|e| corrupt(&what, e))?;
                let origin = matrix.cell_origin(cx, cz).expect("cell inside the matrix");
                let local = model::parse(&land.model, &map_texture)
                    .map_err(|e| corrupt(format!("{what} model"), e))?;
                let meshes = model::place(&local, &land_base.at(origin));
                terrain.insert(cx, cz, &land.attributes);
                for placement in &land.props {
                    let model_id = if building_ids.contains(&(placement.model_id as u16))
                        && placement.model_id >= 0
                    {
                        placement.model_id
                    } else {
                        0
                    };
                    if !prop_models.contains_key(&model_id) {
                        let bytes =
                            narc_member(&prop_narc, prop_narc_path, model_id.max(0) as usize)?;
                        let local = model::parse(&bytes, &prop_texture)
                            .map_err(|e| corrupt(format!("{prop_narc_path}#{model_id}"), e))?;
                        prop_models.insert(model_id, local);
                    }
                    let translation = [
                        origin[0] + placement.translation[0],
                        placement.translation[1],
                        origin[2] + placement.translation[2],
                    ];
                    let placed = Placement {
                        translation,
                        rotation: Placement::IDENTITY.rotation,
                        scale: placement.scale,
                    };
                    props.push(PropInstance {
                        model_id,
                        cell: [cx as u8, cz as u8],
                        placement: *placement,
                        translation,
                        meshes: model::place(&prop_models[&model_id], &placed),
                    });
                }
                cells.push(LandCell {
                    cell_x: cx as u8,
                    cell_z: cz as u8,
                    map_id: matrix.header(cx, cz).unwrap_or(map_id),
                    land_id,
                    origin,
                    meshes,
                    attrs: land.attributes,
                });
            }
        }

        let camera = *CameraPresets::from_overlay(&ov01)?
            .get(header.camera_type)
            .ok_or_else(|| AssetsError::Missing(format!("camera preset {}", header.camera_type)))?;
        let sprite = ov01::player_sprite(gender);
        let mmodel = SpriteModelTable::from_overlay(&ov01)?
            .model_for_sprite(sprite)
            .ok_or_else(|| AssetsError::Missing(format!("sprite {sprite} in the model table")))?;
        let player_bytes = store.member(ov01::MMODEL_NARC, usize::from(mmodel))?;
        let player_what = format!("{}#{mmodel}", ov01::MMODEL_NARC);
        let btx = Btx::parse(&player_bytes).map_err(|e| corrupt(&player_what, e))?;
        let (Some(texture), Some(palette)) = (btx.textures().first(), btx.palettes().first()) else {
            return Err(corrupt(
                &player_what,
                NdsError::Invalid {
                    what: "player texture archive without a texture",
                },
            ));
        };
        let player = model::decode_texture(&btx, texture.name(), palette.name())
            .map_err(|e| corrupt(format!("{player_what} pixels"), e))?;

        let events = MapEvents::load(store, header.events_bank)?;
        let init_scripts = InitScripts::load(store, header.script_header_bank)?;

        let meshes = cells
            .iter()
            .flat_map(|c| c.meshes.iter().cloned())
            .chain(props.iter().flat_map(|p| p.meshes.iter().cloned()))
            .collect();
        Ok(Self {
            map_id,
            name,
            header,
            matrix,
            area,
            window,
            cells,
            props,
            meshes,
            camera,
            player,
            position: [x, z],
            terrain,
            events,
            init_scripts,
        })
    }

    /// The resident cell holding tile `(x, z)`.
    #[must_use]
    pub fn cell_at(&self, x: i32, z: i32) -> Option<&LandCell> {
        let (cx, cz) = MapMatrix::cell_of_tile(x, z);
        self.cells
            .iter()
            .find(|c| i32::from(c.cell_x) == cx && i32::from(c.cell_z) == cz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(w: u8, h: u8) -> MapMatrix {
        let n = usize::from(w) * usize::from(h);
        MapMatrix {
            id: 0,
            width: w,
            height: h,
            name: String::new(),
            has_headers: false,
            has_altitudes: false,
            headers: vec![0; n],
            altitudes: vec![0; n],
            land_ids: vec![0; n],
        }
    }

    #[test]
    fn windows_clamp_to_the_matrix() {
        let m = matrix(47, 17);
        assert_eq!(
            CellWindow::around(&m, 21, 12, 1),
            CellWindow { x0: 20, z0: 11, width: 3, height: 3 }
        );
        assert_eq!(
            CellWindow::around(&m, 0, 16, 1),
            CellWindow { x0: 0, z0: 15, width: 2, height: 2 }
        );
        assert_eq!(CellWindow::all(&m), CellWindow { x0: 0, z0: 0, width: 47, height: 17 });
        let one = matrix(1, 1);
        assert_eq!(CellWindow::around(&one, 0, 0, 1), CellWindow::all(&one));
        assert!(CellWindow::all(&m).contains(46, 16) && !CellWindow::all(&m).contains(47, 0));
    }
}
