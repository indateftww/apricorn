//! The 0x4000-byte NDS cartridge header.

use super::{NdsError, u16le, u32le};

/// A region of the ROM described by an (offset, size) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// Absolute offset of the region within the ROM image.
    pub offset: u32,
    /// Size of the region in bytes.
    pub size: u32,
}

/// An ARM binary (ARM9 or ARM7) as described by the cartridge header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmBinary {
    /// Offset of the binary within the ROM image.
    pub rom_offset: u32,
    /// Entry point address.
    pub entry_address: u32,
    /// RAM address the binary is loaded to.
    pub ram_address: u32,
    /// Size of the binary in bytes.
    pub size: u32,
}

/// Parsed fields of the NDS cartridge header.
///
/// Only the fields the engine consumes are retained; the full 0x4000-byte
/// header stays in the raw image (e.g. for CRC verification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Game title, up to 12 ASCII bytes, trailing NULs stripped.
    pub title: String,
    /// Four-character game code (HeartGold US is `IPKE`).
    pub game_code: [u8; 4],
    /// Two-character maker code (`01` = Nintendo).
    pub maker_code: [u8; 2],
    /// Unit code: 0 = NDS, 2 = NDS+DSi, 3 = DSi.
    pub unit_code: u8,
    /// Device capacity exponent; ROM capacity is `1 << (17 + n)` bits.
    pub device_cap: u8,
    /// The ARM9 binary.
    pub arm9: ArmBinary,
    /// The ARM7 binary.
    pub arm7: ArmBinary,
    /// File Name Table region.
    pub fnt: Region,
    /// File Allocation Table region.
    pub fat: Region,
    /// ARM9 overlay table region.
    pub arm9_overlay: Region,
    /// ARM7 overlay table region.
    pub arm7_overlay: Region,
    /// Offset of the banner (icon + titles) within the ROM image.
    pub banner_offset: u32,
    /// End-of-application offset (nominal ROM size).
    pub application_end_offset: u32,
    /// Size of the cartridge header (always 0x4000 on retail carts).
    pub rom_header_size: u32,
    /// CRC-16 over the Nintendo logo at header offset 0x0C0..0x15C.
    pub logo_crc: u16,
    /// CRC-16 over header bytes 0x000..0x15E (i.e. everything before it).
    pub header_crc: u16,
}

impl Header {
    /// The fixed size of the retail cartridge header.
    pub const SIZE: usize = 0x4000;

    /// Parses the header from the start of a ROM image.
    ///
    /// # Errors
    /// Returns [`NdsError::Truncated`] if `data` is shorter than
    /// [`Header::SIZE`].
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        if data.len() < Self::SIZE {
            return Err(NdsError::Truncated {
                what: "cartridge header",
                need: Self::SIZE,
                got: data.len(),
            });
        }

        let title = data[0x00..0x0C]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();

        Ok(Self {
            title,
            game_code: [data[0x0C], data[0x0D], data[0x0E], data[0x0F]],
            maker_code: [data[0x10], data[0x11]],
            unit_code: data[0x12],
            device_cap: data[0x14],
            arm9: ArmBinary {
                rom_offset: u32le(data, 0x20)?,
                entry_address: u32le(data, 0x24)?,
                ram_address: u32le(data, 0x28)?,
                size: u32le(data, 0x2C)?,
            },
            arm7: ArmBinary {
                rom_offset: u32le(data, 0x30)?,
                entry_address: u32le(data, 0x34)?,
                ram_address: u32le(data, 0x38)?,
                size: u32le(data, 0x3C)?,
            },
            fnt: Region {
                offset: u32le(data, 0x40)?,
                size: u32le(data, 0x44)?,
            },
            fat: Region {
                offset: u32le(data, 0x48)?,
                size: u32le(data, 0x4C)?,
            },
            arm9_overlay: Region {
                offset: u32le(data, 0x50)?,
                size: u32le(data, 0x54)?,
            },
            arm7_overlay: Region {
                offset: u32le(data, 0x58)?,
                size: u32le(data, 0x5C)?,
            },
            banner_offset: u32le(data, 0x68)?,
            application_end_offset: u32le(data, 0x80)?,
            rom_header_size: u32le(data, 0x84)?,
            logo_crc: u16le(data, 0x15C)?,
            header_crc: u16le(data, 0x15E)?,
        })
    }

    /// The game code as a string, if it is printable ASCII.
    #[must_use]
    pub fn game_code_str(&self) -> String {
        self.game_code
            .iter()
            .filter(|b| b.is_ascii_graphic())
            .map(|&b| b as char)
            .collect()
    }
}

/// CRC-16 with reflected polynomial 0xA001 (normal form 0x8005), initial
/// value 0xFFFF, and no final inversion.
///
/// This is the checksum used for the cartridge header's logo CRC and header
/// CRC fields (ndstool's `CalcCrc16`).
#[must_use]
pub fn crc16_arc(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xA001 & mask);
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CRC-16/MODBUS check value for the ASCII digits 1-9, pinning the
    /// polynomial/init/mirror choices above.
    #[test]
    fn crc16_matches_modbus_check_value() {
        assert_eq!(crc16_arc(b"123456789"), 0x4B37);
    }

    #[test]
    fn rejects_short_data() {
        assert!(matches!(
            Header::parse(&[0u8; 0x100]),
            Err(NdsError::Truncated { .. })
        ));
    }
}
