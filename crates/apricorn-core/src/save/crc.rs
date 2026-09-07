//! CRC-16/CCITT — the save system's checksum (`GF_CalcCRC16`,
//! `src/math_util.c`).
//!
//! `GF_CalcCRC16` is NitroSDK's `MATH_CalcCRC16CCITT` over the game's
//! precomputed table: polynomial `0x1021`, initial value `0xFFFF`, no
//! reflection, no final XOR — the standard CRC-16/CCITT-FALSE. Every
//! integrity check in the save format uses it: each block's trailing
//! u16, each chunk footer's `crc`, and each extra chunk's footer.
//!
//! **Parity is differential:** `apricorn-harness`'s `tests/save_hg.rs`
//! plants a scratch CRC table in the retail image's memory via the
//! pinned `MATH_CRC16InitTable`, points the game's own
//! `sCRC16TablePtr` global at it, and locks this implementation to the
//! original `GF_CalcCRC16` byte-stream by byte-stream.

/// `GF_CalcCRC16` / `MATH_CalcCRC16CCITT` — CRC-16/CCITT-FALSE
/// (poly `0x1021`, init `0xFFFF`, no reflection, no final XOR).
///
/// ```
/// use apricorn_core::save::crc16;
///
/// // The CCITT-FALSE check value.
/// assert_eq!(crc16(b"123456789"), 0x29B1);
/// // Nothing hashed leaves the initial register.
/// assert_eq!(crc16(&[]), 0xFFFF);
/// ```
#[must_use]
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_ccitt_false_check_values() {
        // The canonical check value, plus streams covering the empty
        // input, single bytes, and a long run — all confirmed against
        // the original GF_CalcCRC16 through arm-runner in the harness
        // tests.
        assert_eq!(crc16(b"123456789"), 0x29B1);
        assert_eq!(crc16(&[]), 0xFFFF);
        assert_eq!(crc16(&[0x00]), 0xE1F0);
        assert_eq!(crc16(&[0xFF; 4]), 0x1D0F);
        assert_eq!(crc16(&[0u8; 0x5C]), 0xA2F2);
        assert_eq!(crc16(b" HeartGold"), 0xD0BD);
    }
}
