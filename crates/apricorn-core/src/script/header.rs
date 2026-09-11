//! The per-map init-script header — `MapHeader.scriptHeaderBank`, an
//! `a/0/1/2` member alongside the map's script bank
//! (`MapScriptHeader_ReadFromNarc`, `src/map_events.c:205`).
//!
//! Layout, from pret's `InitScriptEntry_*` macros
//! (`asm/macros/script.inc:4967-5034`) and the readers
//! `GetMapLoadScriptId` / `GetMapSceneScriptId`
//! (`src/script_manager.c:618-660`):
//!
//! ```text
//! entry*      u8 type; u32 payload            (5 bytes each)
//!             type 2/3/4 (ON_TRANSITION / ON_RESUME / ON_LOAD): payload = u16 script id, u16 0
//!             type 1 (ON_FRAME_TABLE):        payload = offset of the table, relative to after the entry
//! u8 0        end of entries
//! table       (u16 var1, u16 var2, u16 script)* terminated by u16 0
//! ```
//!
//! Script ids here are the map's own (`1..2000`, so `script - 1` indexes
//! the map bank) and the frame table runs the first row whose two
//! variables read equal. `0xFFFF` from either reader means "none".
//!
//! Note for the orchestrator: the map-data workstream parses the same
//! member in `field/script_header.rs`; this minimal reader exists so
//! the VM has no dependency on it, and the two should be merged.

use crate::assets::{AssetStore, AssetsError};

use super::bank::SCRIPT_NARC;

/// `INIT_SCRIPT_ON_FRAME_TABLE` (`include/constants/init_script_types.h`).
pub const ON_FRAME_TABLE: u8 = 1;
/// `INIT_SCRIPT_ON_TRANSITION` — runs on first entering a map.
pub const ON_TRANSITION: u8 = 2;
/// `INIT_SCRIPT_ON_RESUME` — runs once the map is drawn.
pub const ON_RESUME: u8 = 3;
/// `INIT_SCRIPT_ON_LOAD` — runs once the layout is loaded.
pub const ON_LOAD: u8 = 4;

/// One `InitScriptGoToIfEqual var1, var2, scriptID` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTableEntry {
    /// The first variable id (or literal).
    pub var1: u16,
    /// The second variable id (or literal).
    pub var2: u16,
    /// The map script id to run when they read equal.
    pub script: u16,
}

/// A loaded init-script header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitScriptHeader {
    bytes: Vec<u8>,
}

impl InitScriptHeader {
    /// Wraps the member bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Loads member `member` of the script archive.
    ///
    /// # Errors
    /// [`AssetsError::Missing`] for no such member.
    pub fn load(store: &AssetStore, member: usize) -> Result<Self, AssetsError> {
        Ok(Self::new(store.member(SCRIPT_NARC, member)?.into_owned()))
    }

    /// The raw bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The `(type, payload)` entries in order, up to the terminator
    /// (or the end of the member, for a header without one).
    #[must_use]
    pub fn entries(&self) -> Vec<(u8, u32)> {
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(&kind) = self.bytes.get(at) {
            if kind == 0 {
                break;
            }
            let Some(p) = self.bytes.get(at + 1..at + 5) else {
                break;
            };
            out.push((kind, u32::from_le_bytes([p[0], p[1], p[2], p[3]])));
            at += 5;
        }
        out
    }

    /// `GetMapLoadScriptId` (`src/script_manager.c:618`): the script id
    /// of the first entry of type `kind`, read as the payload's low
    /// halfword. `None` for `0xFFFF` (no entry). Meant for types 2–4;
    /// type 1's payload is an offset, which the original never reads
    /// this way (`TryStartMapScriptByType` routes type 1 to
    /// [`Self::scene_script_id`]).
    #[must_use]
    pub fn load_script_id(&self, kind: u8) -> Option<u16> {
        self.entries()
            .into_iter()
            .find(|&(k, _)| k == kind)
            .map(|(_, payload)| payload as u16)
            .filter(|&id| id != 0xFFFF)
    }

    /// The frame table an entry of type `kind` (normally
    /// [`ON_FRAME_TABLE`]) points at — every row up to its terminator.
    /// `None` when there is no such entry or its offset is 0.
    #[must_use]
    pub fn frame_table(&self, kind: u8) -> Option<Vec<FrameTableEntry>> {
        let mut at = 0;
        let table = loop {
            let &k = self.bytes.get(at)?;
            if k == 0 {
                return None;
            }
            let p = self.bytes.get(at + 1..at + 5)?;
            at += 5;
            if k == kind {
                let ofs = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                if ofs == 0 {
                    return None;
                }
                break at.wrapping_add(ofs as usize);
            }
        };
        let mut rows = Vec::new();
        let mut at = table;
        loop {
            let r = self.bytes.get(at..at + 2)?;
            let var1 = u16::from_le_bytes([r[0], r[1]]);
            if var1 == 0 {
                return Some(rows);
            }
            let r = self.bytes.get(at + 2..at + 6)?;
            rows.push(FrameTableEntry {
                var1,
                var2: u16::from_le_bytes([r[0], r[1]]),
                script: u16::from_le_bytes([r[2], r[3]]),
            });
            at += 6;
        }
    }

    /// `GetMapSceneScriptId` (`src/script_manager.c:630`): the script id
    /// of the first frame-table row whose two variables, read through
    /// `var` (`FieldSystem_VarGet`: ids below `VAR_BASE` are literals),
    /// are equal. `None` when no entry, no table, or no row matches.
    #[must_use]
    pub fn scene_script_id(&self, kind: u8, mut var: impl FnMut(u16) -> u16) -> Option<u16> {
        self.frame_table(kind)?
            .into_iter()
            .find(|row| var(row.var1) == var(row.var2))
            .map(|row| row.script)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The T20 (New Bark Town) header shape, `scr_seq_0615_T20_hdr.s`:
    /// an ON_FRAME_TABLE entry, ON_TRANSITION 7, ON_RESUME 10, end;
    /// then two rows.
    fn new_bark() -> InitScriptHeader {
        let mut b = vec![ON_FRAME_TABLE];
        // Offset from after this entry (byte 5) to the table at byte 16.
        b.extend_from_slice(&11u32.to_le_bytes());
        b.push(ON_TRANSITION);
        b.extend_from_slice(&7u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.push(ON_RESUME);
        b.extend_from_slice(&10u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.push(0);
        assert_eq!(b.len(), 16);
        for (v1, v2, s) in [(0x4106u16, 1u16, 4u16), (0x4072, 1, 9)] {
            b.extend_from_slice(&v1.to_le_bytes());
            b.extend_from_slice(&v2.to_le_bytes());
            b.extend_from_slice(&s.to_le_bytes());
        }
        b.extend_from_slice(&0u16.to_le_bytes());
        InitScriptHeader::new(b)
    }

    #[test]
    fn fixed_entries_read_like_the_original() {
        let h = new_bark();
        assert_eq!(h.entries().len(), 3);
        assert_eq!(h.load_script_id(ON_TRANSITION), Some(7));
        assert_eq!(h.load_script_id(ON_RESUME), Some(10));
        assert_eq!(h.load_script_id(ON_LOAD), None);
        // An empty header (the bedroom's: a lone terminator, padded).
        let empty = InitScriptHeader::new(vec![0, 0, 0, 0]);
        assert!(empty.entries().is_empty());
        assert_eq!(empty.load_script_id(ON_TRANSITION), None);
        assert_eq!(empty.frame_table(ON_FRAME_TABLE), None);
        // 0xFFFF is "none" even when present.
        let none = InitScriptHeader::new(vec![ON_LOAD, 0xFF, 0xFF, 0, 0, 0]);
        assert_eq!(none.load_script_id(ON_LOAD), None);
    }

    #[test]
    fn frame_table_picks_the_first_equal_row() {
        let h = new_bark();
        let rows = h.frame_table(ON_FRAME_TABLE).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            FrameTableEntry {
                var1: 0x4106,
                var2: 1,
                script: 4
            }
        );
        // Variables read through the closure; literals are themselves.
        let vars = |v: u16| match v {
            0x4106 => 0,
            0x4072 => 1,
            other => other,
        };
        assert_eq!(h.scene_script_id(ON_FRAME_TABLE, vars), Some(9));
        let vars = |v: u16| if v >= 0x4000 { 1 } else { v };
        assert_eq!(h.scene_script_id(ON_FRAME_TABLE, vars), Some(4));
        let vars = |v: u16| if v >= 0x4000 { 5 } else { v };
        assert_eq!(h.scene_script_id(ON_FRAME_TABLE, vars), None);
        assert_eq!(h.scene_script_id(ON_LOAD, |v| v), None);
    }
}
