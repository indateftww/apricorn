//! Static NSBMD subset used by the bedroom: NNS resource dictionaries,
//! material bindings, SBC material/shape commands and packed GX lists.
//! Unsupported node transforms/opcodes fail explicitly, never draw junk.
use crate::{
    formats::{Btx, TexFmt},
    nds::{NdsError, u16le, u32le},
};
use std::sync::Arc;

/// Decoded NNS texture and its bound palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Row-major RGBA8 pixels.
    pub pixels: Vec<[u8; 4]>,
}
/// A GX vertex after static node and placement transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vertex {
    /// World coordinates with twelve fractional bits.
    pub position: [i32; 3],
    /// Texture coordinates with four fractional bits.
    pub uv: [i16; 2],
    /// Vertex color in BGR555.
    pub color: u16,
}
/// A single material/shape draw from the model's SBC program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mesh {
    /// Triangulated GX primitive stream, preserving draw order.
    pub triangles: Vec<[Vertex; 3]>,
    /// Bound texture; absent for solid-color shapes such as shadows.
    pub texture: Option<Arc<Texture>>,
    /// Material TEXIMAGE_PARAM, including repeat/flip bits.
    pub texture_flags: u32,
    /// Polygon alpha, 0–31.
    pub alpha: u8,
}
fn invalid(what: &'static str) -> NdsError {
    NdsError::Invalid { what }
}
fn slice(b: &[u8], p: usize, n: usize) -> Result<&[u8], NdsError> {
    b.get(p..p.checked_add(n).ok_or(invalid("model length"))?)
        .ok_or(NdsError::Truncated {
            what: "model",
            need: p.saturating_add(n),
            got: b.len(),
        })
}
fn dict(b: &[u8], p: usize) -> Result<Vec<(&str, &[u8])>, NdsError> {
    let head = slice(b, p, 8)?;
    let e = p + u16le(head, 6)? as usize;
    let size = u16le(b, e)? as usize;
    let names = e + u16le(b, e + 2)? as usize;
    (0..head[1] as usize)
        .map(|i| {
            let name = slice(b, names + 16 * i, 16)?;
            let name = name.split(|&v| v == 0).next().unwrap();
            Ok((
                std::str::from_utf8(name).map_err(|_| invalid("model name"))?,
                slice(b, e + 4 + i * size, size)?,
            ))
        })
        .collect()
}
pub(crate) fn decode_texture(
    btx: &Btx<'_>,
    name: &str,
    palette: &str,
) -> Result<Texture, NdsError> {
    let tex = btx
        .texture_by_name(name)
        .ok_or(invalid("material texture binding"))?;
    let pal = btx
        .palette_by_name(palette)
        .ok_or(invalid("material palette binding"))?;
    let pixels = (0..tex.width() as usize * tex.height() as usize)
        .map(|i| {
            let byte = tex
                .data()
                .get(match tex.fmt() {
                    TexFmt::Pltt4 => i / 4,
                    TexFmt::Pltt16 => i / 2,
                    _ => i,
                })
                .copied()
                .ok_or(invalid("texture pixels"))?;
            let (index, alpha) = match tex.fmt() {
                TexFmt::Pltt4 => ((byte >> (i % 4 * 2)) & 3, 255),
                TexFmt::Pltt16 => ((byte >> (i % 2 * 4)) & 15, 255),
                TexFmt::Pltt256 => (byte, 255),
                TexFmt::A3i5 => (byte & 31, ((byte >> 5) as u16 * 255 / 7) as u8),
                TexFmt::A5i3 => (byte & 7, ((byte >> 3) as u16 * 255 / 31) as u8),
            };
            let rgb = u16le(pal.data(), index as usize * 2)?;
            let a = if index == 0 && tex.color0_transparent() {
                0
            } else {
                alpha
            };
            Ok([
                expand(rgb & 31),
                expand((rgb >> 5) & 31),
                expand((rgb >> 10) & 31),
                a,
            ])
        })
        .collect::<Result<Vec<_>, NdsError>>()?;
    Ok(Texture {
        width: tex.width() as usize,
        height: tex.height() as usize,
        pixels,
    })
}
fn expand(v: u16) -> u8 {
    ((v << 3) | (v >> 2)) as u8
}

pub(crate) fn parse(b: &[u8], tex: &Btx<'_>, translation: [i32; 3]) -> Result<Vec<Mesh>, NdsError> {
    if slice(b, 0, 4)? != b"BMD0" || u32le(b, 8)? as usize != b.len() {
        return Err(invalid("BMD0 header"));
    }
    let block = u32le(b, 16)? as usize;
    let set = slice(b, block, u32le(b, block + 4)? as usize)?;
    if slice(set, 0, 4)? != b"MDL0" {
        return Err(invalid("MDL0 block"));
    }
    let models = dict(set, 8)?;
    if models.len() != 1 {
        return Err(invalid("single field model"));
    }
    let offset = u32le(models[0].1, 0)? as usize;
    let m = slice(set, offset, u32le(set, offset)? as usize)?;
    let nodes = dict(m, 64)?;
    for (_, d) in &nodes {
        if u16le(m, 64 + u32le(d, 0)? as usize)? & 7 != 7 {
            return Err(invalid("static identity model node"));
        }
    }
    let scale = u32le(m, 28)? as i32;
    let mat = u32le(m, 8)? as usize;
    let shp = u32le(m, 12)? as usize;
    let materials = dict(m, mat + 4)?;
    let shapes = dict(m, shp)?;
    let mut tex_names = vec![None; materials.len()];
    let mut pal_names = vec![None; materials.len()];
    for (offset, names) in [(0, &mut tex_names), (2, &mut pal_names)] {
        for (name, d) in dict(m, mat + u16le(m, mat + offset)? as usize)? {
            let ids = slice(
                m,
                mat + u16le(d, 0)? as usize,
                *d.get(2).ok_or(invalid("material binding count"))? as usize,
            )?;
            for &id in ids {
                *names
                    .get_mut(id as usize)
                    .ok_or(invalid("material binding index"))? = Some(name);
            }
        }
    }
    let mut meshes = Vec::new();
    let mut p = u32le(m, 4)? as usize;
    let mut selected = 0;
    while p < mat {
        let op = m[p];
        p += 1;
        match op {
            0 => {}
            1 => break,
            2 => {
                slice(m, p, 2)?;
                p += 2;
            }
            0x26 => {
                slice(m, p, 4)?;
                p += 4;
            }
            0x06 => {
                slice(m, p, 3)?;
                p += 3;
            }
            0x0b | 0x2b => {}
            4 | 0x24 | 0x44 => {
                selected = *slice(m, p, 1)?.first().unwrap() as usize;
                p += 1;
            }
            5 => {
                let id = slice(m, p, 1)?[0] as usize;
                p += 1;
                let (_, d) = materials.get(selected).ok_or(invalid("SBC material"))?;
                let material = mat + u32le(d, 0)? as usize;
                let (_, d) = shapes.get(id).ok_or(invalid("SBC shape"))?;
                let shape = shp + u32le(d, 0)? as usize;
                let dl = slice(
                    m,
                    shape + u32le(m, shape + 8)? as usize,
                    u32le(m, shape + 12)? as usize,
                )?;
                let texture = match (tex_names[selected], pal_names[selected]) {
                    (Some(t), Some(p)) => Some(Arc::new(decode_texture(tex, t, p)?)),
                    (None, None) => None,
                    _ => return Err(invalid("incomplete material binding")),
                };
                meshes.push(Mesh {
                    triangles: display_list(
                        dl,
                        scale,
                        translation,
                        u16le(m, material + 4)? & 32767,
                    )?,
                    texture,
                    texture_flags: u32le(m, material + 20)?,
                    alpha: ((u32le(m, material + 12)? >> 16) & 31) as u8,
                });
            }
            _ => return Err(invalid("unsupported field SBC opcode")),
        }
    }
    Ok(meshes)
}
fn sign10(v: u32) -> i32 {
    ((v << 22) as i32) >> 22
}
fn display_list(
    b: &[u8],
    scale: i32,
    translation: [i32; 3],
    initial: u16,
) -> Result<Vec<[Vertex; 3]>, NdsError> {
    let mut p = 0;
    let mut position = [0i32; 3];
    let mut uv = [0; 2];
    let mut color = initial;
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut primitive = None;
    while p < b.len() {
        let ops = slice(b, p, 4)?.to_vec();
        p += 4;
        for op in ops {
            let n = match op {
                0 | 0x41 => 0,
                0x23 => 2,
                0x20..=0x22 | 0x24..=0x2b | 0x40 => 1,
                _ => return Err(invalid("unsupported field GX command")),
            };
            let args = slice(b, p, n * 4)?;
            p += n * 4;
            let a = if n > 0 { u32le(args, 0)? } else { 0 };
            match op {
                0 => {}
                0x20 => color = a as u16 & 32767,
                0x21 => {}
                0x22 => uv = [a as i16, (a >> 16) as i16],
                0x23 => {
                    position = [
                        a as i16 as i32,
                        (a >> 16) as i16 as i32,
                        u32le(args, 4)? as i16 as i32,
                    ];
                }
                0x24 => position = [sign10(a) << 6, sign10(a >> 10) << 6, sign10(a >> 20) << 6],
                0x25 => {
                    position[0] = a as i16 as i32;
                    position[1] = (a >> 16) as i16 as i32;
                }
                0x26 => {
                    position[0] = a as i16 as i32;
                    position[2] = (a >> 16) as i16 as i32;
                }
                0x27 => {
                    position[1] = a as i16 as i32;
                    position[2] = (a >> 16) as i16 as i32;
                }
                0x28 => {
                    for i in 0..3 {
                        position[i] += sign10(a >> (i * 10));
                    }
                }
                0x29..=0x2b => {}
                0x40 => {
                    primitive = Some(a & 3);
                    vertices.clear();
                }
                0x41 => primitive = None,
                _ => unreachable!(),
            }
            if (0x23..=0x28).contains(&op) {
                if primitive.is_none() {
                    return Err(invalid("vertex outside primitive"));
                }
                let mut world = translation;
                for i in 0..3 {
                    world[i] += ((position[i] as i64 * scale as i64) >> 12) as i32;
                }
                vertices.push(Vertex {
                    position: world,
                    uv,
                    color,
                });
                let n = vertices.len();
                let ids = match primitive.unwrap() {
                    0 if n % 3 == 0 => vec![[n - 3, n - 2, n - 1]],
                    1 if n % 4 == 0 => vec![[n - 4, n - 3, n - 2], [n - 4, n - 2, n - 1]],
                    2 if n >= 3 => {
                        if n % 2 == 1 {
                            vec![[n - 3, n - 2, n - 1]]
                        } else {
                            vec![[n - 2, n - 3, n - 1]]
                        }
                    }
                    3 if n >= 4 && n % 2 == 0 => vec![[n - 4, n - 3, n - 1], [n - 4, n - 1, n - 2]],
                    _ => vec![],
                };
                for ids in ids {
                    triangles.push(ids.map(|i| vertices[i]));
                }
            }
        }
    }
    Ok(triangles)
}
