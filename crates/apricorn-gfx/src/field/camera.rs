//! The field camera — pret `src/camera.c` (C) and the field's camera
//! presets (`asm/overlay_01_021EABA8.s:465` `ov01_02206478`, 17 × 0x24
//! bytes, read by `FieldCamera_Create` at `:25-58`).
//!
//! The original keeps everything in the SDK's fixed point: angles are
//! 16-bit units (`0x10000` = 360°) looked up in `FX_SinCosTable_`
//! (`lib/include/nitro/fx/fx_trig.h:20-26`: 4096 `(sin, cos)` pairs of
//! fx16, indexed by `angle >> 4`), distances are fx32 (12 fractional
//! bits), and `FX_Mul`/`FX_Div` round half up as the ARM and the
//! hardware divider do.
//! This port reproduces that arithmetic wherever the original does it
//! in fixed point — the camera position, the ortho extents, the
//! billboard bias — and only then moves to `f64` for the matrices,
//! which the SDK builds in fx32 (`MTX_LookAt`, `MTX_PerspectiveW`,
//! `MTX_OrthoW`, `lib/NitroSDK/asm/fx_mtx4[34].s`). The `f64` stage
//! calls no transcendental function: sines and cosines come from the
//! table (regenerated once with the platform's `sin`/`cos` rounded to
//! fx16 — the SHA-1 test pins every entry to the SDK's), tangents from
//! the table quotient, and the one square root (`MTX_LookAt`'s
//! normalisation) is IEEE-exact on every platform, so the projection
//! of a given point is bit-identical everywhere.
//!
//! Conventions, as the SDK's: row vectors (`v' = v · M`), camera space
//! has +x right, +y up, and the view looking down −z; clip space
//! divides by `w`, and the viewport `G3_ViewPort(0, 0, 255, 191)`
//! (`src/gf_3d_vramman.c:61`) maps NDC x ∈ [−1, 1] to 0..256 and NDC
//! y ∈ [−1, 1] to screen rows 192..0 (row 0 is the top).

use std::sync::OnceLock;

/// One fixed-point unit: fx32 has twelve fractional bits.
pub const FX32_ONE: i32 = 1 << 12;

/// The SDK's `FX32_CONST(1.33333333)` aspect ratio (`camera.c:40`):
/// the C cast truncates `1.33333333 · 4096 = 5461.33` to 5461.
pub const ASPECT_FX32: i32 = 5461;

/// The field's billboard depth bias in world units — `FieldSystem.unk11C`
/// as `ov01_021E6220` (`src/field/fieldmap.c:580-613`) reads it: the
/// projection's `_32` gains `_22 · (8 · cos(−angle.x))` while the
/// field effects and billboard lists draw, so a sprite anchored at its
/// feet wins the depth test against the ground it stands on.
pub const BILLBOARD_BIAS_UNITS: i32 = 8;

/// The SDK's `FX_SinCosTable_`: entry `i` is `(sin, cos)` of
/// `i · 2π / 4096` in fx16 — `lib/NitroSDK/asm/fx_sincos.s`, which is
/// exactly `round(sin · 4096)` for all 4096 entries (verified against
/// the vendored table; the SHA-1 test below pins the regeneration).
fn sin_cos_table() -> &'static [[i16; 2]; 4096] {
    static TABLE: OnceLock<Box<[[i16; 2]; 4096]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = Box::new([[0i16; 2]; 4096]);
        for (i, entry) in table.iter_mut().enumerate() {
            let angle = i as f64 * std::f64::consts::TAU / 4096.0;
            *entry = [
                (angle.sin() * 4096.0).round() as i16,
                (angle.cos() * 4096.0).round() as i16,
            ];
        }
        table
    })
}

/// `FX_SinIdx(angle)`: the sine of a 16-bit angle as fx16/fx32
/// (`fx_trig.h:20`, `FX_SinCosTable_[(idx >> 4) << 1]`).
#[must_use]
pub fn sin_idx(angle: u16) -> i32 {
    i32::from(sin_cos_table()[usize::from(angle >> 4)][0])
}

/// `FX_CosIdx(angle)`: the cosine of a 16-bit angle as fx16/fx32
/// (`fx_trig.h:24`, `FX_SinCosTable_[((idx >> 4) << 1) + 1]`).
#[must_use]
pub fn cos_idx(angle: u16) -> i32 {
    i32::from(sin_cos_table()[usize::from(angle >> 4)][1])
}

/// `FX_Mul(a, b)`: fx32 × fx32 with the SDK's round-half-up,
/// `(a · b + 0x800) >> 12` in a 64-bit intermediate.
#[must_use]
pub fn fx_mul(a: i64, b: i64) -> i64 {
    (a * b + 0x800) >> 12
}

/// `FX_Div(a, b)`: fx32 ÷ fx32 through the hardware divider in its
/// 64/32 mode with 20 guard bits, rounded half up —
/// `lib/NitroSDK/asm/fx_cp.s` `FX_DivAsync` writes `DIVCNT = 1`,
/// `NUMER = a << 32`, `DENOM = b`, and `FX_GetDivResult` returns
/// `((a << 32) / b + 0x80000) >> 20` (`adds r2, r1, #0x80000; adc r1,
/// r0, #0; mov r0, r2, lsr #0x14; orr r0, r0, r1, lsl #12`). The
/// divider truncates the quotient toward zero; the `+ 0x80000`
/// (half of the 20 dropped bits) then rounds it to the nearest fx32,
/// halves up.
///
/// # Panics
/// Panics on a zero divisor, as the divider would flag.
#[must_use]
pub fn fx_div(a: i64, b: i64) -> i64 {
    assert!(b != 0, "FX_Div by zero");
    let quotient = (i128::from(a) << 32) / i128::from(b);
    i64::try_from((quotient + 0x8_0000) >> 20).expect("FX_Div result fits an i64")
}

/// `CameraParam.perspectiveType` (`include/camera.h:14-15`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// `NNS_G3dGlbPerspective` from the half-angle's sine and cosine.
    Perspective,
    /// `NNS_G3dGlbOrtho` with extents `tan(half-angle) · distance`.
    Orthographic,
}

/// One row of the field camera table (`ov01_02206478`, 0x24 bytes):
/// the gfx-side shape of pret's preset. The core crate's own preset
/// type converts into this; [`CameraPreset::from_table_entry`] reads
/// the ROM row directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraPreset {
    /// Target-to-camera distance, fx32 (bytes 0..4).
    pub distance: i32,
    /// Camera angle about x, y, z in 16-bit units (bytes 4..10; a pad
    /// u16 follows). `x` pitches the camera above the target — the
    /// field's presets carry a negative pitch (`0xDC82` ≈ −50°).
    pub angle: [u16; 3],
    /// Perspective or orthographic (byte 12, read as u8 at `:36-39`).
    pub projection: Projection,
    /// `perspectiveAngle`: the vertical field of view's **half** angle
    /// in 16-bit units (bytes 14..16). `Camera_InitInternal` takes its
    /// sine and cosine (`camera.c:38-39`) and `MTX_PerspectiveW` reads
    /// them as `cot(fovy/2) = cos/sin`, gluPerspective's convention.
    pub fovy: u16,
    /// Near clipping plane, fx32 (bytes 16..20).
    pub near: i32,
    /// Far clipping plane, fx32 (bytes 20..24).
    pub far: i32,
    /// `Camera_OffsetLookAtPosAndTarget` offset, fx32 (bytes 24..36):
    /// added to both the camera and the target after creation.
    pub look_at_offset: [i32; 3],
}

impl CameraPreset {
    /// The size of one table row.
    pub const ENTRY_SIZE: usize = 0x24;
    /// The number of presets (`FieldCamera_Create` asserts `< 0x11`).
    pub const COUNT: usize = 17;

    /// Preset 0 — the outdoor default: perspective, distance
    /// `0x29AEC1` (≈ 666.9), pitch `0xDD62`, half-angle `0x5C1`
    /// (≈ 8.1°), near 150, far 1200, no offset.
    pub const OUTDOOR: Self = Self {
        distance: 0x0029_AEC1,
        angle: [0xDD62, 0, 0],
        projection: Projection::Perspective,
        fovy: 0x05C1,
        near: 0x0009_6000,
        far: 0x004B_0000,
        look_at_offset: [0, 0, 0],
    };

    /// Preset 4 — indoor rooms (the player's bedroom): orthographic,
    /// distance `0x61B89B` (≈ 1563.5), pitch `0xDC82`, half-angle
    /// `0x281` (≈ 3.5°), near 150, far 1736, no offset.
    pub const INDOOR: Self = Self {
        distance: 0x0061_B89B,
        angle: [0xDC82, 0, 0],
        projection: Projection::Orthographic,
        fovy: 0x0281,
        near: 0x0009_6000,
        far: 0x006C_7000,
        look_at_offset: [0, 0, 0],
    };

    /// Reads one 0x24-byte row of the ROM's camera table.
    #[must_use]
    pub fn from_table_entry(row: &[u8; Self::ENTRY_SIZE]) -> Self {
        let i32_at = |p: usize| i32::from_le_bytes([row[p], row[p + 1], row[p + 2], row[p + 3]]);
        let u16_at = |p: usize| u16::from_le_bytes([row[p], row[p + 1]]);
        Self {
            distance: i32_at(0),
            angle: [u16_at(4), u16_at(6), u16_at(8)],
            projection: if row[12] == 0 {
                Projection::Perspective
            } else {
                Projection::Orthographic
            },
            fovy: u16_at(14),
            near: i32_at(16),
            far: i32_at(20),
            look_at_offset: [i32_at(24), i32_at(28), i32_at(32)],
        }
    }
}

/// A 4×4 matrix in the SDK's row-vector convention.
pub type Mtx44 = [[f64; 4]; 4];

/// The resolved field camera: the view (`MTX_LookAt`) and projection
/// matrices for one preset and target, plus the billboard variant of
/// the projection with the field's depth bias folded in.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    /// Camera position, world units (the fx32 result over 4096).
    pub position: [f64; 3],
    /// Look-at target, world units.
    pub target: [f64; 3],
    /// The camera (view) matrix: world → camera space.
    pub view: Mtx44,
    /// The projection matrix: camera space → clip space.
    pub projection: Mtx44,
    /// The projection with `_32 += _22 · bias` (`fieldmap.c:592-596`),
    /// used for billboards and field effects only.
    pub billboard_projection: Mtx44,
    /// The bias in world units the billboard projection carries —
    /// `8 · cos(−angle.x)` after the original's fx32 rounding.
    pub billboard_bias: f64,
}

impl Camera {
    /// `Camera_Init_FromTargetDistanceAndAngle` +
    /// `Camera_SetPerspectiveClippingPlane` +
    /// `Camera_OffsetLookAtPosAndTarget`, as `FieldCamera_Create` runs
    /// them (`asm/overlay_01_021EABA8.s:33-58`), for `target` in fx32
    /// world coordinates — the player's position vector (tile centre,
    /// ground height).
    #[must_use]
    pub fn from_preset(preset: &CameraPreset, target: [i32; 3]) -> Self {
        let [ax, ay, _] = preset.angle;
        let distance = i64::from(preset.distance);
        // Camera_CalcLookAtPosFromTargetAndAngle (camera.c:20-26):
        // note the x/z terms cosine the *positive* angle while y sines
        // the negated one — the table is not symmetric under rounding,
        // so both lookups are kept as written.
        let neg_x = ax.wrapping_neg();
        let cam_x = fx_mul(
            fx_mul(i64::from(sin_idx(ay)), distance),
            i64::from(cos_idx(ax)),
        );
        let cam_z = fx_mul(
            fx_mul(i64::from(cos_idx(ay)), distance),
            i64::from(cos_idx(ax)),
        );
        let cam_y = fx_mul(i64::from(sin_idx(neg_x)), distance);
        let offset = preset.look_at_offset.map(i64::from);
        let target_fx = [
            i64::from(target[0]) + offset[0],
            i64::from(target[1]) + offset[1],
            i64::from(target[2]) + offset[2],
        ];
        let position_fx = [
            target_fx[0] + cam_x,
            target_fx[1] + cam_y,
            target_fx[2] + cam_z,
        ];
        let to_units = |v: i64| v as f64 / f64::from(FX32_ONE);
        let position = position_fx.map(to_units);
        let target = target_fx.map(to_units);
        let view = look_at(position, [0.0, 1.0, 0.0], target);

        // Camera_ApplyPerspectiveType (camera.c:266-278).
        let fovy_sin = i64::from(sin_idx(preset.fovy));
        let fovy_cos = i64::from(cos_idx(preset.fovy));
        let near = to_units(i64::from(preset.near));
        let far = to_units(i64::from(preset.far));
        let projection = match preset.projection {
            Projection::Perspective => perspective(
                fovy_sin as f64,
                fovy_cos as f64,
                to_units(i64::from(ASPECT_FX32)),
                near,
                far,
            ),
            Projection::Orthographic => {
                // fx32 end to end, as the C does: y = tan · distance,
                // x = y · aspect; NNS_G3dGlbOrtho(y, -y, -x, x, n, f).
                let y = fx_mul(fx_div(fovy_sin, fovy_cos), distance);
                let x = fx_mul(y, i64::from(ASPECT_FX32));
                let (y, x) = (to_units(y), to_units(x));
                ortho(y, -y, -x, x, near, far)
            }
        };

        // fieldmap.c:590-596: (unk11C << 12) * FX_CosIdx(-angle.x),
        // rounded to fx32, times _22, rounded, added to _32.
        let bias_fx =
            ((i64::from(BILLBOARD_BIAS_UNITS) << 12) * i64::from(cos_idx(neg_x)) + 0x800) >> 12;
        let billboard_bias = to_units(bias_fx);
        let mut billboard_projection = projection;
        billboard_projection[3][2] += projection[2][2] * billboard_bias;

        Self {
            position,
            target,
            view,
            projection,
            billboard_projection,
            billboard_bias,
        }
    }

    /// World (fx32) → camera space, in world units.
    #[must_use]
    pub fn to_camera(&self, world: [i32; 3]) -> [f64; 3] {
        let v = world.map(|c| f64::from(c) / f64::from(FX32_ONE));
        let r = mul_point(&self.view, v);
        [r[0], r[1], r[2]]
    }

    /// Camera space → clip space `[x, y, z, w]`, through the plain or
    /// the billboard-biased projection.
    #[must_use]
    pub fn camera_to_clip(&self, camera: [f64; 3], billboard: bool) -> [f64; 4] {
        let m = if billboard {
            &self.billboard_projection
        } else {
            &self.projection
        };
        mul_point(m, camera)
    }

    /// World (fx32) → clip space through the model's projection.
    #[must_use]
    pub fn to_clip(&self, world: [i32; 3]) -> [f64; 4] {
        self.camera_to_clip(self.to_camera(world), false)
    }

    /// World (fx32) → screen `[x, y, depth]` (pixels, NDC depth), or
    /// `None` behind the eye (`w ≤ 0`).
    #[must_use]
    pub fn project(&self, world: [i32; 3]) -> Option<[f64; 3]> {
        to_screen(self.to_clip(world))
    }
}

/// Clip → screen: the `G3_ViewPort(0, 0, 255, 191)` mapping, x to
/// 0..256 left to right and y to 0..192 top to bottom, plus NDC depth.
#[must_use]
pub fn to_screen(clip: [f64; 4]) -> Option<[f64; 3]> {
    let w = clip[3];
    if w <= 0.0 {
        return None;
    }
    Some([
        128.0 + 128.0 * (clip[0] / w),
        96.0 - 96.0 * (clip[1] / w),
        clip[2] / w,
    ])
}

/// `[x y z 1] · m`.
fn mul_point(m: &Mtx44, p: [f64; 3]) -> [f64; 4] {
    let mut out = [0.0; 4];
    for (j, o) in out.iter_mut().enumerate() {
        *o = p[0] * m[0][j] + p[1] * m[1][j] + p[2] * m[2][j] + m[3][j];
    }
    out
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let len = dot(v, v).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}

/// `MTX_LookAt(camPos, camUp, camTarget)` (`fx_mtx43.s:617`): the
/// gluLookAt camera matrix in row-vector form — rows are the camera's
/// right, up and back axes' world coordinates transposed, the last row
/// the negated dotted position.
#[must_use]
pub fn look_at(position: [f64; 3], up: [f64; 3], target: [f64; 3]) -> Mtx44 {
    let forward = normalize([
        target[0] - position[0],
        target[1] - position[1],
        target[2] - position[2],
    ]);
    let side = normalize(cross(forward, up));
    let up = cross(side, forward);
    [
        [side[0], up[0], -forward[0], 0.0],
        [side[1], up[1], -forward[1], 0.0],
        [side[2], up[2], -forward[2], 0.0],
        [
            -dot(side, position),
            -dot(up, position),
            dot(forward, position),
            1.0,
        ],
    ]
}

/// `MTX_PerspectiveW(fovySin, fovyCos, aspect, n, f, FX32_ONE)`
/// (`fx_mtx44.s:576`): gluPerspective with `cot = fovyCos / fovySin`.
#[must_use]
pub fn perspective(fovy_sin: f64, fovy_cos: f64, aspect: f64, near: f64, far: f64) -> Mtx44 {
    let cot = fovy_cos / fovy_sin;
    let depth = far - near;
    [
        [cot / aspect, 0.0, 0.0, 0.0],
        [0.0, cot, 0.0, 0.0],
        [0.0, 0.0, -(far + near) / depth, -1.0],
        [0.0, 0.0, -2.0 * far * near / depth, 0.0],
    ]
}

/// `MTX_OrthoW(t, b, l, r, n, f, FX32_ONE)` (`fx_mtx44.s:666`): glOrtho.
#[must_use]
pub fn ortho(top: f64, bottom: f64, left: f64, right: f64, near: f64, far: f64) -> Mtx44 {
    let width = right - left;
    let height = top - bottom;
    let depth = far - near;
    [
        [2.0 / width, 0.0, 0.0, 0.0],
        [0.0, 2.0 / height, 0.0, 0.0],
        [0.0, 0.0, -2.0 / depth, 0.0],
        [
            -(right + left) / width,
            -(top + bottom) / height,
            -(far + near) / depth,
            1.0,
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::{Digest, Sha1};

    #[test]
    fn sin_cos_table_matches_the_sdk() {
        // Spot values read off lib/NitroSDK/asm/fx_sincos.s, and the
        // SHA-1 of all 8192 little-endian fx16 values as the file
        // lists them: the regeneration must reproduce the SDK bit for
        // bit, on every platform.
        assert_eq!((sin_idx(0), cos_idx(0)), (0, 0x1000));
        assert_eq!((sin_idx(0x10), cos_idx(0x10)), (6, 0x1000));
        assert_eq!((sin_idx(0x20), cos_idx(0x20)), (13, 0x1000));
        assert_eq!((sin_idx(0x4000), cos_idx(0x4000)), (0x1000, 0));
        assert_eq!((sin_idx(0x8000), cos_idx(0x8000)), (0, -0x1000));
        // The preset angles this module is actually asked for.
        assert_eq!((sin_idx(0xDC82), cos_idx(0xDC82)), (-3134, 2637));
        assert_eq!((sin_idx(0x237E), cos_idx(0x237E)), (3130, 2642));
        assert_eq!((sin_idx(0x5C1), cos_idx(0x5C1)), (576, 4055));
        assert_eq!((sin_idx(0x281), cos_idx(0x281)), (251, 4088));
        let mut hasher = Sha1::new();
        for entry in sin_cos_table().iter() {
            hasher.update(entry[0].to_le_bytes());
            hasher.update(entry[1].to_le_bytes());
        }
        assert_eq!(
            format!("{:x}", hasher.finalize()),
            "18b7e1baae69ac1be13a4edcd7805f2943c6847e"
        );
    }

    #[test]
    fn fx_mul_and_div_follow_the_sdk() {
        assert_eq!(fx_mul(FX32_ONE.into(), FX32_ONE.into()), FX32_ONE.into());
        // Round half up: 0.5 · 0.5 = 0.25 exactly; 1/4096 · 1/2 rounds up.
        assert_eq!(fx_mul(2048, 2048), 1024);
        assert_eq!(fx_mul(1, 2048), 1);
        assert_eq!(fx_div(FX32_ONE.into(), 2048), 2 * i64::from(FX32_ONE));
        // FX_Div rounds half up through 20 guard bits: 251/4088 in fx32
        // is 251.49 LSB, which rounds down to 251.
        assert_eq!(fx_div(251, 4088), 251);
        // Exactly half an LSB (1/8192 in fx32 = 0.5 LSB) rounds up;
        // one part in 8193 falls short of the half and rounds down.
        // Truncation, which the divider alone would give, yields 0 for
        // both.
        assert_eq!(fx_div(1, 8192), 1);
        assert_eq!(fx_div(1, 8193), 0);
        // The half-up rounding is toward +infinity on negatives too:
        // −0.5 LSB → 0, −1.5 LSB → −1 (the divider's quotient is
        // truncated toward zero before the guard bits are rounded).
        assert_eq!(fx_div(-1, 8192), 0);
        assert_eq!(fx_div(-3, 8192), -1);
        assert_eq!(fx_div(-1, 8193), 0);
        // A quotient just above a half rounds up: 3/8191 = 1.5002 LSB.
        assert_eq!(fx_div(3, 8191), 2);
    }

    #[test]
    fn preset_row_parses_the_documented_layout() {
        // Preset 4's row bytes, from ov01_02206478 + 4 · 0x24.
        let mut row = [0u8; 0x24];
        row[..4].copy_from_slice(&0x0061_B89Bu32.to_le_bytes());
        row[4..6].copy_from_slice(&0xDC82u16.to_le_bytes());
        row[12] = 1;
        row[14..16].copy_from_slice(&0x0281u16.to_le_bytes());
        row[16..20].copy_from_slice(&0x0009_6000u32.to_le_bytes());
        row[20..24].copy_from_slice(&0x006C_7000u32.to_le_bytes());
        assert_eq!(CameraPreset::from_table_entry(&row), CameraPreset::INDOOR);
    }

    /// The bedroom camera at the bedroom's player tile (6, 6): the
    /// numbers below are derived by hand from the C and the table.
    #[test]
    fn indoor_ortho_camera_pins_its_derivation() {
        let target = [(6 * 16 + 8) * FX32_ONE, 0, (6 * 16 + 8) * FX32_ONE];
        let cam = Camera::from_preset(&CameraPreset::INDOOR, target);

        // Camera_CalcLookAtPosFromTargetAndAngle: angle.y = 0 so the
        // camera sits straight south (+z) and above: z = FX_Mul(
        // FX_Mul(4096, 0x61B89B), 2637) = FX_Mul(6404251, 2637) =
        // (16888009887 + 2048) >> 12 = 4123049 → 1006.60; y =
        // FX_Mul(3130, 6404251) = (20045305630 + 2048) >> 12 =
        // 4893873 → 1194.79; x = 0.
        assert_eq!(cam.position[0], 104.0);
        assert!((cam.position[1] - 4_893_873.0 / 4096.0).abs() < 1e-9);
        assert!((cam.position[2] - (104.0 + 4_123_049.0 / 4096.0)).abs() < 1e-9);

        // Ortho half extents in fx32: y = FX_Mul(FX_Div(251, 4088),
        // 0x61B89B) = FX_Mul(251, 6404251) = 392448 → 95.8125 units;
        // x = FX_Mul(392448, 5461) = 523232 → 127.7422 units.
        let half_y = 392_448.0 / 4096.0;
        let half_x = 523_232.0 / 4096.0;
        assert!((cam.projection[1][1] - 1.0 / half_y).abs() < 1e-12);
        assert!((cam.projection[0][0] - 1.0 / half_x).abs() < 1e-12);

        // The target sits on the view axis: dead centre.
        let centre = cam.project(target).unwrap();
        assert!((centre[0] - 128.0).abs() < 1e-9);
        assert!((centre[1] - 96.0).abs() < 1e-9);

        // One tile east: 16 units → 16 · 128 / half_x pixels right.
        let east = cam
            .project([target[0] + 16 * FX32_ONE, 0, target[2]])
            .unwrap();
        assert!((east[0] - (128.0 + 16.0 * 128.0 / half_x)).abs() < 1e-9);
        assert!((east[1] - 96.0).abs() < 1e-9);

        // One tile south (+z) on the ground: the camera looks down the
        // pitch, so ground z maps to screen y by sin(pitch) — the
        // view's up axis has z component −sin(pitch) where
        // sin(pitch) = 1194.75 / distance, so the point moves down by
        // 16 · sin(pitch) · 96 / half_y pixels and closer to the
        // camera (smaller NDC depth).
        let south = cam
            .project([target[0], 0, target[2] + 16 * FX32_ONE])
            .unwrap();
        let sin_pitch = (cam.position[1] - cam.target[1])
            / ((cam.position[1] - cam.target[1]).powi(2)
                + (cam.position[2] - cam.target[2]).powi(2))
            .sqrt();
        assert!((south[1] - (96.0 + 16.0 * sin_pitch * 96.0 / half_y)).abs() < 1e-9);
        assert!(south[2] < centre[2]);

        // A unit up (+y) rises by cos(pitch) · 96 / half_y.
        let up = cam.project([target[0], 16 * FX32_ONE, target[2]]).unwrap();
        let cos_pitch = (1.0 - sin_pitch * sin_pitch).sqrt();
        assert!((up[1] - (96.0 - 16.0 * cos_pitch * 96.0 / half_y)).abs() < 1e-9);

        // The billboard bias: FX_Mul(8 << 12, cos_idx(0x237E) = 2642)
        // = 21136 → 5.16 units nearer, i.e. a smaller ortho depth by
        // 2 · bias / (far − near).
        assert_eq!(cam.billboard_bias, 21_136.0 / 4096.0);
        let plain = cam.camera_to_clip([0.0, 0.0, -1000.0], false);
        let biased = cam.camera_to_clip([0.0, 0.0, -1000.0], true);
        let depth = (0x6C_7000 - 0x9_6000) as f64 / 4096.0;
        assert!((plain[2] - biased[2] - 2.0 * cam.billboard_bias / depth).abs() < 1e-12);
    }

    /// The outdoor perspective camera: pins the perspective terms and
    /// the ≈1:1 world-unit-to-pixel scale at the target depth.
    #[test]
    fn outdoor_perspective_camera_pins_its_derivation() {
        let target = [8 * FX32_ONE, 0, 8 * FX32_ONE];
        let cam = Camera::from_preset(&CameraPreset::OUTDOOR, target);
        // cot(half-angle) = 4055 / 576 (the table values for 0x5C1).
        let cot = 4055.0 / 576.0;
        assert!((cam.projection[1][1] - cot).abs() < 1e-12);
        assert!((cam.projection[0][0] - cot / (5461.0 / 4096.0)).abs() < 1e-12);
        assert_eq!(cam.projection[2][3], -1.0);
        assert_eq!(cam.projection[3][3], 0.0);

        // The target is dead centre; its camera-space depth is the
        // length of the fx32 offset the table produced — 666.48, a
        // hair under the preset's 666.92 because the table's
        // (sin, cos) pair for the pitch is not unit length.
        let centre = cam.project(target).unwrap();
        assert!((centre[0] - 128.0).abs() < 1e-9);
        assert!((centre[1] - 96.0).abs() < 1e-9);
        let depth = cam.to_camera(target)[2];
        let offset = [
            cam.position[0] - cam.target[0],
            cam.position[1] - cam.target[1],
            cam.position[2] - cam.target[2],
        ];
        let length = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
        assert!((-depth - length).abs() < 1e-9);
        assert!((length - 666.48).abs() < 0.01);

        // One unit east at the target depth spans cot · 128 / (aspect
        // · distance) ≈ 1.01 pixels — the field's near-1:1 scale.
        let east = cam.project([target[0] + FX32_ONE, 0, target[2]]).unwrap();
        let expected = 128.0 * cam.projection[0][0] / -depth;
        assert!((east[0] - 128.0 - expected).abs() < 1e-9);
        assert!((expected - 1.0).abs() < 0.02);

        // Behind the eye projects to nothing.
        assert!(
            cam.project([target[0], 0, target[2] + 10_000 * FX32_ONE])
                .is_none()
        );
    }
}
