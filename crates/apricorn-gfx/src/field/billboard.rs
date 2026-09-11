//! Map-object billboards — the `mmodel` quads of `a/0/8/1`.
//!
//! Every overworld character is one shared quad model with a
//! per-character NSBTX (`asm/overlay_01_sprite_data.s:436`
//! `ov01_022074A8` maps sprite → mmodel texture and packs the size
//! class in bits 10–15 of its third u16; `sub_021FA248`,
//! `asm/overlay_01_021F944C.s:2015-2045`, resolves that class through
//! `ov01_02207318`). The standard class is `mmdl_m32x32` (NARC member
//! 266): one `pPlane1` node, and an SBC program of
//! `NODEDESC, NODE, BB, POSSCALE, MAT, SHP, POSSCALE, RET` — opcode 7
//! is `NNS_G3D_SBC_BB`, the *full* billboard, which keeps the node's
//! camera-space translation and scale and replaces its rotation with
//! the identity in camera space (local +x is screen right, +y screen
//! up). Its quad is `x ∈ [−16, 16], y ∈ [0, 32], z = 0` with UVs
//! `(0, 32)` at the bottom-left and `(32, 0)` at the top-right (both
//! read from the ROM's display list): the model is anchored at the
//! character's **feet** and stands 32 units tall, which the field
//! presets scale to ≈1 px per unit at the target depth. The 16×16 and
//! 64×64 classes are the same shape at their sizes.
//!
//! The quad is drawn by `BillboardLists_Draw` (`asm/unk_02023694.s:171`
//! → `sub_02023950` → `GF3dRender_DrawModel`) after the map, while the
//! projection carries the field's depth bias
//! ([`crate::field::camera::BILLBOARD_BIAS_UNITS`]) so the feet row
//! beats the ground under it.
//!
//! Which texture, direction and walk frame a character shows is the
//! map-object agent's concern: a [`BillboardView`] takes a texture and
//! the texel rectangle to show, and this module places it.

use super::camera::Camera;
use super::{Projected, Surface, draw_triangle};
use apricorn_core::field::model::Texture;

/// One billboard to draw: a texel rectangle of `texture` on a
/// camera-facing quad anchored at `world_pos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BillboardView<'a> {
    /// The character's decoded NSBTX (or any RGBA8 image).
    pub texture: &'a Texture,
    /// The texel rectangle `(u, v, width, height)` shown — the walk
    /// frame within a strip; the whole texture for a one-frame image.
    pub rect: (u16, u16, u16, u16),
    /// The anchor in fx32 world coordinates: the object's position
    /// vector, the bottom-centre of the quad (its feet).
    pub world_pos: [i32; 3],
    /// The quad's width and height in world units — `(32, 32)` for
    /// `mmdl_m32x32` — which the field cameras scale ≈1:1 to pixels
    /// at the target depth.
    pub size_px: (u16, u16),
}

impl BillboardView<'_> {
    /// The quad's four corners projected through the biased
    /// projection: bottom-left, bottom-right, top-right, top-left —
    /// `None` when the anchor is behind the eye.
    pub(crate) fn corners(&self, camera: &Camera) -> Option<[Projected; 4]> {
        let anchor = camera.to_camera(self.world_pos);
        let half_w = f64::from(self.size_px.0) / 2.0;
        let h = f64::from(self.size_px.1);
        let (u, v, w, rh) = (
            f64::from(self.rect.0),
            f64::from(self.rect.1),
            f64::from(self.rect.2),
            f64::from(self.rect.3),
        );
        // NNSi_G3dFuncSbc_BB: the local axes become the camera's, the
        // translation stays — so the quad lives in camera space at the
        // anchor's depth.
        let corners = [
            ([anchor[0] - half_w, anchor[1], anchor[2]], [u, v + rh]),
            ([anchor[0] + half_w, anchor[1], anchor[2]], [u + w, v + rh]),
            ([anchor[0] + half_w, anchor[1] + h, anchor[2]], [u + w, v]),
            ([anchor[0] - half_w, anchor[1] + h, anchor[2]], [u, v]),
        ];
        let mut out = [Projected::default(); 4];
        for (slot, (point, uv)) in out.iter_mut().zip(corners) {
            let clip = camera.camera_to_clip(point, true);
            *slot = Projected::from_clip(clip, uv, [31.0; 3])?;
        }
        Some(out)
    }

    /// Draws the billboard as two opaque, depth-writing triangles
    /// (the quad's `[0, 1, 2], [0, 2, 3]` triangulation).
    pub(crate) fn draw(&self, camera: &Camera, out: &mut [[u8; 4]], depth: &mut [f64]) {
        let Some([bl, br, tr, tl]) = self.corners(camera) else {
            return;
        };
        let surface = Surface {
            texture: Some(self.texture),
            flags: 0,
            alpha: 31,
        };
        draw_triangle([bl, br, tr], &surface, true, out, depth);
        draw_triangle([bl, tr, tl], &surface, true, out, depth);
    }
}
