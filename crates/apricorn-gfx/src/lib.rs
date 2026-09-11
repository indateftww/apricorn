//! # apricorn-gfx
//!
//! Deterministic CPU rasterizer: [`LogicalFrame`]s to RGBA buffers.
//!
//! The renderer architecture (PLAN.md Phase 3) is *CPU rasterize,
//! wgpu present*: this crate turns one logical frame into two
//! 256×192 RGBA8 screens with plain integer arithmetic, and the
//! presenter (apricorn-desktop, step 8) only uploads and draws them.
//! GPU compositing can come later without changing this contract.
//!
//! Everything here is a pure function of the frame and the asset
//! source — no time, no state, and integer arithmetic throughout the
//! 2D path — so the same frame hashes the same pixels in the dump CLI,
//! the golden tests, and the harness, forever. The one documented
//! exception is the [`field`] (3D) layer, which uses `f64` with a fixed
//! evaluation order and no transcendental call in its render path (the
//! SDK's fixed-point sine table is regenerated once and pinned by
//! SHA-1), and is therefore bit-identical across platforms too.

#![deny(missing_docs)]

pub mod field;
pub mod raster;
mod sprites;

pub use field::{BillboardView, Camera, CameraPreset, SceneView, render_view};
pub use raster::{AssetSource, ScreenBuffer, render};
