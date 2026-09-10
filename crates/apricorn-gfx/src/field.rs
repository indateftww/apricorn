//! Static field projection and depth-buffered, textured triangles.
use apricorn_core::field::{
    FieldScene,
    model::{Mesh, Texture},
};

// FieldCamera_Create camera type 4, ov01_02206478 + 4 * 0x24:
// distance 0x61B89B / 4096, angle 0xDC82, ortho, fovy 0x281.
fn projection(field: &FieldScene, p: [i32; 3]) -> [f64; 3] {
    let pitch = (0x1_0000 - 0xdc82) as f64 * std::f64::consts::TAU / 65536.;
    let half_height =
        (0x281 as f64 * std::f64::consts::TAU / 65536.).tan() * (0x61b89b as f64 / 4096.);
    let k = 96. / half_height;
    let x = p[0] as f64 / 4096. - (field.position[0] * 16 + 8) as f64;
    let y = p[1] as f64 / 4096.;
    let z = p[2] as f64 / 4096. - (field.position[1] * 16 + 8) as f64;
    [
        128. + x * k,
        96. + (z * pitch.sin() - y * pitch.cos()) * k,
        -y * pitch.sin() - z * pitch.cos(),
    ]
}
pub fn render(field: &FieldScene, out: &mut [[u8; 4]]) {
    out.fill([0, 0, 0, 255]);
    let mut depth = vec![f64::INFINITY; out.len()];
    // Opaque geometry first, then transparent shadow materials.
    for transparent in [false, true] {
        for mesh in &field.meshes {
            if (mesh.alpha < 31) != transparent {
                continue;
            }
            for triangle in &mesh.triangles {
                let p = triangle.map(|v| projection(field, v.position));
                let area = edge(p[0], p[1], p[2][0], p[2][1]);
                if area.abs() < 0.00001 {
                    continue;
                }
                let minx = p
                    .iter()
                    .map(|v| v[0])
                    .fold(f64::INFINITY, f64::min)
                    .floor()
                    .max(0.) as usize;
                let maxx = p
                    .iter()
                    .map(|v| v[0])
                    .fold(f64::NEG_INFINITY, f64::max)
                    .ceil()
                    .clamp(0., 256.) as usize;
                let miny = p
                    .iter()
                    .map(|v| v[1])
                    .fold(f64::INFINITY, f64::min)
                    .floor()
                    .max(0.) as usize;
                let maxy = p
                    .iter()
                    .map(|v| v[1])
                    .fold(f64::NEG_INFINITY, f64::max)
                    .ceil()
                    .clamp(0., 192.) as usize;
                for y in miny..maxy {
                    for x in minx..maxx {
                        let a = edge(p[1], p[2], x as f64 + 0.5, y as f64 + 0.5) / area;
                        let b = edge(p[2], p[0], x as f64 + 0.5, y as f64 + 0.5) / area;
                        let c = 1. - a - b;
                        if a < -0.000001 || b < -0.000001 || c < -0.000001 {
                            continue;
                        }
                        let w = [a, b, c];
                        let z = (0..3).map(|i| w[i] * p[i][2]).sum::<f64>();
                        let index = y * 256 + x;
                        if z > depth[index] + 0.00001 {
                            continue;
                        }
                        let uv = [0, 1].map(|axis| {
                            (0..3)
                                .map(|i| w[i] * triangle[i].uv[axis] as f64 / 16.)
                                .sum::<f64>()
                        });
                        let mut rgba = sample(mesh, uv);
                        if rgba[3] == 0 {
                            continue;
                        }
                        for axis in 0..3 {
                            let color = (0..3)
                                .map(|i| w[i] * ((triangle[i].color >> (axis * 5)) & 31) as f64)
                                .sum::<f64>();
                            rgba[axis] = (rgba[axis] as f64 * (color + 1.) / 32.).round() as u8;
                        }
                        rgba[3] = (rgba[3] as u16 * mesh.alpha as u16 / 31) as u8;
                        blend(&mut out[index], rgba);
                        if !transparent {
                            depth[index] = z;
                        }
                    }
                }
            }
        }
    }
    // Map-object billboard. Direction SOUTH uses texture name *.1.
    let p = projection(
        field,
        [
            (field.position[0] * 16 + 8) * 4096,
            0,
            (field.position[1] * 16 + 8) * 4096,
        ],
    );
    for y in 0..field.player.height {
        for x in 0..field.player.width {
            let sx = p[0].round() as i32 + x as i32 - 16;
            let sy = p[1].round() as i32 + y as i32 - 28;
            if !(0..256).contains(&sx) || !(0..192).contains(&sy) {
                continue;
            }
            let i = sy as usize * 256 + sx as usize;
            if p[2] - 8. <= depth[i] {
                blend(&mut out[i], field.player.pixels[y * field.player.width + x]);
            }
        }
    }
}
fn edge(a: [f64; 3], b: [f64; 3], x: f64, y: f64) -> f64 {
    (x - a[0]) * (b[1] - a[1]) - (y - a[1]) * (b[0] - a[0])
}
fn coord(v: f64, size: usize, repeat: bool, flip: bool) -> usize {
    let v = v.floor() as i32;
    if !repeat {
        return v.clamp(0, size as i32 - 1) as usize;
    }
    let n = v.rem_euclid(size as i32) as usize;
    if flip && v.div_euclid(size as i32) & 1 != 0 {
        size - 1 - n
    } else {
        n
    }
}
fn sample(mesh: &Mesh, uv: [f64; 2]) -> [u8; 4] {
    let Some(Texture {
        width,
        height,
        pixels,
    }) = mesh.texture.as_deref()
    else {
        return [255; 4];
    };
    let x = coord(
        uv[0],
        *width,
        mesh.texture_flags & (1 << 16) != 0,
        mesh.texture_flags & (1 << 18) != 0,
    );
    let y = coord(
        uv[1],
        *height,
        mesh.texture_flags & (1 << 17) != 0,
        mesh.texture_flags & (1 << 19) != 0,
    );
    pixels[y * width + x]
}
fn blend(dst: &mut [u8; 4], src: [u8; 4]) {
    let a = src[3] as u32;
    for i in 0..3 {
        dst[i] = ((src[i] as u32 * a + dst[i] as u32 * (255 - a)) / 255) as u8;
    }
}
