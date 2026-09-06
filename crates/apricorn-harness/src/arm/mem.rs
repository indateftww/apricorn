//! arm-runner memory: a flat address decode into owned buffers.
//!
//! Three regions, nothing else — every other address is a hard
//! [`RunnerError`], which is exactly what a leaf function from a
//! retail ROM should never touch:
//!
//! ```text
//! ITCM      32 KiB  @ 0x01000000
//! DTCM      16 KiB  @ 0x00FF8000
//! main RAM   4 MiB  @ 0x02000000   (the ARM9 image loads here)
//! ```
//!
//! The ARM9 image (`NdsRom::arm9_image`) is copied to main RAM at its
//! header `ram_address`; everything the image does not fill stays
//! zero — which is exactly the `.bss` semantics the loader gives the
//! real machine, and why the harness can call `sMTRNG_State` code
//! without a separate zero-fill step.

use super::RunnerError;

/// Main RAM base (also the retail ARM9 image's `ram_address`).
pub const MAIN_BASE: u32 = 0x0200_0000;
/// Main RAM size: 4 MiB.
pub const MAIN_SIZE: usize = 4 * 1024 * 1024;
/// Instruction TCM base.
pub const ITCM_BASE: u32 = 0x0100_0000;
/// Instruction TCM size: 32 KiB.
pub const ITCM_SIZE: usize = 32 * 1024;
/// Data TCM base.
pub const DTCM_BASE: u32 = 0x00FF_8000;
/// Data TCM size: 16 KiB.
pub const DTCM_SIZE: usize = 16 * 1024;
/// Where `call` points the stack pointer: high main RAM, far above the
/// image and its globals, growing down.
pub const SCRATCH_STACK_TOP: u32 = 0x0230_0000;

/// The interpreter's flat memory: three owned buffers behind one
/// address decode.
#[derive(Debug, Clone)]
pub struct Memory {
    itcm: Vec<u8>,
    dtcm: Vec<u8>,
    main: Vec<u8>,
}

impl Memory {
    /// All-zero memory: 32 KiB ITCM, 16 KiB DTCM, 4 MiB main RAM.
    #[must_use]
    pub fn new() -> Self {
        Self {
            itcm: vec![0; ITCM_SIZE],
            dtcm: vec![0; DTCM_SIZE],
            main: vec![0; MAIN_SIZE],
        }
    }

    /// Copies the decompressed ARM9 image to `base` (main RAM on
    /// retail: [`MAIN_BASE`]). Bytes past the image stay zero.
    ///
    /// # Errors
    /// Returns [`RunnerError::UnmappedWrite`] if the image does not
    /// fit in main RAM at `base`.
    pub fn load_arm9_image(&mut self, image: &[u8], base: u32) -> Result<(), RunnerError> {
        let end = base as usize + image.len();
        if base < MAIN_BASE || end > MAIN_BASE as usize + MAIN_SIZE {
            return Err(RunnerError::UnmappedWrite { addr: base });
        }
        self.main[base as usize - MAIN_BASE as usize..end - MAIN_BASE as usize]
            .copy_from_slice(image);
        Ok(())
    }

    /// The mapped window containing `addr`, as `(buffer, offset)`.
    fn resolve(&self, addr: u32) -> Result<(&[u8], usize), RunnerError> {
        const fn within(addr: u32, base: u32, size: usize) -> bool {
            addr >= base && (addr as usize) < base as usize + size
        }
        if within(addr, MAIN_BASE, MAIN_SIZE) {
            Ok((&self.main, (addr - MAIN_BASE) as usize))
        } else if within(addr, ITCM_BASE, ITCM_SIZE) {
            Ok((&self.itcm, (addr - ITCM_BASE) as usize))
        } else if within(addr, DTCM_BASE, DTCM_SIZE) {
            Ok((&self.dtcm, (addr - DTCM_BASE) as usize))
        } else {
            Err(RunnerError::UnmappedRead { addr })
        }
    }

    /// The mutable window containing `addr`, as `(buffer, offset)`.
    fn resolve_mut(&mut self, addr: u32) -> Result<(&mut [u8], usize), RunnerError> {
        const fn within(addr: u32, base: u32, size: usize) -> bool {
            addr >= base && (addr as usize) < base as usize + size
        }
        if within(addr, MAIN_BASE, MAIN_SIZE) {
            let off = (addr - MAIN_BASE) as usize;
            Ok((&mut self.main, off))
        } else if within(addr, ITCM_BASE, ITCM_SIZE) {
            let off = (addr - ITCM_BASE) as usize;
            Ok((&mut self.itcm, off))
        } else if within(addr, DTCM_BASE, DTCM_SIZE) {
            let off = (addr - DTCM_BASE) as usize;
            Ok((&mut self.dtcm, off))
        } else {
            Err(RunnerError::UnmappedWrite { addr })
        }
    }

    /// Reads one byte.
    ///
    /// # Errors
    /// Returns [`RunnerError::UnmappedRead`] outside mapped RAM.
    pub fn read8(&self, addr: u32) -> Result<u8, RunnerError> {
        let (buf, off) = self.resolve(addr)?;
        Ok(buf[off])
    }

    /// Reads a little-endian halfword (2-byte aligned).
    ///
    /// # Errors
    /// Returns [`RunnerError::UnmappedRead`] outside mapped RAM or
    /// [`RunnerError::Alignment`] on an odd address.
    pub fn read16(&self, addr: u32) -> Result<u16, RunnerError> {
        if addr & 1 != 0 {
            return Err(RunnerError::Alignment { addr, width: 2 });
        }
        let (buf, off) = self.resolve(addr)?;
        Ok(u16::from_le_bytes([buf[off], buf[off + 1]]))
    }

    /// Reads a little-endian word (4-byte aligned; no rotate-on-misalign
    /// — a misaligned `LDR` from leaf code is a bug worth failing on).
    ///
    /// # Errors
    /// Returns [`RunnerError::UnmappedRead`] outside mapped RAM or
    /// [`RunnerError::Alignment`] on a misaligned address.
    pub fn read32(&self, addr: u32) -> Result<u32, RunnerError> {
        if addr & 3 != 0 {
            return Err(RunnerError::Alignment { addr, width: 4 });
        }
        let (buf, off) = self.resolve(addr)?;
        Ok(u32::from_le_bytes([
            buf[off],
            buf[off + 1],
            buf[off + 2],
            buf[off + 3],
        ]))
    }

    /// Writes one byte.
    ///
    /// # Errors
    /// Returns [`RunnerError::UnmappedWrite`] outside mapped RAM.
    pub fn write8(&mut self, addr: u32, value: u8) -> Result<(), RunnerError> {
        let (buf, off) = self.resolve_mut(addr)?;
        buf[off] = value;
        Ok(())
    }

    /// Writes a little-endian halfword (2-byte aligned).
    ///
    /// # Errors
    /// Returns [`RunnerError::UnmappedWrite`] outside mapped RAM or
    /// [`RunnerError::Alignment`] on an odd address.
    pub fn write16(&mut self, addr: u32, value: u16) -> Result<(), RunnerError> {
        if addr & 1 != 0 {
            return Err(RunnerError::Alignment { addr, width: 2 });
        }
        let (buf, off) = self.resolve_mut(addr)?;
        buf[off..off + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Writes a little-endian word (4-byte aligned).
    ///
    /// # Errors
    /// Returns [`RunnerError::UnmappedWrite`] outside mapped RAM or
    /// [`RunnerError::Alignment`] on a misaligned address.
    pub fn write32(&mut self, addr: u32, value: u32) -> Result<(), RunnerError> {
        if addr & 3 != 0 {
            return Err(RunnerError::Alignment { addr, width: 4 });
        }
        let (buf, off) = self.resolve_mut(addr)?;
        buf[off..off + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// The bytes of `len` at `addr` (for hashing watched regions).
    ///
    /// # Errors
    /// Returns [`RunnerError::UnmappedRead`] if the block crosses out
    /// of mapped RAM.
    pub fn read_block(&self, addr: u32, len: u32) -> Result<&[u8], RunnerError> {
        let (buf, off) = self.resolve(addr)?;
        let end = off
            .checked_add(len as usize)
            .ok_or(RunnerError::UnmappedRead {
                addr: addr.wrapping_add(len),
            })?;
        buf.get(off..end).ok_or(RunnerError::UnmappedRead {
            addr: addr.wrapping_add(len),
        })
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_three_regions() {
        let mut mem = Memory::new();
        mem.write8(ITCM_BASE, 0x11).expect("itcm w8");
        mem.write16(DTCM_BASE, 0x2233).expect("dtcm w16");
        mem.write32(MAIN_BASE, 0x4455_6677).expect("main w32");
        assert_eq!(mem.read8(ITCM_BASE).unwrap(), 0x11);
        assert_eq!(mem.read16(DTCM_BASE).unwrap(), 0x2233);
        assert_eq!(mem.read32(MAIN_BASE).unwrap(), 0x4455_6677);

        // The top of each region is mapped; one past it is not.
        let top = MAIN_BASE + MAIN_SIZE as u32 - 4;
        mem.write32(top, 0xDEAD_BEEF).expect("top word");
        assert_eq!(mem.read32(top).unwrap(), 0xDEAD_BEEF);
        assert_eq!(
            mem.read32(top + 4),
            Err(RunnerError::UnmappedRead { addr: top + 4 })
        );
    }

    #[test]
    fn rejects_unmapped_and_unaligned() {
        let mut mem = Memory::new();
        // Cartridge ROM space and ARM7 RAM are unmapped.
        assert_eq!(
            mem.read32(0x0800_0000),
            Err(RunnerError::UnmappedRead { addr: 0x0800_0000 })
        );
        assert_eq!(
            mem.write8(0x0400_0000, 1),
            Err(RunnerError::UnmappedWrite { addr: 0x0400_0000 })
        );
        // Misaligned accesses fail loudly rather than rotating.
        assert_eq!(
            mem.read32(MAIN_BASE + 2),
            Err(RunnerError::Alignment {
                addr: MAIN_BASE + 2,
                width: 4
            })
        );
        assert_eq!(
            mem.read16(MAIN_BASE | 1),
            Err(RunnerError::Alignment {
                addr: MAIN_BASE | 1,
                width: 2
            })
        );
    }

    #[test]
    fn loads_arm9_image_and_zero_fills_rest() {
        let mut mem = Memory::new();
        let image = [0xAAu8; 0x100];
        mem.load_arm9_image(&image, MAIN_BASE).expect("loads");
        assert_eq!(mem.read8(MAIN_BASE).unwrap(), 0xAA);
        // .bss past the image stays zero (the loader's semantics).
        assert_eq!(
            mem.read32(MAIN_BASE + image.len() as u32 + 0x1000).unwrap(),
            0
        );
        // Off-main-RAM loads are rejected.
        assert_eq!(
            mem.load_arm9_image(&image, 0x0300_0000),
            Err(RunnerError::UnmappedWrite { addr: 0x0300_0000 })
        );
        assert_eq!(
            mem.load_arm9_image(&[0u8; 8], 0x0200_0000 - 4),
            Err(RunnerError::UnmappedWrite { addr: 0x01FF_FFFC })
        );
        // An image larger than main RAM cannot fit.
        let huge = vec![0u8; MAIN_SIZE + 4];
        assert!(mem.load_arm9_image(&huge, MAIN_BASE).is_err());
    }

    #[test]
    fn read_block_stays_inside_one_region() {
        let mut mem = Memory::new();
        mem.write32(MAIN_BASE, 0x0BAD_C0DE).expect("w");
        assert_eq!(
            mem.read_block(MAIN_BASE, 4).unwrap(),
            &[0xDE, 0xC0, 0xAD, 0x0B]
        );
        // Runs off the end of main RAM: unmapped.
        assert_eq!(
            mem.read_block(MAIN_BASE + MAIN_SIZE as u32 - 2, 4),
            Err(RunnerError::UnmappedRead {
                addr: MAIN_BASE + MAIN_SIZE as u32 + 2
            })
        );
    }
}
