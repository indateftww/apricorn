//! The ROM container tying header, filesystem, and overlays together.

use super::header::{crc16_arc, Header};
use super::nitrofs::NitroFs;
use super::overlay;
use super::{slice, NdsError};

/// A parsed NDS ROM image.
///
/// Borrows the raw image; the caller keeps the bytes (file contents,
/// mmap, whatever) and hands the slice in.
#[derive(Debug)]
pub struct NdsRom<'a> {
    data: &'a [u8],
    /// The parsed cartridge header.
    pub header: Header,
    fs: NitroFs,
    overlays: Vec<overlay::Overlay>,
}

impl<'a> NdsRom<'a> {
    /// Parses a complete ROM image.
    ///
    /// # Errors
    /// Returns a [`NdsError`] if the header, FNT, FAT, overlay table, or
    /// any FAT entry is truncated or inconsistent.
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        let header = Header::parse(data)?;

        let fnt = slice(data, header.fnt.offset, header.fnt.size, "FNT")?;
        let fat = slice(data, header.fat.offset, header.fat.size, "FAT")?;
        let fs = NitroFs::parse(fnt, fat)?;

        for &(start, end) in fs.fat() {
            if start > end {
                return Err(NdsError::Invalid { what: "FAT entry has start > end" });
            }
            if end as usize > data.len() {
                return Err(NdsError::Truncated {
                    what: "FAT entry",
                    need: end as usize,
                    got: data.len(),
                });
            }
        }

        let overlay_data =
            slice(data, header.arm9_overlay.offset, header.arm9_overlay.size, "ARM9 overlay table")?;
        let overlays = overlay::parse_all(overlay_data)?;

        Ok(Self { data, header, fs, overlays })
    }

    /// The Nitro filesystem parsed from this ROM.
    #[must_use]
    pub fn nitrofs(&self) -> &NitroFs {
        &self.fs
    }

    /// The ARM9 overlay table, in overlay-number order.
    #[must_use]
    pub fn overlays(&self) -> &[overlay::Overlay] {
        &self.overlays
    }

    /// The bytes of the file at FAT id `fat_id` (overlays included — they
    /// occupy the first FAT ids).
    ///
    /// # Errors
    /// Returns a [`NdsError`] if the id is out of range.
    pub fn file(&self, fat_id: u32) -> Result<&'a [u8], NdsError> {
        let &(start, end) = self.fs.fat().get(fat_id as usize).ok_or(NdsError::Invalid {
            what: "FAT id out of range",
        })?;
        Ok(&self.data[start as usize..end as usize])
    }

    /// The bytes of the NitroFS file at `path` (e.g. `data/UTF16.dat`).
    ///
    /// # Errors
    /// Returns a [`NdsError`] if no file has that path.
    pub fn file_by_path(&self, path: &str) -> Result<&'a [u8], NdsError> {
        let fat_id = self.fs.fat_id_by_path(path).ok_or(NdsError::Invalid {
            what: "no such NitroFS file",
        })?;
        self.file(fat_id)
    }

    /// Whether the header CRC field matches the header bytes.
    #[must_use]
    pub fn header_crc_ok(&self) -> bool {
        crc16_arc(&self.data[0x00..0x15E]) == self.header.header_crc
    }

    /// Whether the logo CRC field matches the Nintendo logo bytes.
    #[must_use]
    pub fn logo_crc_ok(&self) -> bool {
        crc16_arc(&self.data[0xC0..0x15C]) == self.header.logo_crc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a small synthetic ROM in memory:
    ///
    /// ```text
    /// root:      boot.bin, data/
    /// data:      a.bin, sub/
    /// sub:       c.bin
    /// ```
    ///
    /// with the payload bytes `BOOT!`, `AAA`, `CCC`.
    fn build_synthetic_rom() -> Vec<u8> {
        const HEADER: usize = Header::SIZE;
        const FNT: usize = HEADER;
        const FAT: usize = FNT + 64;
        const FILES: usize = FAT + 24;
        let boot = FILES as u32;
        let a = boot + 5;
        let c = a + 3;
        let total = (c + 3) as usize;

        let mut rom = vec![0u8; total];

        // Header: title / codes / regions.
        rom[0x00..0x08].copy_from_slice(b"APRICORN");
        rom[0x0C..0x10].copy_from_slice(b"TEST");
        rom[0x10..0x12].copy_from_slice(b"01");
        rom[0x84..0x88].copy_from_slice(&0x4000u32.to_le_bytes());
        rom[0x80..0x84].copy_from_slice(&(total as u32).to_le_bytes());

        fn put32(rom: &mut [u8], off: usize, v: u32) {
            rom[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        put32(&mut rom, 0x40, FNT as u32);
        put32(&mut rom, 0x44, 64);
        put32(&mut rom, 0x48, FAT as u32);
        put32(&mut rom, 0x4C, 24);

        // FNT: 3 directory records (root, data=f001, sub=f002), then the
        // name subtables: root at +24, data at +41, sub at +54.
        let fnt = &mut rom[FNT..FNT + 64];
        let rec = |fnt: &mut [u8], i: usize, entry_start: u32, top: u16, parent: u16| {
            fnt[8 * i..8 * i + 4].copy_from_slice(&entry_start.to_le_bytes());
            fnt[8 * i + 4..8 * i + 6].copy_from_slice(&top.to_le_bytes());
            fnt[8 * i + 6..8 * i + 8].copy_from_slice(&parent.to_le_bytes());
        };
        rec(fnt, 0, 24, 0, 3); // root: 3 directories
        rec(fnt, 1, 41, 1, 0xF000); // data: first file id 1, parent root
        rec(fnt, 2, 54, 2, 0xF001); // sub: first file id 2, parent data

        fnt[24] = 0x08;
        fnt[25..33].copy_from_slice(b"boot.bin");
        fnt[33] = 0x84; // directory, name length 4
        fnt[34..38].copy_from_slice(b"data");
        fnt[38..40].copy_from_slice(&0xF001u16.to_le_bytes());
        fnt[40] = 0x00;

        fnt[41] = 0x05;
        fnt[42..47].copy_from_slice(b"a.bin");
        fnt[47] = 0x83;
        fnt[48..51].copy_from_slice(b"sub");
        fnt[51..53].copy_from_slice(&0xF002u16.to_le_bytes());
        fnt[53] = 0x00;

        fnt[54] = 0x05;
        fnt[55..60].copy_from_slice(b"c.bin");
        fnt[60] = 0x00;

        // FAT: boot.bin = id 0, a.bin = id 1, c.bin = id 2.
        rom[FAT..FAT + 4].copy_from_slice(&boot.to_le_bytes());
        rom[FAT + 4..FAT + 8].copy_from_slice(&a.to_le_bytes());
        rom[FAT + 8..FAT + 12].copy_from_slice(&a.to_le_bytes());
        rom[FAT + 12..FAT + 16].copy_from_slice(&c.to_le_bytes());
        rom[FAT + 16..FAT + 20].copy_from_slice(&c.to_le_bytes());
        rom[FAT + 20..FAT + 24].copy_from_slice(&(c + 3).to_le_bytes());

        // Payloads.
        rom[boot as usize..boot as usize + 5].copy_from_slice(b"BOOT!");
        rom[a as usize..a as usize + 3].copy_from_slice(b"AAA");
        rom[c as usize..c as usize + 3].copy_from_slice(b"CCC");

        // CRCs last: the logo CRC first, because the header CRC covers it.
        let logo_crc = crc16_arc(&rom[0xC0..0x15C]);
        rom[0x15C..0x15E].copy_from_slice(&logo_crc.to_le_bytes());
        let header_crc = crc16_arc(&rom[0x00..0x15E]);
        rom[0x15E..0x160].copy_from_slice(&header_crc.to_le_bytes());

        rom
    }

    #[test]
    fn parses_synthetic_rom() {
        let rom = build_synthetic_rom();
        let nds = NdsRom::parse(&rom).expect("synthetic ROM must parse");

        assert_eq!(nds.header.title, "APRICORN");
        assert_eq!(nds.header.game_code_str(), "TEST");
        assert!(nds.header_crc_ok());
        assert!(nds.logo_crc_ok());

        let files: Vec<(String, u32)> =
            nds.nitrofs().files().iter().map(|f| (f.path.clone(), f.fat_id)).collect();
        assert_eq!(
            files,
            vec![
                ("boot.bin".into(), 0),
                ("data/a.bin".into(), 1),
                ("data/sub/c.bin".into(), 2),
            ]
        );

        assert_eq!(nds.nitrofs().dirs().len(), 3);
        assert_eq!(nds.nitrofs().dirs()[0].path, "");
        assert_eq!(nds.nitrofs().dirs()[1].path, "data");
        assert_eq!(nds.nitrofs().dirs()[1].parent, 0xF000);
        assert_eq!(nds.nitrofs().dirs()[2].path, "data/sub");
        assert_eq!(nds.nitrofs().dirs()[2].parent, 0xF001);

        assert_eq!(nds.nitrofs().fat().len(), 3);
        assert_eq!(nds.file(0).unwrap(), b"BOOT!");
        assert_eq!(nds.file(1).unwrap(), b"AAA");
        assert_eq!(nds.file_by_path("data/sub/c.bin").unwrap(), b"CCC");
        assert!(nds.file_by_path("nope.bin").is_err());
        assert!(nds.overlays().is_empty());
    }
}