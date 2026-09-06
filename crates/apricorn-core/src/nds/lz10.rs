//! LZ77-10 — the forward LZ77 used for compressed NARC members.
//!
//! Where BLZ (see [`super::blz`]) compresses the ARM9 overlays, the
//! LZ77-10 variant compresses *assets*: HeartGold stores selected NARC
//! members — e.g. the copyright-beat NCGR/NSCR members of the intro
//! movie's `a/2/6/2` (`gs_opening`), requested from pret with
//! `isCompressed=TRUE` — in the same format the SDK's
//! `MI_UncompressLZ8`/`MI_UncompressLZ16` decoders accept.
//!
//! A stored stream of `L` bytes:
//!
//! ```text
//! [0]        0x10 magic
//! [1 .. 4]   u24 decompressed size (little-endian)
//! [4 .. L)   payload: groups of [flags byte][up to 8 codes]
//!            flags consumed MSB-first (bit 7 is the first token);
//!            a clear bit = literal (1 byte copied verbatim), a set
//!            bit = match (2 bytes [b1][b2]:
//!            len = (b1 >> 4) + 3  (3..=18),
//!            disp = ((b1 & 0xF) << 8 | b2) + 1  (1..=4096))
//! ```
//!
//! Matches copy from `write - disp` forward, so a match may read bytes
//! it has itself just written (RLE-style runs, `disp == 1`). Decoding
//! stops when the declared image is full — the last match may end mid-
//! copy, and any bytes left in the stream (4-byte alignment padding)
//! are ignored, matching the SDK decoder, which never looks past the
//! image end.

use super::NdsError;

/// The magic byte of an LZ77-10 stream.
const MAGIC: u8 = 0x10;

/// Returns whether `src` carries the LZ77-10 magic.
///
/// This is only the sniff — a `0x10` first byte is common in binary
/// data, so callers treat a decompression failure as "not this format"
/// (the same convention as the MAT sniff in the converter).
#[must_use]
pub fn is_lz10(src: &[u8]) -> bool {
    src.first() == Some(&MAGIC)
}

/// Decompresses an LZ77-10 stream.
///
/// # Errors
/// Returns an [`NdsError`] if the magic is wrong, the payload is
/// exhausted before the declared image is full, or a match reaches
/// before the start of the image — every corruption the SDK decoder
/// would misbehave on is rejected before any out-of-bounds access.
pub fn decompress(src: &[u8]) -> Result<Vec<u8>, NdsError> {
    if src.len() < 4 {
        return Err(NdsError::Truncated {
            what: "LZ77-10 stream (no header)",
            need: 4,
            got: src.len(),
        });
    }
    if src[0] != MAGIC {
        return Err(NdsError::Invalid {
            what: "LZ77-10 magic",
        });
    }
    // The size is a u24 in bytes 1–3 (not a u32 — the header is 4 bytes
    // total, and a maximal-size stream's payload starts at 4).
    let size = (u32::from(src[1]) | u32::from(src[2]) << 8 | u32::from(src[3]) << 16) as usize;
    let mut out = vec![0u8; size];
    let mut read = 4;
    let mut write = 0usize;
    while write < out.len() {
        if read >= src.len() {
            return Err(NdsError::Invalid {
                what: "LZ77-10 payload exhausted before the image was filled",
            });
        }
        let flags = src[read];
        read += 1;
        for bit in (0..8).rev() {
            if write == out.len() {
                break; // the image ends mid-group (the last flags group)
            }
            if flags >> bit & 1 == 0 {
                if read >= src.len() {
                    return Err(NdsError::Invalid {
                        what: "LZ77-10 literal runs past the payload",
                    });
                }
                out[write] = src[read];
                read += 1;
                write += 1;
            } else {
                if read + 1 >= src.len() {
                    return Err(NdsError::Invalid {
                        what: "LZ77-10 match runs past the payload",
                    });
                }
                let b1 = src[read];
                let b2 = src[read + 1];
                read += 2;
                let length = usize::from(b1 >> 4) + 3;
                let disp = (usize::from(b1 & 0xF) << 8 | usize::from(b2)) + 1;
                if disp > write {
                    return Err(NdsError::Invalid {
                        what: "LZ77-10 match reaches before the image start",
                    });
                }
                for _ in 0..length {
                    if write == out.len() {
                        break; // the image ends mid-match
                    }
                    out[write] = out[write - disp];
                    write += 1;
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes `src` into a hex dump for asserting whole images.
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn decodes_literals_only() {
        // size 4, one flags byte of literals, one padding byte after
        // (ignored, as the SDK decoder ignores it).
        let src: &[u8] = &[0x10, 0x04, 0x00, 0x00, 0x00, b'A', b'B', b'C', b'D', 0xFF];
        let out = decompress(src).expect("literals must decode");
        assert_eq!(out, b"ABCD");
    }

    #[test]
    fn decodes_overlapping_run() {
        // size 16: one literal 'X', then one match len 15, disp 1 —
        // the match copies the byte it is itself writing (RLE).
        // Flags 0x40: MSB-first, bit 7 clear (literal), bit 6 set
        // (match).
        let src: &[u8] = &[0x10, 0x10, 0x00, 0x00, 0x40, b'X', 0xC0, 0x00];
        let out = decompress(src).expect("RLE match must decode");
        assert_eq!(hex(&out), "58".repeat(16));
    }

    #[test]
    fn decodes_matches_across_groups() {
        // size 32, image "ABCDEFGH" * 4: 8 literals, then a len-18 and
        // a len-6 match, both disp 8 (the image ends mid-group).
        let src: &[u8] = &[
            0x10, 0x20, 0x00, 0x00, // header: 32 bytes
            0x00, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', // 8 literals
            0xC0, // flags: two matches (bits 7 and 6)
            0xF0, 0x07, // match: len 18, disp 8
            0x30, 0x07, // match: len 6, disp 8
        ];
        let out = decompress(src).expect("matches must decode");
        assert_eq!(out, b"ABCDEFGHABCDEFGHABCDEFGHABCDEFGH");
    }

    #[test]
    fn decodes_u24_size_and_empty_image() {
        // An empty image is a valid stream: nothing to decode.
        assert_eq!(
            decompress(&[0x10, 0x00, 0x00, 0x00]).as_deref(),
            Ok(&[][..])
        );
        // Sizes live in a u24: 0xFFFFFF decodes as far as the payload
        // fills (this one fails — the payload is empty).
        assert!(decompress(&[0x10, 0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn rejects_corrupt_streams() {
        // Good vector (the overlapping-run one), mutated one field at a
        // time.
        let good: Vec<u8> = vec![0x10, 0x10, 0x00, 0x00, 0x40, b'X', 0xC0, 0x00];
        assert!(decompress(&good).is_ok());
        // Wrong magic.
        let mut bad = good.clone();
        bad[0] = 0x11;
        assert!(decompress(&bad).is_err());
        // No header at all.
        assert!(decompress(&good[..3]).is_err());
        // Payload truncated mid-literal.
        let mut bad = good.clone();
        bad.truncate(6);
        assert!(decompress(&bad).is_err());
        // Match reaching before the image start (disp 5 at write 1).
        let mut bad = good.clone();
        bad[6] = 0xC4;
        assert!(decompress(&bad).is_err());
        // Match bytes cut off.
        let mut bad = good.clone();
        bad.truncate(7);
        assert!(decompress(&bad).is_err());
        // Header promising an image the payload never fills.
        let mut bad = good;
        bad[1] = 0xFF;
        assert!(decompress(&bad).is_err());
    }

    #[test]
    fn sniffs_the_magic() {
        assert!(is_lz10(&[0x10, 0, 0, 0]));
        assert!(!is_lz10(&[0x11, 0, 0, 0]));
        assert!(!is_lz10(&[]));
    }
}
