//! BLZ — the ARM9 overlay compression (backwards LZ77).
//!
//! HeartGold's 127 compressed overlays (every one except 35 and 124) are
//! BLZ-compressed by pret's `tools/compstatic/compress.c`, which builds
//! the retail overlay table byte-identically. arm9 itself is stored
//! plain — retail never ran compstatic's `-9` static-module step, so BLZ
//! is the *only* compression anywhere in a HeartGold ROM (no NARC member
//! or loose NitroFS file is LZ77-10 compressed).
//!
//! BLZ is LZ77 written backwards: the payload is decoded from its end to
//! its start while writing the image from its end to its start, so
//! matches reference the already-decoded bytes *ahead* of the write
//! cursor. A stored file of `L` bytes lays out as:
//!
//! ```text
//! [0 .. srcOff)      plain head bytes of the image, copied verbatim
//! [srcOff .. payEnd) payload: groups of [codes][flags], read backwards;
//!                    the flags byte sits AFTER its up-to-8 codes in
//!                    memory and is consumed MSB-first; each flags group
//!                    holds exactly 8 codes (the last group may be cut
//!                    short only by the image ending)
//! [payEnd .. L-8)    0xFF padding aligning the footer
//! [L-8 .. L-4)       u32 (addLen << 24) | (L - srcOff)
//! [L-4 .. L)         u32 tailLen == S - L
//! ```
//!
//! where `S` is the uncompressed size (the overlay's `raw_size`),
//! `srcOff = L - (word0 & 0xFFFFFF)`, and `payEnd = L - addLen`. Payload
//! codes, consumed from high addresses down:
//!
//! * literal (flag bit 0) — one byte, copied straight into the image;
//! * match (flag bit 1) — two bytes `[hi][lo]` with `hi` at the higher
//!   address: `len = (hi >> 4) + 3` (3..=18), and the source *end*
//!   `srcEnd = write + ((hi & 0xF) << 8 | lo) + 3`, from which `len`
//!   bytes are copied backwards with `out[write - j] = out[srcEnd - j]`.
//!   The displacement field is the compressor's `i - 2`; sources may
//!   overlap the bytes a match is itself writing (RLE-style runs).
//!
//! Decoding stops when the write cursor reaches `srcOff`, at which point
//! the read cursor must equal `srcOff` exactly (payload consumed to
//! the plain head) — a mismatch means the file is corrupt.
//!
//! See `docs/conversion.md` for the worked retail example.

use super::{NdsError, u32le};

/// Decompresses a BLZ image of `raw_size` bytes.
///
/// `src` is the stored overlay bytes (for overlays, `raw_size` is
/// [`Overlay::raw_size`](super::Overlay::raw_size)).
///
/// # Errors
/// Returns an [`NdsError`] if the footer is missing or inconsistent, the
/// payload overruns or underruns, or a match reaches outside the image —
/// every corruption the retail decoder would misbehave on is rejected
/// before any out-of-bounds access.
pub fn decompress(src: &[u8], raw_size: usize) -> Result<Vec<u8>, NdsError> {
    const FOOTER: usize = 8;
    let len = src.len();
    if len < FOOTER {
        return Err(NdsError::Truncated {
            what: "BLZ image (no footer)",
            need: FOOTER,
            got: len,
        });
    }
    let w0 = u32le(src, len - FOOTER)?;
    let w1 = u32le(src, len - 4)?;

    // tailLen must equal S - L: rejects garbage raw sizes before any
    // allocation.
    if raw_size.checked_sub(len) != Some(w1 as usize) {
        return Err(NdsError::Invalid {
            what: "BLZ footer tailLen does not equal raw size - stored size",
        });
    }

    let src_off = len - (w0 as usize & 0x00FF_FFFF);
    let add_len = (w0 >> 24) as usize;
    let pay_end = len - add_len;
    // addLen always covers at least the footer; padding may grow it.
    if add_len < FOOTER || pay_end < src_off || src_off > raw_size {
        return Err(NdsError::Invalid {
            what: "BLZ footer bounds are inconsistent",
        });
    }

    let mut out = vec![0u8; raw_size];
    out[..src_off].copy_from_slice(&src[..src_off]);
    let mut write = raw_size;
    let mut read = pay_end;
    while write > src_off {
        if read <= src_off {
            return Err(NdsError::Invalid {
                what: "BLZ payload exhausted before the image was filled",
            });
        }
        let flags = src[read - 1];
        read -= 1;
        for bit in (0..8).rev() {
            if write == src_off {
                break; // the image ends mid-group (the last flags group)
            }
            if flags >> bit & 1 == 0 {
                if read <= src_off {
                    return Err(NdsError::Invalid {
                        what: "BLZ literal runs past the payload",
                    });
                }
                read -= 1;
                write -= 1;
                out[write] = src[read];
            } else {
                if read < src_off + 2 {
                    return Err(NdsError::Invalid {
                        what: "BLZ match runs past the payload",
                    });
                }
                let hi = src[read - 1];
                let lo = src[read - 2];
                read -= 2;
                let length = usize::from(hi >> 4) + 3;
                let src_end = write + (usize::from(hi & 0xF) << 8 | usize::from(lo)) + 3;
                if src_end > raw_size || src_end < length || length > write - src_off {
                    return Err(NdsError::Invalid {
                        what: "BLZ match reaches outside the image",
                    });
                }
                for j in 1..=length {
                    out[write - j] = out[src_end - j];
                }
                write -= length;
            }
        }
    }
    if read != src_off {
        return Err(NdsError::Invalid {
            what: "BLZ payload overruns the plain head",
        });
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
    fn decodes_literals_and_matches() {
        // Hand-built vector B (see docs/conversion.md): image "ABCDEFGH" * 3
        // (S=24) via 8 literals + 2 len-8 matches, displacement field 5.
        let src: &[u8] = &[
            0x05, 0x50, 0x05, 0x50, 0xC0, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', 0x00,
            // footer: addLen 8, L - srcOff = 22; tailLen = 24 - 22 = 2
            0x16, 0x00, 0x00, 0x08, 0x02, 0x00, 0x00, 0x00,
        ];
        let out = decompress(src, 24).expect("vector B must decode");
        assert_eq!(out, b"ABCDEFGHABCDEFGHABCDEFGH");
    }

    #[test]
    fn decodes_with_plain_head() {
        // Vector C: S=32, an 8-byte plain head ("HEADHEAD", srcOff = 8)
        // plus the vector-B payload shifted by it.
        let src: &[u8] = &[
            b'H', b'E', b'A', b'D', b'H', b'E', b'A', b'D', 0x05, 0x50, 0x05, 0x50, 0xC0, b'A',
            b'B', b'C', b'D', b'E', b'F', b'G', b'H', 0x00, // footer: L=30, srcOff=8
            0x16, 0x00, 0x00, 0x08, 0x02, 0x00, 0x00, 0x00,
        ];
        let out = decompress(src, 32).expect("vector C must decode");
        assert_eq!(out, b"HEADHEADABCDEFGHABCDEFGHABCDEFGH");
    }

    #[test]
    fn decodes_max_length_and_overlapping_matches() {
        // Vector E: S=32 of 'X' via 3 literals, one len-18 (maximum)
        // match, then one len-11 match whose displacement field 0 makes
        // it copy bytes it has itself just written (overlap).
        let src: &[u8] = &[
            0x00, 0x80, // match: len 11, disp field 0
            0x00, 0xF0, // match: len 18, disp field 0
            b'X', b'X', b'X', 0x18, // three literals + two matches (flags 0x18)
            0x10, 0x00, 0x00, 0x08, // L = 16
            0x10, 0x00, 0x00, 0x00, // tailLen = 32 - 16 = 16
        ];
        let out = decompress(src, 32).expect("vector E must decode");
        assert_eq!(hex(&out), "58".repeat(32));
    }

    #[test]
    fn decodes_padded_footer() {
        // Vector G: S=32, image "ABCDEFGH" * 4, with 4 bytes of 0xFF
        // padding between the payload and the footer (addLen = 12).
        let src: &[u8] = &[
            0x05, 0x50, 0x05, 0x50, 0x05, 0x50, 0xE0, b'A', b'B', b'C', b'D', b'E', b'F', b'G',
            b'H', 0x00, 0xFF, 0xFF, 0xFF, 0xFF, // padding
            0x1C, 0x00, 0x00, 0x0C, // addLen 12, L - srcOff = 28
            0x04, 0x00, 0x00, 0x00, // tailLen = 32 - 28
        ];
        let out = decompress(src, 32).expect("vector G must decode");
        assert_eq!(out, b"ABCDEFGHABCDEFGHABCDEFGHABCDEFGH");
    }

    #[test]
    fn decodes_retail_overlay_9() {
        // The pinned retail bytes of overlay 9: the whole 32-byte image
        // (raw_size 32) stored in 20 bytes — two matches cover the zero
        // fill. This exact byte sequence ships in hg_usa.nds.
        let src: &[u8] = &[
            0x05, 0x50, 0x09, 0x90, 0x03, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1E, // payload
            0x14, 0x00, 0x00, 0x08, // addLen 8, L - srcOff = 20
            0x0C, 0x00, 0x00, 0x00, // tailLen = 32 - 20 = 12
        ];
        let out = decompress(src, 32).expect("overlay 9 must decode");
        assert_eq!(hex(&out), "00".repeat(32));
    }

    #[test]
    fn rejects_corrupt_images() {
        // Vector B with a good footer, mutated one field at a time.
        let good: Vec<u8> = vec![
            0x05, 0x50, 0x05, 0x50, 0xC0, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', 0x00,
            0x16, 0x00, 0x00, 0x08, 0x02, 0x00, 0x00, 0x00,
        ];
        // Wrong raw size: tailLen no longer matches.
        assert!(decompress(&good, 25).is_err());
        // No footer at all.
        assert!(decompress(&good[..12], 24).is_err());
        // Footer tailLen corrupted.
        let mut bad = good.clone();
        bad[19] = 0x03;
        assert!(decompress(&bad, 24).is_err());
        // Match displacement pointing outside the image.
        let mut bad = good.clone();
        bad[0] = 0xFF;
        assert!(decompress(&bad, 24).is_err());
        // A corrupted match displacement byte (pointing outside the
        // image past the top).
        let mut bad = good.clone();
        bad[3] = 0xC1;
        assert!(decompress(&bad, 24).is_err());
        // addLen shorter than the footer itself (tailLen left valid so
        // the addLen bound is what rejects it).
        let mut bad = good.clone();
        bad[17] = 0x07;
        assert!(decompress(&bad, 24).is_err());
    }
}
