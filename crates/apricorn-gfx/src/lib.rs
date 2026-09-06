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
//! source — no floating point, no time, no state — so the same frame
//! hashes the same pixels in the dump CLI, the golden tests, and the
//! harness, forever.

#![deny(missing_docs)]

pub mod raster;

pub use raster::{AssetSource, ScreenBuffer, render};
