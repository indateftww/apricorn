//! Loading a retail ROM and calling its pinned functions.
//!
//! The shared path behind the `arm-runner` bin, the ROM-gated tests,
//! and (in Phase 2 step 7) the differential probes: parse the ROM,
//! decompress the ARM9 image, **verify every committed pin against
//! it**, load it at its `ram_address`, and hand back a [`Cpu`] ready
//! to `prepare_call` a pinned function. Pin verification happens on
//! every load, by design — calling drifted code quietly is the exact
//! failure mode the harness exists to prevent.

use super::exec::Cpu;
use super::mem::Memory;
use crate::pins::{self, PinTable};
use apricorn_core::nds::NdsRom;
use std::fmt;

/// A failure while loading a retail ARM9 image for arm-runner.
#[derive(Debug)]
pub enum RetailError {
    /// The ROM container failed to parse or decompress.
    Rom(apricorn_core::nds::NdsError),
    /// A committed pin failed to verify against the loaded image —
    /// either the image is not the pinned retail dump, or the
    /// interpreter is being pointed at drifted code.
    Pins(String),
}

impl fmt::Display for RetailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RetailError::Rom(e) => write!(f, "ROM: {e}"),
            RetailError::Pins(what) => write!(f, "pins: {what}"),
        }
    }
}

impl std::error::Error for RetailError {}

/// A retail ARM9 image, verified against the committed pin table and
/// loaded into arm-runner memory.
pub struct RetailArm9 {
    cpu: Cpu,
    base: u32,
    table: PinTable,
}

impl RetailArm9 {
    /// Parses `data` as an NDS ROM, decompresses the ARM9 image,
    /// verifies all committed pins against it, and loads it.
    ///
    /// # Errors
    /// [`RetailError::Rom`] on parse/decompress failure;
    /// [`RetailError::Pins`] on any pin mismatch (wrong dump or
    /// drifted addresses).
    pub fn load(data: &[u8]) -> Result<Self, RetailError> {
        let rom = NdsRom::parse(data).map_err(RetailError::Rom)?;
        let image = rom.arm9_image().map_err(RetailError::Rom)?;
        let base = rom.header.arm9.ram_address;
        let table = PinTable::arm9();
        pins::verify_image(&table, &image, base).map_err(|e| RetailError::Pins(e.to_string()))?;
        let mut mem = Memory::new();
        mem.load_arm9_image(&image, base)
            .expect("the image just verified at this base and size");
        Ok(Self {
            cpu: Cpu::new(mem),
            base,
            table,
        })
    }

    /// The CPU, ready for [`Cpu::prepare_call`](super::exec::Cpu::prepare_call).
    /// Globals the callee reads (like `sLCRNG_State`) may be seeded via
    /// [`Cpu::mem_mut`] first — they start zeroed, like a freshly
    /// loaded ROM's `.bss`.
    #[must_use]
    pub fn cpu(&mut self) -> &mut Cpu {
        &mut self.cpu
    }

    /// The address of a pinned function or global by name.
    ///
    /// # Errors
    /// [`RetailError::Pins`] if the name is not in the table.
    pub fn pin_address(&self, name: &str) -> Result<u32, RetailError> {
        self.table
            .get(name)
            .map(|p| p.address)
            .ok_or_else(|| RetailError::Pins(format!("no pin named {name:?}")))
    }

    /// Where the image was loaded (`ram_address`; 0x02000000 retail).
    #[must_use]
    pub fn base(&self) -> u32 {
        self.base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_pin_names_are_errors() {
        // Constructing the full RetailArm9 needs a ROM; the name
        // lookup contract is testable through the pin table alone.
        let table = PinTable::arm9();
        assert!(table.get("LCRandom").is_some());
        assert!(table.get("NotARealPin").is_none());
    }
}
