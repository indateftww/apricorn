//! The field (3D) layer — the picture engine A's BG0 shows while a map
//! is up, rasterized on the CPU from a [`SceneView`].
//!
//! # What the original does
//!
//! The field binds BG0 to the 3D core (`GX_BG0_AS_3D`,
//! `src/field/fieldmap.c:514`), turns the plane on once the map is
//! loaded (`:749`), and draws every frame in `ov01_021E6220`
//! (`:580-613`): reset, push the camera (`Camera_PushLookAtToNNSGlb`),
//! draw the loaded map cells (`MapLoadManager_RenderLoadedMaps`), the
//! props (`ov01_021F3C9C`), then — with the projection's `_32` biased
//! by `8 · cos(−angle.x)` — the field effects and the billboard lists,
//! restore the projection, draw the 3D object tasks, and swap with
//! `GX_SORTMODE_AUTO` and the camera's Z-buffer mode. The hardware
//! then draws opaque polygons first and translucent ones after, depth
//! tested against them without writing depth, and clears to alpha 0
//! (`G3X_SetClearColor(RGB_BLACK, 0, …)`, `src/gf_3d_vramman.c:60`),
//! so uncovered pixels are transparent to the 2D layers below.
//!
//! # This port
//!
//! [`render_view`] draws a [`SceneView`] into a 256×192 RGBA8 buffer
//! whose alpha is the 3D pixel's alpha (0 where nothing was drawn) and
//! a depth buffer of NDC depth: opaque meshes, then billboards through
//! the biased projection, then translucent meshes (polygon alpha < 31)
//! in model order without depth writes. Triangles are projected through
//! [`camera::Camera`], sampled at pixel centres with a top-left fill
//! rule (shared edges are drawn exactly once, so translucent seams do
//! not double-blend), interpolated perspective-correctly, textured
//! with nearest texels honouring the material's repeat/flip bits, and
//! modulated by the interpolated vertex colour. Everything is `f64`
//! with a fixed evaluation order and no transcendental function — the
//! camera's sines and cosines come from the SDK's table — so a scene
//! renders bit-identically on every platform (the documented exception
//! to the crate's integer rule; `docs/gfx.md`, "Field (3D) layer").
//!
//! `raster.rs` composites the result as BG0: `bgs[0].priority` and
//! `enabled` apply, as do the other layers, OBJ, the hardware window
//! (`hidden_rect`), blending (a 3D pixel blends with the second target
//! by its own alpha), backdrop and master brightness.
//!
//! # Seams for the map-data agent's richer `FieldScene`
//!
//! * **Cells** — the loaded 32×32-tile map cells are world-space
//!   [`Mesh`]es: parse each land model with its matrix-cell origin
//!   (`x · 512, 0, z · 512` fx32 per cell, as `FieldScene::bedroom`
//!   centres its one cell) and pass all of them as `SceneView::meshes`.
//! * **Props** — `MapPropArcData { model, translation, rotation,
//!   scale }` become world meshes too: pret builds the model matrix as
//!   `RotX(rotation.x) · RotY(rotation.y) · RotZ(rotation.z)` with each
//!   fx32 component's low 16 bits as a 16-bit angle
//!   (`sub_02020D2C`, `asm/unk_02020B8C.s:218`), scaled by `scale` and
//!   translated (`GF3dRender_DrawModel`, `src/gf_3d_render.c:34`). Bake
//!   that into the vertices ([`camera::sin_idx`]/[`camera::cos_idx`]
//!   give the same table values) and pass the result as meshes.
//! * **Billboards** — every map object is a [`BillboardView`]: its
//!   decoded NSBTX from `a/0/8/1` (`FieldScene::bedroom` decodes one
//!   with `model::decode_texture`), the frame's texel rect, the
//!   object's position vector, and the size class' quad size.
//! * **Camera** — [`CameraPreset`] is the gfx-side shape of the ROM's
//!   `ov01_02206478` row; `CameraPreset::from_table_entry` reads the
//!   0x24 bytes, and `Camera::from_preset(preset, target)` takes the
//!   player's position vector as the target.
//!
//! [`render`] is the shim over today's `FieldScene`: preset 4 at the
//! player's tile with the player texture as the one billboard.

pub mod billboard;
pub mod camera;

pub use billboard::BillboardView;
pub use camera::{Camera, CameraPreset, Projection};

use apricorn_core::field::{
    FieldScene,
    model::{Mesh, Texture, Vertex},
};
use camera::FX32_ONE;

/// The 3D viewport width in pixels.
pub const WIDTH: usize = 256;
/// The 3D viewport height in pixels.
pub const HEIGHT: usize = 192;

/// One frame's worth of field to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneView<'a> {
    /// World-space geometry — map cells and props — in draw order.
    pub meshes: &'a [Mesh],
    /// Map-object billboards, drawn after the meshes with the field's
    /// depth bias.
    pub billboards: Vec<BillboardView<'a>>,
    /// The resolved camera.
    pub camera: Camera,
}

/// The player's position vector for a tile: the tile centre, in fx32
/// (`PlayerAvatar_GetPositionVector`; one tile is 16 world units).
#[must_use]
pub fn tile_position(tile: [i32; 2]) -> [i32; 3] {
    [(tile[0] * 16 + 8) * FX32_ONE, 0, (tile[1] * 16 + 8) * FX32_ONE]
}

/// The shim: today's [`FieldScene`] as a view — camera preset 4 (the
/// indoor orthographic one) targeting the player's tile, the room's
/// meshes, and the player texture as a 32×32 billboard at the tile.
#[must_use]
pub fn scene_view(field: &FieldScene) -> SceneView<'_> {
    let target = tile_position(field.position);
    let player = &field.player;
    let size = (
        u16::try_from(player.width).unwrap_or(u16::MAX),
        u16::try_from(player.height).unwrap_or(u16::MAX),
    );
    SceneView {
        meshes: &field.meshes,
        billboards: vec![BillboardView {
            texture: player,
            rect: (0, 0, size.0, size.1),
            world_pos: target,
            size_px: size,
        }],
        camera: Camera::from_preset(&CameraPreset::INDOOR, target),
    }
}

/// Renders a [`FieldScene`] through [`scene_view`] into `out` (RGBA8,
/// alpha = coverage) and `depth` (NDC depth, `+∞` where clear).
///
/// # Panics
/// Panics unless both buffers hold exactly 256×192 entries.
pub fn render(field: &FieldScene, out: &mut [[u8; 4]], depth: &mut [f64]) {
    render_view(&scene_view(field), out, depth);
}

/// Renders a view into `out` (RGBA8; alpha 0 where no polygon covered
/// the pixel, the polygon's alpha otherwise) and `depth` (NDC depth,
/// `+∞` where clear).
///
/// # Panics
/// Panics unless both buffers hold exactly 256×192 entries.
pub fn render_view(view: &SceneView<'_>, out: &mut [[u8; 4]], depth: &mut [f64]) {
    assert_eq!(out.len(), WIDTH * HEIGHT, "a 256×192 colour buffer");
    assert_eq!(depth.len(), WIDTH * HEIGHT, "a 256×192 depth buffer");
    // The 3D clear: black, alpha 0, depth at the far limit.
    out.fill([0, 0, 0, 0]);
    depth.fill(f64::INFINITY);
    // Opaque geometry first, with depth writes.
    for mesh in view.meshes.iter().filter(|mesh| mesh.alpha == 31) {
        draw_mesh(mesh, &view.camera, true, out, depth);
    }
    // Billboards through the biased projection (fieldmap.c:590-613).
    for billboard in &view.billboards {
        billboard.draw(&view.camera, out, depth);
    }
    // Translucent polygons last, depth tested but not written — the
    // hardware's translucent pass with the depth-update bit clear.
    for mesh in view.meshes.iter().filter(|mesh| mesh.alpha < 31) {
        draw_mesh(mesh, &view.camera, false, out, depth);
    }
}

/// Draws one mesh's triangles through the plain projection.
fn draw_mesh(mesh: &Mesh, camera: &Camera, write_depth: bool, out: &mut [[u8; 4]], depth: &mut [f64]) {
    let surface = Surface {
        texture: mesh.texture.as_deref(),
        flags: mesh.texture_flags,
        alpha: mesh.alpha,
    };
    for triangle in &mesh.triangles {
        let mut projected = [Projected::default(); 3];
        let mut visible = true;
        for (slot, vertex) in projected.iter_mut().zip(triangle) {
            match project_vertex(camera, vertex) {
                Some(p) => *slot = p,
                None => {
                    visible = false;
                    break;
                }
            }
        }
        // No near-plane clipping: a triangle with a vertex behind the
        // eye is dropped whole (the field's cameras never look at one).
        if visible {
            draw_triangle(projected, &surface, write_depth, out, depth);
        }
    }
}

/// A vertex through the view and projection: screen position, NDC
/// depth, and the attributes pre-divided by `w` for perspective-correct
/// interpolation.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Projected {
    /// Screen x in pixels (0..256 across the viewport).
    x: f64,
    /// Screen y in pixels (0..192, row 0 at the top).
    y: f64,
    /// NDC depth, smaller is nearer.
    z: f64,
    /// `1 / w`.
    inv_w: f64,
    /// Texel coordinates over `w`.
    uv: [f64; 2],
    /// Vertex colour channels (0–31) over `w`, in R, G, B order.
    color: [f64; 3],
}

impl Projected {
    /// From clip coordinates with texel `uv` and 5-bit `color`;
    /// `None` behind the eye.
    pub(crate) fn from_clip(clip: [f64; 4], uv: [f64; 2], color: [f64; 3]) -> Option<Self> {
        let [x, y, z] = camera::to_screen(clip)?;
        let inv_w = 1.0 / clip[3];
        Some(Self {
            x,
            y,
            z,
            inv_w,
            uv: [uv[0] * inv_w, uv[1] * inv_w],
            color: [color[0] * inv_w, color[1] * inv_w, color[2] * inv_w],
        })
    }
}

/// Projects a mesh vertex: fx32 position, 4-fractional-bit UVs, BGR555
/// colour (r bits 0–4, g 5–9, b 10–14).
fn project_vertex(camera: &Camera, vertex: &Vertex) -> Option<Projected> {
    let clip = camera.to_clip(vertex.position);
    let uv = [f64::from(vertex.uv[0]) / 16.0, f64::from(vertex.uv[1]) / 16.0];
    let color = [
        f64::from(vertex.color & 31),
        f64::from((vertex.color >> 5) & 31),
        f64::from((vertex.color >> 10) & 31),
    ];
    Projected::from_clip(clip, uv, color)
}

/// The material a triangle samples.
pub(crate) struct Surface<'a> {
    /// The bound texture; `None` draws the vertex colour alone.
    pub texture: Option<&'a Texture>,
    /// `TEXIMAGE_PARAM`: bit 16/17 repeat s/t, bit 18/19 flip s/t.
    pub flags: u32,
    /// Polygon alpha, 0–31.
    pub alpha: u8,
}

/// `(x - a.x)(b.y - a.y) - (y - a.y)(b.x - a.x)`: positive on one side
/// of the directed edge `a → b`.
fn edge(a: &Projected, b: &Projected, x: f64, y: f64) -> f64 {
    (x - a.x) * (b.y - a.y) - (y - a.y) * (b.x - a.x)
}

/// The top-left fill rule's tie-break for the directed edge `a → b` of
/// a positively-oriented triangle (screen y grows downward): a pixel
/// centre exactly on the edge belongs to it only for the top edge
/// (horizontal, running right to left) and left edges (running down).
fn owns_edge(a: &Projected, b: &Projected) -> bool {
    (b.y == a.y && b.x < a.x) || b.y > a.y
}

/// Rasterizes one triangle: pixel-centre sampling with the top-left
/// rule, depth test `z ≤ stored + ε` (equal depth passes, as the
/// bedroom's coplanar decals need), perspective-correct UV and colour,
/// nearest texel, colour modulation, then the polygon alpha.
pub(crate) fn draw_triangle(
    mut p: [Projected; 3],
    surface: &Surface<'_>,
    write_depth: bool,
    out: &mut [[u8; 4]],
    depth: &mut [f64],
) {
    let mut area = edge(&p[0], &p[1], p[2].x, p[2].y);
    if area == 0.0 || !area.is_finite() {
        return;
    }
    if area < 0.0 {
        p.swap(1, 2);
        area = -area;
    }
    let min_x = p.iter().map(|v| v.x).fold(f64::INFINITY, f64::min).floor().max(0.0) as usize;
    let max_x = p
        .iter()
        .map(|v| v.x)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .clamp(0.0, WIDTH as f64) as usize;
    let min_y = p.iter().map(|v| v.y).fold(f64::INFINITY, f64::min).floor().max(0.0) as usize;
    let max_y = p
        .iter()
        .map(|v| v.y)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .clamp(0.0, HEIGHT as f64) as usize;
    let owns = [
        owns_edge(&p[1], &p[2]),
        owns_edge(&p[2], &p[0]),
        owns_edge(&p[0], &p[1]),
    ];
    for y in min_y..max_y {
        for x in min_x..max_x {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let w = [
                edge(&p[1], &p[2], cx, cy),
                edge(&p[2], &p[0], cx, cy),
                edge(&p[0], &p[1], cx, cy),
            ];
            let inside = (0..3).all(|i| w[i] > 0.0 || (w[i] == 0.0 && owns[i]));
            if !inside {
                continue;
            }
            let l = [w[0] / area, w[1] / area, w[2] / area];
            let z = l[0] * p[0].z + l[1] * p[1].z + l[2] * p[2].z;
            let index = y * WIDTH + x;
            if z > depth[index] + 1e-5 {
                continue;
            }
            let inv_w = l[0] * p[0].inv_w + l[1] * p[1].inv_w + l[2] * p[2].inv_w;
            let uv = [0, 1].map(|axis| {
                (l[0] * p[0].uv[axis] + l[1] * p[1].uv[axis] + l[2] * p[2].uv[axis]) / inv_w
            });
            let mut rgba = sample(surface, uv);
            if rgba[3] == 0 {
                continue;
            }
            for axis in 0..3 {
                let color =
                    (l[0] * p[0].color[axis] + l[1] * p[1].color[axis] + l[2] * p[2].color[axis])
                        / inv_w;
                rgba[axis] = (f64::from(rgba[axis]) * (color + 1.0) / 32.0).round() as u8;
            }
            rgba[3] = (u16::from(rgba[3]) * u16::from(surface.alpha) / 31) as u8;
            if rgba[3] == 0 {
                continue;
            }
            blend(&mut out[index], rgba);
            if write_depth {
                depth[index] = z;
            }
        }
    }
}

/// One texel axis: clamp, or repeat (with mirror on odd tiles when
/// flipping) — `TEXIMAGE_PARAM`'s per-axis modes.
fn coord(v: f64, size: usize, repeat: bool, flip: bool) -> usize {
    let v = v.floor() as i64;
    let size = size as i64;
    if !repeat {
        return v.clamp(0, size - 1) as usize;
    }
    let n = v.rem_euclid(size);
    if flip && v.div_euclid(size) & 1 != 0 {
        (size - 1 - n) as usize
    } else {
        n as usize
    }
}

/// The nearest texel at texel coordinates `uv`, or opaque white for an
/// untextured surface.
fn sample(surface: &Surface<'_>, uv: [f64; 2]) -> [u8; 4] {
    let Some(Texture {
        width,
        height,
        pixels,
    }) = surface.texture
    else {
        return [255; 4];
    };
    if *width == 0 || *height == 0 {
        return [0; 4];
    }
    let x = coord(
        uv[0],
        *width,
        surface.flags & (1 << 16) != 0,
        surface.flags & (1 << 18) != 0,
    );
    let y = coord(
        uv[1],
        *height,
        surface.flags & (1 << 17) != 0,
        surface.flags & (1 << 19) != 0,
    );
    pixels.get(y * width + x).copied().unwrap_or([0; 4])
}

/// Writes a pixel over the buffer: over the clear colour (alpha 0) the
/// pixel lands as is; otherwise the colour blends by the source alpha
/// and the alpha keeps the larger of the two — the 3D core's rule for
/// translucent pixels over an empty and over a drawn destination.
fn blend(dst: &mut [u8; 4], src: [u8; 4]) {
    if dst[3] == 0 {
        *dst = src;
        return;
    }
    let a = u32::from(src[3]);
    for i in 0..3 {
        dst[i] = ((u32::from(src[i]) * a + u32::from(dst[i]) * (255 - a)) / 255) as u8;
    }
    dst[3] = dst[3].max(src[3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn vertex(position: [i32; 3], uv: [i16; 2], color: u16) -> Vertex {
        Vertex {
            position,
            uv,
            color,
        }
    }

    /// A ground quad around `centre` (fx32), `half` units to each
    /// side, in one mesh.
    fn ground(centre: [i32; 3], half: i32, texture: Option<Arc<Texture>>, alpha: u8) -> Mesh {
        let h = half * FX32_ONE;
        let (cx, cz) = (centre[0], centre[2]);
        let corner = |dx: i32, dz: i32, u: i16, v: i16| {
            vertex([cx + dx, centre[1], cz + dz], [u * 16, v * 16], 0x7FFF)
        };
        let a = corner(-h, -h, 0, 0);
        let b = corner(h, -h, 1, 0);
        let c = corner(h, h, 1, 1);
        let d = corner(-h, h, 0, 1);
        Mesh {
            triangles: vec![[a, b, c], [a, c, d]],
            texture,
            texture_flags: 0,
            alpha,
        }
    }

    fn solid(color: [u8; 4]) -> Arc<Texture> {
        Arc::new(Texture {
            width: 1,
            height: 1,
            pixels: vec![color],
        })
    }

    fn buffers() -> (Vec<[u8; 4]>, Vec<f64>) {
        (vec![[0; 4]; WIDTH * HEIGHT], vec![0.0; WIDTH * HEIGHT])
    }

    #[test]
    fn clear_pixels_are_transparent_and_covered_ones_opaque() {
        let target = tile_position([0, 0]);
        let meshes = [ground(target, 8, Some(solid([200, 100, 50, 255])), 31)];
        let view = SceneView {
            meshes: &meshes,
            billboards: Vec::new(),
            camera: Camera::from_preset(&CameraPreset::INDOOR, target),
        };
        let (mut out, mut depth) = buffers();
        render_view(&view, &mut out, &mut depth);
        // The 16-unit tile is centred on the screen and ≈16 px wide.
        assert_eq!(out[96 * WIDTH + 128], [200, 100, 50, 255]);
        assert_eq!(out[0], [0, 0, 0, 0], "uncovered corner stays clear");
        assert!(depth[96 * WIDTH + 128].is_finite());
        assert!(depth[0].is_infinite());
        let covered = out.iter().filter(|p| p[3] != 0).count();
        // 16 wide × (16 · sin(pitch) ≈ 12) tall, give or take an edge.
        assert!((150..=230).contains(&covered), "covered {covered}");
    }

    #[test]
    fn shared_edges_draw_translucent_quads_once() {
        // A translucent quad over the clear colour lands with its own
        // alpha and no double-blended diagonal.
        let target = tile_position([0, 0]);
        let meshes = [ground(target, 40, Some(solid([255, 255, 255, 255])), 15)];
        let view = SceneView {
            meshes: &meshes,
            billboards: Vec::new(),
            camera: Camera::from_preset(&CameraPreset::INDOOR, target),
        };
        let (mut out, mut depth) = buffers();
        render_view(&view, &mut out, &mut depth);
        let alpha = (255u16 * 15 / 31) as u8;
        for y in 90..102 {
            for x in 120..136 {
                assert_eq!(out[y * WIDTH + x], [255, 255, 255, alpha], "({x}, {y})");
            }
        }
        assert!(depth[96 * WIDTH + 128].is_infinite(), "translucent writes no depth");
    }

    #[test]
    fn billboard_stands_on_its_anchor_and_wins_the_feet_row() {
        let target = tile_position([0, 0]);
        let floor = ground(target, 100, Some(solid([10, 20, 30, 255])), 31);
        let sprite = Texture {
            width: 32,
            height: 32,
            pixels: (0..32 * 32)
                .map(|i| if i / 32 < 16 { [255, 0, 0, 255] } else { [0, 255, 0, 255] })
                .collect(),
        };
        let meshes = [floor];
        let view = SceneView {
            meshes: &meshes,
            billboards: vec![BillboardView {
                texture: &sprite,
                rect: (0, 0, 32, 32),
                world_pos: target,
                size_px: (32, 32),
            }],
            camera: Camera::from_preset(&CameraPreset::INDOOR, target),
        };
        let (mut out, mut depth) = buffers();
        render_view(&view, &mut out, &mut depth);
        // The anchor projects to (128, 96): the quad spans x 112..144
        // and rises 32 px from y = 96 — bottom half green, top half red.
        assert_eq!(out[95 * WIDTH + 128][..3], [0, 255, 0]);
        assert_eq!(out[70 * WIDTH + 128][..3], [255, 0, 0]);
        assert_eq!(out[97 * WIDTH + 128][..3], [10, 20, 30], "floor below the feet");
        assert_eq!(out[80 * WIDTH + 100][..3], [10, 20, 30], "floor beside the quad");
        assert_eq!(out[80 * WIDTH + 150][..3], [10, 20, 30]);
        // The feet row is nearer than the floor there: the bias.
        let floor_depth = depth[97 * WIDTH + 128];
        assert!(depth[95 * WIDTH + 128] < floor_depth);
    }

    #[test]
    fn texture_repeat_and_flip_follow_teximage_param() {
        assert_eq!(coord(5.0, 4, false, false), 3, "clamp");
        assert_eq!(coord(-1.0, 4, false, false), 0);
        assert_eq!(coord(5.0, 4, true, false), 1, "repeat");
        assert_eq!(coord(5.0, 4, true, true), 2, "flip mirrors odd tiles");
        assert_eq!(coord(-1.0, 4, true, true), 0);
    }
}
