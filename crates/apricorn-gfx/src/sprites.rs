//! The intro's tiled OBJ cells: OAM ordering, 1D/2D tile addressing,
//! flips and axis-aligned NANR scale/translation, before BG compositing.

use crate::raster::AssetSource;
use apricorn_core::cache::{AnimFrame, AnimSequence};
use apricorn_core::formats::ncer::CellMapping;
use apricorn_core::frame::EngineFrame;

#[derive(Clone, Copy)]
pub(crate) struct Pixel {
    pub color: [u8; 4],
    pub priority: u8,
    pub semi: bool,
}

fn animation_frame(sequence: &AnimSequence, elapsed: u32) -> Option<&AnimFrame> {
    let total: u32 = sequence.frames.iter().map(|f| u32::from(f.delay)).sum();
    if total == 0 {
        return sequence.frames.first();
    }
    let mut tick = elapsed;
    if tick >= total {
        if sequence.play_mode == 2 {
            let prefix: u32 = sequence.frames[..usize::from(sequence.loop_start)]
                .iter()
                .map(|f| u32::from(f.delay))
                .sum();
            tick = prefix + (tick - total) % (total - prefix).max(1);
        } else {
            return sequence.frames.last();
        }
    }
    for frame in &sequence.frames {
        if tick < u32::from(frame.delay) {
            return Some(frame);
        }
        tick -= u32::from(frame.delay);
    }
    sequence.frames.last()
}

pub(crate) fn rasterize<S: AssetSource + ?Sized>(
    engine: &EngineFrame,
    store: &S,
) -> Vec<Option<Pixel>> {
    let mut pixels = vec![None; 256 * 192];
    const SIZES: [[(i32, i32); 4]; 3] = [
        [(8, 8), (16, 16), (32, 32), (64, 64)],
        [(16, 8), (32, 8), (32, 16), (64, 32)],
        [(8, 16), (8, 32), (16, 32), (32, 64)],
    ];
    for sprite in &engine.sprites {
        let (Some(tiles), Some(palette), Some(cells), Some(animation)) = (
            store.tiles(sprite.tiles),
            store.placed_palette(sprite.palette),
            store.cells(sprite.cells),
            store.animation(sprite.animation),
        ) else {
            continue;
        };
        let Some(sequence) = animation.sequences().get(sprite.sequence) else {
            continue;
        };
        let Some(frame) = animation_frame(sequence, sprite.elapsed) else {
            continue;
        };
        let Some(cell) = cells.cells().get(usize::from(frame.cell)) else {
            continue;
        };
        // Intro animations use translation and axis-aligned scaling.
        // General rotated OBJ and mosaic remain outside this renderer.
        if frame.rotation != 0 {
            continue;
        }
        let (sx, sy) = if sequence.element == 1 {
            (frame.scale_x as i32, frame.scale_y as i32)
        } else {
            (4096, 4096)
        };
        if sx <= 0 || sy <= 0 {
            continue;
        }
        let origin_x = i32::from(sprite.x) + i32::from(frame.x);
        let origin_y = i32::from(sprite.y) + i32::from(frame.y);
        let boundary = match cells.mapping() {
            CellMapping::OneD32K | CellMapping::TwoD => 1,
            CellMapping::OneD64K => 2,
            CellMapping::OneD128K => 4,
            CellMapping::OneD256K => 8,
        };
        let transfer = cells
            .vram_transfer()
            .and_then(|v| v.blocks.get(usize::from(frame.cell)))
            .map_or(0, |b| b.0 as usize / if tiles.is_4bpp() { 32 } else { 64 });
        for oam in &cell.oam {
            if oam.shape >= 3 || oam.mode >= 2 || (!oam.affine && oam.a0 & 0x200 != 0) {
                continue;
            }
            let (w, h) = SIZES[usize::from(oam.shape)][usize::from(oam.size)];
            let ox = i32::from(oam.x);
            let oy = i32::from(oam.y as u8 as i8);
            let left = (origin_x + ox * sx / 4096).max(0);
            let top = (origin_y + oy * sy / 4096).max(0);
            let right = (origin_x + (ox + w) * sx / 4096).min(256);
            let bottom = (origin_y + (oy + h) * sy / 4096).min(192);
            let base =
                usize::from(oam.tile) * boundary / if tiles.is_4bpp() { 1 } else { 2 } + transfer;
            let stride = if cells.mapping() == CellMapping::TwoD {
                if tiles.is_4bpp() { 32 } else { 16 }
            } else {
                w as usize / 8
            };
            for y in top..bottom {
                for x in left..right {
                    let at = (y * 256 + x) as usize;
                    if pixels[at].is_some() {
                        continue;
                    }
                    let mut px = (x - origin_x) * 4096 / sx - ox;
                    let mut py = (y - origin_y) * 4096 / sy - oy;
                    if !(0..w).contains(&px) || !(0..h).contains(&py) {
                        continue;
                    }
                    if !oam.affine && oam.h_flip {
                        px = w - 1 - px;
                    }
                    if !oam.affine && oam.v_flip {
                        py = h - 1 - py;
                    }
                    let (px, py) = (px as usize, py as usize);
                    let tile = base + (py / 8) * stride + px / 8;
                    let Some(&value) = tiles.pixels().get(tile * 64 + (py % 8) * 8 + px % 8) else {
                        continue;
                    };
                    if value == 0 {
                        continue;
                    }
                    let bank = if tiles.is_4bpp() {
                        usize::from(sprite.palette_bank) * 16
                    } else {
                        0
                    };
                    if let Some(&color) = palette.get(bank + usize::from(value)) {
                        pixels[at] = Some(Pixel {
                            color,
                            priority: sprite.priority,
                            semi: oam.mode == 1,
                        });
                    }
                }
            }
        }
    }
    pixels
}
