//! The NARC container: Nitro's archive format.
//!
//! Almost all of HeartGold's content lives inside NARC archives — the
//! NitroFS's `.narc` files and its hundreds of extensionless `a/…` files
//! alike. A NARC is itself a filesystem: a flat, index-addressed list of
//! member blobs (graphics, models, text banks, raw tables).
//!
//! # Layout
//!
//! Like every Nitro container it opens with a 0x10-byte header:
//!
//! ```text
//! 0x00 4  magic "NARC"
//! 0x04 2  byte-order mark 0xFFFE
//! 0x06 2  version (0x0100 in HeartGold)
//! 0x08 4  total file size
//! 0x0C 2  header size (0x10)
//! 0x0E 2  chunk count (3; a rare FNTI fourth chunk exists in some games)
//! ```
//!
//! followed by exactly three chunks, each `magic + u32 chunk_size` where
//! the size counts the chunk *including* its 8-byte header:
//!
//! - **BTAF** — the member allocation table: `u16 member count`,
//!   `u16 reserved`, then `(u32 start, u32 end)` per member, relative to
//!   the start of the GMIF body. Members are *not* guaranteed contiguous
//!   or 4-byte aligned — `pbr/msg.narc`'s members are odd-sized and
//!   packed without alignment padding, so treat these as plain ranges.
//! - **BTNF** — the filename table, structurally the same directory +
//!   name-subtable scheme as the NitroFS FNT (see [`crate::nds`]): an
//!   array of 8-byte directory records (the root is record 0 and its
//!   third field holds the directory count), then length-prefixed name
//!   records. Subtable offsets are relative to the record-array base.
//!   *Every HeartGold NARC has a root-only, empty BTNF (0x10 bytes)* —
//!   members are addressed by index alone. The empty-table convention
//!   points the root's subtable at offset 4, inside the root record
//!   itself, where the top-file-id byte 0x00 reads as the end marker.
//! - **GMIF** — the member data, concatenated (padded with 0xFF to
//!   4-byte alignment where the writer bothered to align at all).
//!
//! Retail HeartGold (US) contains 308 NARCs holding 56,689 members; see
//! `docs/narc.md` and the integration tests for the worked ground truth.

use crate::nds::{NdsError, u16le, u32le};

/// A parsed NARC archive. Borrows the archive bytes; see [`Narc::parse`].
#[derive(Debug)]
pub struct Narc<'a> {
    data: &'a [u8],
    /// BTAF member ranges, relative to the GMIF body.
    members: Vec<(u32, u32)>,
    /// Absolute offset of the GMIF body within `data`.
    body: usize,
    /// Member names (full `dir/file` paths), by member id. `None` for
    /// index-addressed members (the norm in HeartGold).
    names: Vec<Option<String>>,
}

/// Whether `data` begins with a NARC header.
#[must_use]
pub fn is_narc(data: &[u8]) -> bool {
    data.get(0..6) == Some(&[b'N', b'A', b'R', b'C', 0xFE, 0xFF])
}

impl<'a> Narc<'a> {
    /// Parses a complete NARC archive.
    ///
    /// # Errors
    /// Returns a [`NdsError`] if the header, chunks, allocation table, or
    /// filename table are truncated or inconsistent.
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        if data.len() < 0x10 {
            return Err(NdsError::Truncated {
                what: "NARC header",
                need: 0x10,
                got: data.len(),
            });
        }
        if &data[0..4] != b"NARC" {
            return Err(NdsError::Invalid {
                what: "not a NARC archive",
            });
        }
        if u16le(data, 0x04)? != 0xFFFE {
            return Err(NdsError::Invalid {
                what: "NARC byte-order mark",
            });
        }
        let file_size = u32le(data, 0x08)? as usize;
        if file_size != data.len() {
            return Err(NdsError::Invalid {
                what: "NARC file size does not match its data",
            });
        }
        if u16le(data, 0x0C)? != 0x10 {
            return Err(NdsError::Invalid {
                what: "NARC header size",
            });
        }
        let chunk_count = u16le(data, 0x0E)?;
        if chunk_count != 3 {
            return Err(NdsError::Invalid {
                what: "NARC chunk count (only BTAF/BTNF/GMIF archives are supported)",
            });
        }

        // The three chunks, in order. Each begins with its magic and a
        // size that includes the 8-byte chunk header.
        let expected = [b"BTAF", b"BTNF", b"GMIF"];
        let mut chunks: [(usize, usize); 3] = [(0, 0); 3];
        let mut off = 0x10;
        for (i, magic) in expected.iter().enumerate() {
            if data.get(off..off + 4) != Some(&magic[..]) {
                return Err(NdsError::Invalid {
                    what: "NARC chunk magic or order",
                });
            }
            let size = u32le(data, off + 4)? as usize;
            if size < 8 || off.checked_add(size).is_none() || off + size > data.len() {
                return Err(NdsError::Invalid {
                    what: "NARC chunk overruns the archive",
                });
            }
            chunks[i] = (off, size);
            off += size;
        }
        if off != data.len() {
            return Err(NdsError::Invalid {
                what: "NARC chunks do not cover the whole archive",
            });
        }
        let [
            (btaf_off, btaf_size),
            (btnf_off, btnf_size),
            (gmif_off, gmif_size),
        ] = chunks;

        // BTAF: member count, reserved, then one (start, end) per member.
        if btaf_size < 12 {
            return Err(NdsError::Truncated {
                what: "BTAF chunk",
                need: 12,
                got: btaf_size,
            });
        }
        let member_count = usize::from(u16le(data, btaf_off + 8)?);
        if btaf_size != 12 + member_count * 8 {
            return Err(NdsError::Invalid {
                what: "BTAF chunk size disagrees with the member count",
            });
        }
        let body_len = gmif_size - 8;
        let mut members = Vec::with_capacity(member_count);
        for i in 0..member_count {
            let o = btaf_off + 12 + 8 * i;
            let start = u32le(data, o)?;
            let end = u32le(data, o + 4)?;
            if start > end {
                return Err(NdsError::Invalid {
                    what: "BTAF member start is beyond its end",
                });
            }
            if end as usize > body_len {
                return Err(NdsError::Invalid {
                    what: "BTAF member reaches beyond the GMIF data",
                });
            }
            members.push((start, end));
        }

        let names = parse_btnf(data, btnf_off, btnf_size, member_count)?;

        Ok(Self {
            data,
            members,
            body: gmif_off + 8,
            names,
        })
    }

    /// The number of members in the archive.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.members.len()
    }

    /// The bytes of member `id` (members are index-addressed; this is how
    /// all HeartGold NARCs work).
    ///
    /// # Errors
    /// Returns a [`NdsError`] if the id is out of range.
    pub fn file(&self, id: usize) -> Result<&'a [u8], NdsError> {
        let &(start, end) = self.members.get(id).ok_or(NdsError::Invalid {
            what: "NARC member id out of range",
        })?;
        Ok(&self.data[self.body + start as usize..self.body + end as usize])
    }

    /// The full path (`dir/file`) of member `id`, if the archive's BTNF
    /// names it (most HeartGold archives name nothing and index instead).
    #[must_use]
    pub fn name(&self, id: usize) -> Option<&str> {
        self.names.get(id)?.as_deref()
    }

    /// The bytes of the member at `path`, if the archive's BTNF names it.
    #[must_use]
    pub fn file_by_name(&self, path: &str) -> Option<&'a [u8]> {
        let id = self.names.iter().position(|n| n.as_deref() == Some(path))?;
        self.file(id).ok()
    }
}

/// Parses the BTNF chunk into per-member full paths.
///
/// The layout mirrors the NitroFS FNT (see [`crate::nds`]): an array of
/// 8-byte directory records (the root is record 0; its third field holds
/// the directory count), with name subtables referenced by offsets
/// relative to the record-array base.
fn parse_btnf(
    data: &[u8],
    off: usize,
    size: usize,
    member_count: usize,
) -> Result<Vec<Option<String>>, NdsError> {
    let what = "BTNF";
    let table = data.get(off + 8..off + size).ok_or(NdsError::Truncated {
        what,
        need: off + size,
        got: data.len(),
    })?;
    if table.len() < 8 {
        return Err(NdsError::Truncated {
            what: "BTNF directory records",
            need: 8,
            got: table.len(),
        });
    }
    let dir_count = usize::from(u16le(table, 6)?); // the root's third field
    if table.len() < dir_count * 8 {
        return Err(NdsError::Truncated {
            what: "BTNF directory records",
            need: dir_count * 8,
            got: table.len(),
        });
    }

    // One pass over every directory's name subtable, collecting file
    // entries (with their owning directory) and the parent/child links
    // implied by directory entries.
    // Per member: (name, owning directory). Per directory: name and
    // parent — the root has neither.
    let mut file_names: Vec<Option<(String, usize)>> = vec![None; member_count];
    let mut dir_names: Vec<Option<String>> = vec![None; dir_count];
    let mut dir_parents: Vec<Option<usize>> = vec![None; dir_count];
    for dir in 0..dir_count {
        let subtable = u32le(table, 8 * dir)? as usize;
        let mut next_id = usize::from(u16le(table, 8 * dir + 4)?);
        let mut pos = subtable;
        loop {
            let Some(&first) = table.get(pos) else {
                return Err(NdsError::Truncated {
                    what: "BTNF name subtable",
                    need: pos + 1,
                    got: table.len(),
                });
            };
            pos += 1;
            if first == 0 {
                break; // end of subtable
            }
            let is_dir = first & 0x80 != 0;
            let name_len = usize::from(first & 0x7F);
            let Some(name_bytes) = table.get(pos..pos + name_len) else {
                return Err(NdsError::Truncated {
                    what: "BTNF name",
                    need: pos + name_len,
                    got: table.len(),
                });
            };
            let name = core::str::from_utf8(name_bytes)
                .map_err(|_| NdsError::Invalid {
                    what: "non-UTF-8 NARC member name",
                })?
                .to_owned();
            pos += name_len;

            if is_dir {
                let id = u16le(table, pos).map_err(|_| NdsError::Truncated {
                    what,
                    need: pos + 2,
                    got: table.len(),
                })?;
                pos += 2;
                let child = usize::from(id & 0x0FFF);
                if child == 0 || child >= dir_count {
                    return Err(NdsError::Invalid {
                        what: "BTNF directory ID out of range",
                    });
                }
                dir_names[child] = Some(name);
                dir_parents[child] = Some(dir);
            } else {
                if next_id >= member_count {
                    return Err(NdsError::Invalid {
                        what: "BTNF member id out of range",
                    });
                }
                file_names[next_id] = Some((name, dir));
                next_id += 1;
            }
        }
    }
    if dir_count > 1 && dir_names[1..].iter().any(Option::is_none) {
        return Err(NdsError::Invalid {
            what: "BTNF directory never referenced",
        });
    }

    // Resolve each member's name to a full path through its owning
    // directory's parent chain. The chain cannot be longer than the
    // directory count; exceeding that means a cycle.
    let mut paths: Vec<Option<String>> = Vec::with_capacity(member_count);
    for file_name in file_names {
        let Some((name, mut dir)) = file_name else {
            paths.push(None);
            continue;
        };
        let mut parts = vec![name];
        let mut hops = 0;
        while let Some(parent) = dir_parents[dir] {
            parts.push(dir_names[dir].clone().ok_or(NdsError::Invalid {
                what: "BTNF directory never referenced",
            })?);
            dir = parent;
            hops += 1;
            if hops > dir_count {
                return Err(NdsError::Invalid {
                    what: "BTNF directory parent cycle",
                });
            }
        }
        parts.reverse();
        paths.push(Some(parts.join("/")));
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a NARC in memory with named members laid out as:
    ///
    /// ```text
    /// root:  a.bin (member 0), sub/ (directory)
    /// sub:   c.bin (member 1)
    /// ```
    ///
    /// Member payloads are `AAA` and `CCC`, padded with 0xFF to 4 bytes.
    fn build_named_narc() -> Vec<u8> {
        let payloads: [&[u8]; 2] = [b"AAA", b"CCC"];

        // Directory records: root and "sub".
        let dir_count = 2usize;
        // Root subtable comes after both records; the sub subtable after
        // the root's. Root lists a.bin then sub/; sub lists c.bin.
        let root_subtable = dir_count * 8;
        let root_sub = [
            0x05,
            b'a',
            b'.',
            b'b',
            b'i',
            b'n', // a.bin
            0x80 + 3,
            b's',
            b'u',
            b'b',
            0x01,
            0xF0, // sub -> dir 1 (id 0xF001, little-endian)
            0x00,
        ];
        let sub_sub_off = root_subtable + root_sub.len();
        let sub_sub = [
            0x05, b'c', b'.', b'b', b'i', b'n', // c.bin
            0x00,
        ];
        let btnf_data_len = dir_count * 8 + root_sub.len() + sub_sub.len();

        // BTAF + GMIF: members at 0..3 and 3..6 of the GMIF body.
        let btaf_size = 12 + 8 * payloads.len();
        let gmif_size = 8 + 4 * payloads.len();
        let total = 0x10 + btaf_size + (8 + btnf_data_len) + gmif_size;

        let mut rom = vec![0u8; total];
        rom[0..4].copy_from_slice(b"NARC");
        rom[4..6].copy_from_slice(&0xFFFEu16.to_le_bytes());
        rom[6..8].copy_from_slice(&0x0100u16.to_le_bytes());
        rom[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        rom[0x0C..0x0E].copy_from_slice(&0x10u16.to_le_bytes());
        rom[0x0E..0x10].copy_from_slice(&3u16.to_le_bytes());

        let mut off = 0x10;
        // BTAF
        rom[off..off + 4].copy_from_slice(b"BTAF");
        rom[off + 4..off + 8].copy_from_slice(&(btaf_size as u32).to_le_bytes());
        rom[off + 8..off + 10].copy_from_slice(&2u16.to_le_bytes()); // member count
        for (i, (start, end)) in [(0u32, 3u32), (4, 7)].iter().enumerate() {
            rom[off + 12 + 8 * i..off + 16 + 8 * i].copy_from_slice(&start.to_le_bytes());
            rom[off + 16 + 8 * i..off + 20 + 8 * i].copy_from_slice(&end.to_le_bytes());
        }
        off += btaf_size;

        // BTNF
        rom[off..off + 4].copy_from_slice(b"BTNF");
        rom[off + 4..off + 8].copy_from_slice(&((8 + btnf_data_len) as u32).to_le_bytes());
        let btnf = off + 8;
        let mut rec = |o: usize, sub: u32, top: u16, parent: u16| {
            rom[btnf + o..btnf + o + 4].copy_from_slice(&sub.to_le_bytes());
            rom[btnf + o + 4..btnf + o + 6].copy_from_slice(&top.to_le_bytes());
            rom[btnf + o + 6..btnf + o + 8].copy_from_slice(&parent.to_le_bytes());
        };
        rec(0, root_subtable as u32, 0, dir_count as u16); // root: a.bin starts at id 0
        rec(8, sub_sub_off as u32, 1, 0xF000); // sub: c.bin starts at id 1
        rom[btnf + root_subtable..btnf + root_subtable + root_sub.len()].copy_from_slice(&root_sub);
        rom[btnf + sub_sub_off..btnf + sub_sub_off + sub_sub.len()].copy_from_slice(&sub_sub);
        off += 8 + btnf_data_len;

        // GMIF: payloads padded with 0xFF (the NARC writer's padding byte).
        rom[off..off + 4].copy_from_slice(b"GMIF");
        rom[off + 4..off + 8].copy_from_slice(&(gmif_size as u32).to_le_bytes());
        for (i, payload) in payloads.iter().enumerate() {
            rom[off + 8 + 4 * i..off + 8 + 4 * i + payload.len()].copy_from_slice(payload);
        }
        rom
    }

    #[test]
    fn parses_named_members() {
        let data = build_named_narc();
        let narc = Narc::parse(&data).expect("named NARC must parse");

        assert_eq!(narc.file_count(), 2);
        assert_eq!(narc.file(0).unwrap(), b"AAA");
        assert_eq!(narc.file(1).unwrap(), b"CCC");
        assert_eq!(narc.name(0), Some("a.bin"));
        assert_eq!(narc.name(1), Some("sub/c.bin"));
        assert_eq!(narc.file_by_name("sub/c.bin"), Some(&b"CCC"[..]));
        assert!(narc.file_by_name("nope.bin").is_none());
        assert!(narc.file(2).is_err());
    }

    /// The retail HeartGold convention: a root-only, empty name table
    /// whose root subtable offset (4) points *inside* the root record,
    /// where the top-file-id byte 0x00 serves as the end marker.
    #[test]
    fn parses_retail_style_index_only_archive() {
        // NARC header + BTAF(1 member 0..1) + BTNF(root only) + GMIF(0x00 + 0xFF pad).
        let data = [
            b'N', b'A', b'R', b'C', 0xFE, 0xFF, 0x00, 0x01, 0x40, 0x00, 0x00, 0x00, 0x10, 0x00,
            0x03, 0x00, b'B', b'T', b'A', b'F', 0x14, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, b'B', b'T', b'N', b'F', 0x10, 0x00,
            0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, b'G', b'M', b'I', b'F',
            0x0C, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF,
        ];
        assert!(is_narc(&data));
        let narc = Narc::parse(&data).expect("retail-style NARC must parse");
        assert_eq!(narc.file_count(), 1);
        assert_eq!(narc.file(0).unwrap(), &[0x00]);
        assert_eq!(narc.name(0), None);
    }

    #[test]
    fn rejects_broken_archives() {
        let good = build_named_narc();

        let mut bad_magic = good.clone();
        bad_magic[0..4].copy_from_slice(b"NARB");
        assert!(Narc::parse(&bad_magic).is_err());

        let truncated = &good[..good.len() - 4];
        assert!(Narc::parse(truncated).is_err());

        // File-size field one byte off.
        let mut bad_size = good.clone();
        bad_size[8..12].copy_from_slice(&((good.len() - 1) as u32).to_le_bytes());
        assert!(Narc::parse(&bad_size).is_err());

        // A member reaching beyond the GMIF body.
        let mut bad_range = good.clone();
        bad_range[0x1C..0x20].copy_from_slice(&0xFFFFu32.to_le_bytes());
        assert!(Narc::parse(&bad_range).is_err());

        // start > end.
        let mut reversed = good.clone();
        reversed[0x1C..0x20].copy_from_slice(&3u32.to_le_bytes());
        reversed[0x20..0x24].copy_from_slice(&0u32.to_le_bytes());
        assert!(Narc::parse(&reversed).is_err());

        assert!(!is_narc(b"not a narc at all"));
        assert!(Narc::parse(b"NARC").is_err());
    }
}
