//! The Nitro filesystem: File Name Table (FNT) and File Allocation Table
//! (FAT).
//!
//! # FNT layout
//!
//! The FNT begins with an array of 8-byte directory records, indexed by
//! directory ID (`0xF000 | index`, the root being `0xF000`):
//!
//! ```text
//! u32 entry_start    // offset of this directory's name subtable, from FNT base
//! u16 top_file_id   // FAT id of the first file listed in the subtable
//! u16 parent        // parent directory ID; for the root: the directory count
//! ```
//!
//! Name subtables are length-prefixed records:
//! - `0x00` — end of subtable.
//! - `0x01..=0x7F` — file: name length, then the name; consumes one FAT id.
//! - `0x80..=0xFF` — directory: name length minus 0x80, the name, then a
//!   `u16` directory ID.
//!
//! # FAT
//!
//! The FAT is a flat array of `(u32 start, u32 end)` pairs indexed by file
//! ID. Overlay files occupy the first IDs (129 of them in HeartGold), which
//! is why the root directory's `top_file_id` is 0x81.

use super::{u16le, u32le, NdsError};

/// A file enumerated from the FNT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NitroFile {
    /// Full path relative to the NitroFS root, e.g. `data/UTF16.dat`.
    pub path: String,
    /// Index into the FAT locating the file's bytes.
    pub fat_id: u32,
}

/// A directory enumerated from the FNT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NitroDir {
    /// Full path relative to the NitroFS root; the root itself is `""`.
    pub path: String,
    /// Raw directory ID (`0xF000 | index`; the root is `0xF000`).
    pub id: u16,
    /// Raw parent directory ID; for the root this field holds the total
    /// directory count instead (a quirk of the format).
    pub parent: u16,
}

/// The parsed contents of a ROM's Nitro filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NitroFs {
    dirs: Vec<NitroDir>,
    files: Vec<NitroFile>,
    fat: Vec<(u32, u32)>,
}

impl NitroFs {
    /// Parses the FNT and FAT slices of a ROM.
    ///
    /// # Errors
    /// Returns a [`NdsError`] if the tables are truncated, reference
    /// out-of-range directory IDs, leave a directory unreferenced, or
    /// contain a name that is not valid UTF-8.
    pub fn parse(fnt: &[u8], fat: &[u8]) -> Result<Self, NdsError> {
        let what = "FNT";
        if fnt.len() < 8 {
            return Err(NdsError::Truncated { what, need: 8, got: fnt.len() });
        }
        let dir_count = usize::from(u16le(fnt, 6)?);
        let need = dir_count * 8;
        if fnt.len() < need {
            return Err(NdsError::Truncated { what, need, got: fnt.len() });
        }

        // The root record is record 0; its entry_start locates the root name
        // subtable and its "parent" field is the directory count.
        let mut records = Vec::with_capacity(dir_count);
        for i in 0..dir_count {
            records.push((u32le(fnt, 8 * i)?, u16le(fnt, 8 * i + 4)?, u16le(fnt, 8 * i + 6)?));
        }

        let mut dirs: Vec<NitroDir> = records
            .iter()
            .enumerate()
            .map(|(i, &(_, _, parent))| NitroDir {
                path: String::new(),
                id: 0xF000 | i as u16,
                parent,
            })
            .collect();

        let mut files = Vec::new();
        let mut stack: Vec<(usize, String)> = vec![(0, String::new())];
        while let Some((index, prefix)) = stack.pop() {
            let (entry_start, mut next_id, _) = records[index];
            let mut pos = entry_start as usize;
            loop {
                let Some(&first) = fnt.get(pos) else {
                    return Err(NdsError::Truncated { what, need: pos + 1, got: fnt.len() });
                };
                pos += 1;
                if first == 0 {
                    break; // end of subtable
                }
                let is_dir = first & 0x80 != 0;
                let name_len = usize::from(first & 0x7F);
                let Some(name_bytes) = fnt.get(pos..pos + name_len) else {
                    return Err(NdsError::Truncated { what, need: pos + name_len, got: fnt.len() });
                };
                let name = String::from_utf8(name_bytes.to_vec())
                    .map_err(|_| NdsError::Invalid { what: "non-UTF-8 NitroFS name" })?;
                pos += name_len;

                let path = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                };

                if is_dir {
                    let id = u16le(fnt, pos).map_err(|_| NdsError::Truncated {
                        what,
                        need: pos + 2,
                        got: fnt.len(),
                    })?;
                    pos += 2;
                    let child = usize::from(id & 0x0FFF);
                    if child == 0 || child >= dir_count {
                        return Err(NdsError::Invalid { what: "FNT directory ID out of range" });
                    }
                    dirs[child].path = path.clone();
                    stack.push((child, path));
                } else {
                    files.push(NitroFile { path, fat_id: u32::from(next_id) });
                    next_id += 1;
                }
            }
        }

        if dirs.iter().skip(1).any(|d| d.path.is_empty()) {
            return Err(NdsError::Invalid { what: "FNT directory never referenced" });
        }

        let fat_what = "FAT";
        let fat_count = fat.len() / 8;
        let mut fat_entries = Vec::with_capacity(fat_count);
        for i in 0..fat_count {
            let start = u32le(fat, 8 * i).map_err(|_| NdsError::Truncated {
                what: fat_what,
                need: 8 * i + 8,
                got: fat.len(),
            })?;
            let end = u32le(fat, 8 * i + 4).map_err(|_| NdsError::Truncated {
                what: fat_what,
                need: 8 * i + 8,
                got: fat.len(),
            })?;
            fat_entries.push((start, end));
        }

        Ok(Self { dirs, files, fat: fat_entries })
    }

    /// All directories, in record order (index 0 is the root, path `""`).
    #[must_use]
    pub fn dirs(&self) -> &[NitroDir] {
        &self.dirs
    }

    /// All files, in traversal order.
    #[must_use]
    pub fn files(&self) -> &[NitroFile] {
        &self.files
    }

    /// The FAT as `(start, end)` pairs indexed by file ID.
    #[must_use]
    pub fn fat(&self) -> &[(u32, u32)] {
        &self.fat
    }

    /// Looks up the FAT id of the file at `path`.
    #[must_use]
    pub fn fat_id_by_path(&self, path: &str) -> Option<u32> {
        self.files.iter().find(|f| f.path == path).map(|f| f.fat_id)
    }
}