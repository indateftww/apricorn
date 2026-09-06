//! MAT — message banks: the game's text storage.
//!
//! Each member of NARC `a/0/2/7` (`msgdata/msg.narc`) is a "MAT" (message
//! archive table) as loaded by the SDK's `ReadMsgData_ExistingNarc_*`:
//!
//! ```text
//! +0x00 2  count   number of messages
//! +0x02 2  key     per-bank seed for the entry obfuscation
//! +0x04 8*count    MAT_ENTRY { u32 offset (bytes), u32 length (u16 units) }
//! ...             the strings: `length` little-endian u16 code units each,
//!                 tiling the member exactly in index order
//! ```
//!
//! The `length` of every retail message counts its trailing EOS unit
//! (`0xFFFF`), and every message's units end with one. The strings are
//! UTF-16-like **code units** for the generation's character encoding
//! (LF is `0xE000`, control codes run through `0xFFFE`, see
//! `constants/charcode.h`) — decoding them against `charmap.txt` is
//! Phase 4 work; this module only decrypts.
//!
//! Both layers are obfuscated with rolling XOR schemes (mirroring
//! `msgdata.c`'s `Decrypt1`/`Decrypt2`):
//!
//! * **Entries** — each entry `n` XORed with the same 16-bit seed
//!   replicated into both halves: `seed = (key * 765 * (n+1)) & 0xFFFF`
//!   (`seed | seed << 16`).
//! * **String units** — message `n`'s units XORed with a rolling seed:
//!   `seed = ((n+1) * 596947) & 0xFFFF`, `seed += 18749` after each unit.
//!
//! Retail HeartGold (US): 829 banks, 49,984 messages, 2,106,048 units in
//! `a/0/2/7` — every one packed exactly as above. See `docs/nitro-msg.md`
//! for the worked ground truth.

use crate::nds::{NdsError, u16le, u32le};

/// The EOS unit every message ends with.
pub const EOS: u16 = 0xFFFF;

/// Decrypt1's per-entry multiplier (`765`).
const ENTRY_MUL: u64 = 765;
/// Decrypt2's per-message seed multiplier (`596947`).
const TEXT_MUL: u64 = 596_947;
/// Decrypt2's per-unit seed increment (`18749`).
const TEXT_ADD: u64 = 18_749;

/// A parsed MAT message bank. Borrows the member bytes; see
/// [`MsgBank::parse`].
///
/// All messages are decrypted and validated at parse time; [`MsgBank::message`]
/// is then a plain slice lookup.
#[derive(Debug)]
pub struct MsgBank<'a> {
    /// The raw member (header, entry table, and string data).
    data: &'a [u8],
    /// The per-bank key, stored raw in the header.
    key: u16,
    /// Every message's decrypted units, concatenated (EOS included).
    units: Vec<u16>,
    /// Start index of each message in [`MsgBank::units`]; one entry per
    /// message plus a final end sentinel, so message `i` spans
    /// `starts[i]..starts[i + 1]`.
    starts: Vec<usize>,
}

impl<'a> MsgBank<'a> {
    /// Parses a complete MAT member.
    ///
    /// Decrypts the entry table and every message, and enforces the retail
    /// invariants: entries tile the member exactly in index order (first at
    /// `4 + 8*count`, last ending at the member end), no zero-length
    /// messages, and every message ending with the EOS unit.
    ///
    /// # Errors
    /// Returns an [`NdsError`] if the member is truncated, the entry table
    /// does not decrypt to the packed layout, or a message does not end
    /// with EOS.
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        if data.len() < 4 {
            return Err(NdsError::Truncated {
                what: "MAT header",
                need: 4,
                got: data.len(),
            });
        }
        let count = usize::from(u16le(data, 0)?);
        let key = u16le(data, 2)?;
        let table_end = 4 + 8 * count;
        if data.len() < table_end {
            return Err(NdsError::Truncated {
                what: "MAT entry table",
                need: table_end,
                got: data.len(),
            });
        }

        // Decrypt1: one seed per entry, replicated into both halves.
        let seed = |n: usize| {
            let seed = (u64::from(key) * ENTRY_MUL * (n as u64 + 1)) & 0xFFFF;
            seed as u32 | (seed as u32) << 16
        };

        // Entries must tile the member exactly: message 0 at the end of the
        // table, each next one right after the last, the final one ending
        // at the member end.
        let mut units = Vec::new();
        let mut starts = Vec::with_capacity(count + 1);
        let mut cursor = table_end;
        for n in 0..count {
            let entry_off = 4 + 8 * n;
            let offset = u32le(data, entry_off)? ^ seed(n);
            let length = u32le(data, entry_off + 4)? ^ seed(n);
            if offset != cursor as u32 {
                return Err(NdsError::Invalid {
                    what: "MAT entries are not packed in index order",
                });
            }
            let length = usize::try_from(length).ok().and_then(|l| l.checked_mul(2));
            let Some(length) = length.filter(|l| *l > 0) else {
                return Err(NdsError::Invalid {
                    what: "MAT message has zero or unrepresentable length",
                });
            };
            let Some(end) = cursor.checked_add(length) else {
                return Err(NdsError::Invalid {
                    what: "MAT message span overflows",
                });
            };
            if end > data.len() {
                return Err(NdsError::Invalid {
                    what: "MAT message span overruns the member",
                });
            }

            // Decrypt2: a rolling seed across the message's units.
            let mut text_seed = ((n as u64 + 1) * TEXT_MUL) as u16;
            starts.push(units.len());
            for i in 0..length / 2 {
                let unit = u16le(data, cursor + 2 * i)? ^ text_seed;
                units.push(unit);
                text_seed = text_seed.wrapping_add(TEXT_ADD as u16);
            }
            if units.last() != Some(&EOS) {
                return Err(NdsError::Invalid {
                    what: "MAT message does not end with EOS",
                });
            }
            cursor = end;
        }
        if cursor != data.len() {
            return Err(NdsError::Invalid {
                what: "MAT messages do not tile the member exactly",
            });
        }
        starts.push(units.len());

        Ok(Self {
            data,
            key,
            units,
            starts,
        })
    }

    /// The number of messages in the bank.
    #[must_use]
    pub fn message_count(&self) -> usize {
        self.starts.len() - 1
    }

    /// The per-bank obfuscation key, as stored in the header.
    #[must_use]
    pub fn key(&self) -> u16 {
        self.key
    }

    /// The message's decrypted code units, including the trailing EOS.
    #[must_use]
    pub fn message(&self, id: usize) -> Option<&[u16]> {
        if id < self.starts.len() - 1 {
            Some(&self.units[self.starts[id]..self.starts[id + 1]])
        } else {
            None
        }
    }

    /// Every message's decrypted code units in index order — always
    /// `message_count()` slices (unlike [`MsgBank::message`], which returns
    /// `None` past the end).
    pub fn messages(&self) -> impl Iterator<Item = &[u16]> + '_ {
        self.starts
            .iter()
            .zip(self.starts.iter().skip(1))
            .map(|(&a, &b)| &self.units[a..b])
    }

    /// The raw member bytes, as passed to [`MsgBank::parse`].
    #[must_use]
    pub fn raw(&self) -> &'a [u8] {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encrypts `units` exactly as Decrypt2 undoes, for building fixtures.
    fn encrypt_units(out: &mut Vec<u8>, n: usize, units: &[u16]) {
        let mut seed = ((n as u64 + 1) * TEXT_MUL) as u16;
        for &u in units {
            out.extend_from_slice(&(u ^ seed).to_le_bytes());
            seed = seed.wrapping_add(TEXT_ADD as u16);
        }
    }

    /// Encrypts entry `n` in place, exactly as Decrypt1 undoes, for
    /// building fixtures.
    fn encrypt_entry(data: &mut [u8], key: u16, n: usize, offset: u32, length: u32) {
        let seed = (u64::from(key) * ENTRY_MUL * (n as u64 + 1)) & 0xFFFF;
        let seed = seed as u32 | (seed as u32) << 16;
        data[4 + 8 * n..8 + 8 * n].copy_from_slice(&(offset ^ seed).to_le_bytes());
        data[8 + 8 * n..12 + 8 * n].copy_from_slice(&(length ^ seed).to_le_bytes());
    }

    /// Builds a well-formed MAT with the given messages (EOS is appended to
    /// each automatically, as every retail message carries one).
    fn build_mat(key: u16, messages: &[&[u16]]) -> Vec<u8> {
        let mut data = vec![0u8; 4 + 8 * messages.len()];
        data[0..2].copy_from_slice(&(messages.len() as u16).to_le_bytes());
        data[2..4].copy_from_slice(&key.to_le_bytes());
        let mut cursor = data.len();
        for (n, &units) in messages.iter().enumerate() {
            let full: Vec<u16> = units.iter().copied().chain([EOS]).collect();
            encrypt_entry(&mut data, key, n, cursor as u32, full.len() as u32);
            encrypt_units(&mut data, n, &full);
            cursor += 2 * full.len();
        }
        data
    }

    #[test]
    fn parses_and_decrypts_messages() {
        // 0x12b.. spells "bulbasaur" in the generation's charmap.
        let bulbasaur = [
            0x12c, 0x13f, 0x136, 0x12c, 0x12b, 0x13d, 0x12b, 0x13f, 0x13c,
        ];
        let with_ctrl = [0xE000, 0xFFFE, 0x0101, 2, 0x0001, 0x0002, 0x12b];
        let data = build_mat(0xFEE8, &[&[0x1be, 0x1be], &bulbasaur, &with_ctrl]);
        assert_eq!(data.len(), 4 + 8 * 3 + 2 * (3 + 10 + 8));

        let bank = MsgBank::parse(&data).expect("fixture must parse");
        assert_eq!(bank.message_count(), 3);
        assert_eq!(bank.key(), 0xFEE8);
        assert_eq!(bank.message(0), Some(&[0x1be, 0x1be, EOS][..]));
        assert_eq!(
            bank.message(1),
            Some(
                &[
                    0x12c, 0x13f, 0x136, 0x12c, 0x12b, 0x13d, 0x12b, 0x13f, 0x13c, EOS
                ][..]
            )
        );
        assert_eq!(
            bank.message(2),
            Some(&[0xE000, 0xFFFE, 0x0101, 2, 0x0001, 0x0002, 0x12b, EOS][..])
        );
        assert_eq!(bank.message(3), None);
        assert_eq!(bank.messages().count(), 3);
        assert_eq!(bank.raw(), &data[..]);

        // A different key must yield different ciphertext: fixture with key
        // 0 is not the same bytes as key 0xFEE8.
        let other = build_mat(0x0001, &[&[0x1be, 0x1be]]);
        let other = MsgBank::parse(&other).expect("fixture must parse");
        assert_eq!(other.message(0), bank.message(0));
        assert_ne!(&other.raw()[4..], &bank.raw()[4..]);
    }

    #[test]
    fn parses_empty_bank() {
        let data = build_mat(0x1234, &[]);
        assert_eq!(data.len(), 4);
        let bank = MsgBank::parse(&data).expect("empty bank must parse");
        assert_eq!(bank.message_count(), 0);
        assert_eq!(bank.messages().count(), 0);
        assert_eq!(bank.message(0), None);
    }

    #[test]
    fn rejects_broken_mat() {
        let good = build_mat(0xFEE8, &[&[0x1be], &[0x12b, 0x12c]]);

        // Header or entry table truncated.
        assert!(MsgBank::parse(&good[..3]).is_err());
        let cut_table = &good[..4 + 8];
        assert!(MsgBank::parse(cut_table).is_err());

        // Entry offset pointing into the table instead of after it.
        let mut bad = good.clone();
        bad[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert!(MsgBank::parse(&bad).is_err());

        // Gap between the two messages (entries not packed).
        let mut bad = good.clone();
        let entry1_off = 4 + 8;
        let mut off1 = u32::from_le_bytes(bad[entry1_off..entry1_off + 4].try_into().unwrap());
        off1 ^= 1;
        bad[entry1_off..entry1_off + 4].copy_from_slice(&off1.to_le_bytes());
        assert!(MsgBank::parse(&bad).is_err());

        // Trailing slack: the member is one byte longer than its messages.
        let mut bad = good.clone();
        bad.push(0);
        assert!(MsgBank::parse(&bad).is_err());

        // Message span overrunning the member.
        let mut bad = good.clone();
        bad.truncate(bad.len() - 2);
        assert!(MsgBank::parse(&bad).is_err());

        // A message that does not end with EOS. Rebuild the fixture with
        // the terminator stripped: entry lengths still say "included", so
        // the plaintext's last unit is wrong.
        let mut bad = good.clone();
        let last = bad.len();
        let mut unit = u16::from_le_bytes(bad[last - 2..last].try_into().unwrap());
        // The last unit sits at message index 1, unit index 1 (the EOS slot).
        let mut seed = (2 * TEXT_MUL) as u16;
        seed = seed.wrapping_add(TEXT_ADD as u16);
        unit ^= seed;
        unit ^= 0x1234; // decrypt, break, re-encrypt
        unit ^= seed;
        bad[last - 2..last].copy_from_slice(&unit.to_le_bytes());
        assert!(MsgBank::parse(&bad).is_err());

        // Zero-length message: a one-message MAT whose entry claims a
        // length of 0.
        let mut bad = vec![0u8; 12];
        bad[0..2].copy_from_slice(&1u16.to_le_bytes());
        bad[2..4].copy_from_slice(&0x1234u16.to_le_bytes());
        encrypt_entry(&mut bad, 0x1234, 0, 12, 0);
        assert!(MsgBank::parse(&bad).is_err());

        // A wrong bank key decrypts the entries to garbage.
        let mut bad = good.clone();
        bad[2..4].copy_from_slice(&0u16.to_le_bytes());
        assert!(MsgBank::parse(&bad).is_err());
    }
}
