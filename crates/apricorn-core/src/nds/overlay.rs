//! The ARM9 overlay table.

use super::{u32le, NdsError};

/// An entry of the ARM9 overlay table (0x20 bytes each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overlay {
    /// Overlay number (0-based).
    pub id: u32,
    /// RAM address the overlay loads to.
    pub ram_address: u32,
    /// Uncompressed size; bit 31 set means the overlay is LZ-compressed.
    pub raw_size: u32,
    /// Size of the overlay's BSS section.
    pub bss_size: u32,
    /// Static-initializer start address (0 if none).
    pub sinit_start: u32,
    /// Static-initializer end address (0 if none).
    pub sinit_end: u32,
    /// FAT id of the overlay's file within the ROM.
    pub fat_id: u32,
    /// Size of the compressed overlay, or 0 when uncompressed.
    pub compressed_size: u32,
}

impl Overlay {
    /// Whether the overlay is LZ77-compressed in ROM.
    #[must_use]
    pub fn is_compressed(&self) -> bool {
        self.raw_size & 0x8000_0000 != 0
    }
}

/// Parses every entry of an overlay table region.
///
/// # Errors
/// Returns a [`NdsError`] if the region size is not a multiple of 0x20 or
/// the table is truncated.
pub(crate) fn parse_all(data: &[u8]) -> Result<Vec<Overlay>, NdsError> {
    const ENTRY: usize = 0x20;
    if !data.len().is_multiple_of(ENTRY) {
        return Err(NdsError::Invalid { what: "overlay table size is not a multiple of 0x20" });
    }
    let mut overlays = Vec::with_capacity(data.len() / ENTRY);
    for i in 0..data.len() / ENTRY {
        let o = i * ENTRY;
        overlays.push(Overlay {
            id: u32le(data, o)?,
            ram_address: u32le(data, o + 4)?,
            raw_size: u32le(data, o + 8)?,
            bss_size: u32le(data, o + 12)?,
            sinit_start: u32le(data, o + 16)?,
            sinit_end: u32le(data, o + 20)?,
            fat_id: u32le(data, o + 24)?,
            compressed_size: u32le(data, o + 28)?,
        });
    }
    Ok(overlays)
}