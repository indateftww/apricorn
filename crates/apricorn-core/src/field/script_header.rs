//! Map script headers — pret `src/script_manager.c:618` (`GetMapLoadScriptId`,
//! `GetMapSceneScriptId`) and `include/constants/init_script_types.h`:
//! NARC `a/0/1/2` (`fielddata/script/scr_seq`) member
//! `MapHeader::script_header_bank`, read into a 0x100-byte buffer
//! (`MapScriptHeader_ReadFromNarc` asserts the member is smaller).
//!
//! The member is 5-byte records `{u8 type; u32 payload}` terminated by
//! type 0. For types 2–4 the script id is the payload's low 16 bits
//! (0xFFFF = none). For type 1 (`ON_FRAME_TABLE`) the payload is an
//! offset from the byte *after* the record to a table of 6-byte
//! `{u16 varA, varB, scriptId}` entries terminated by `varA == 0`; a
//! zero offset means no table. Each frame the first entry whose two
//! variables compare equal runs.

use crate::assets::{AssetStore, AssetsError};
use crate::nds::{NdsError, u16le, u32le};

/// NitroFS path of the script archive.
pub const SCRIPT_NARC: &str = "a/0/1/2";
/// The game's header buffer; members must be smaller.
pub const MAX_BYTES: usize = 0x100;
/// Bytes per header record.
pub const RECORD_SIZE: usize = 5;
/// Bytes per frame-table entry.
pub const FRAME_TABLE_ENTRY_SIZE: usize = 6;
/// `INIT_SCRIPT_ON_FRAME_TABLE`.
pub const ON_FRAME_TABLE: u8 = 1;
/// `INIT_SCRIPT_ON_TRANSITION`.
pub const ON_TRANSITION: u8 = 2;
/// `INIT_SCRIPT_ON_RESUME`.
pub const ON_RESUME: u8 = 3;
/// `INIT_SCRIPT_ON_LOAD`.
pub const ON_LOAD: u8 = 4;
/// The script id meaning "none".
pub const NO_SCRIPT: u16 = 0xFFFF;

/// One header record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InitScriptRecord {
    /// The init script type (`ON_*`).
    pub kind: u8,
    /// The raw payload: a script id (low 16 bits) or, for
    /// [`ON_FRAME_TABLE`], the table offset.
    pub payload: u32,
}

/// One frame-table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameTableEntry {
    /// `varA`.
    pub var_a: u16,
    /// `varB`.
    pub var_b: u16,
    /// The script to run when `varA == varB`.
    pub script_id: u16,
}

/// A parsed script header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitScripts {
    /// The records before the terminator, in order.
    pub records: Vec<InitScriptRecord>,
    /// The `ON_FRAME_TABLE` entries (empty without a table).
    pub frame_table: Vec<FrameTableEntry>,
}

impl InitScripts {
    /// Decodes a member.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the member is larger than the game's
    /// buffer, has no terminator, or a frame table runs past its end.
    pub fn parse(bytes: &[u8]) -> Result<Self, NdsError> {
        if bytes.len() >= MAX_BYTES {
            return Err(NdsError::Invalid {
                what: "script header size",
            });
        }
        let mut records = Vec::new();
        let mut frame_table = Vec::new();
        let mut p = 0;
        loop {
            let kind = *bytes.get(p).ok_or(NdsError::Truncated {
                what: "script header terminator",
                need: p + 1,
                got: bytes.len(),
            })?;
            if kind == 0 {
                break;
            }
            let payload = u32le(bytes, p + 1)?;
            records.push(InitScriptRecord { kind, payload });
            p += RECORD_SIZE;
            if kind == ON_FRAME_TABLE && payload != 0 && frame_table.is_empty() {
                let mut q = p + payload as usize;
                loop {
                    let var_a = u16le(bytes, q)?;
                    if var_a == 0 {
                        break;
                    }
                    frame_table.push(FrameTableEntry {
                        var_a,
                        var_b: u16le(bytes, q + 2)?,
                        script_id: u16le(bytes, q + 4)?,
                    });
                    q += FRAME_TABLE_ENTRY_SIZE;
                }
            }
        }
        Ok(Self {
            records,
            frame_table,
        })
    }

    /// Loads member `bank` (`MapHeader::script_header_bank`).
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the member is missing or malformed.
    pub fn load(store: &AssetStore, bank: u16) -> Result<Self, AssetsError> {
        let bytes = store.member(SCRIPT_NARC, usize::from(bank))?;
        Self::parse(&bytes).map_err(|source| AssetsError::Corrupt {
            what: format!("{SCRIPT_NARC}#{bank}"),
            source,
        })
    }

    /// `GetMapLoadScriptId`: the script of the first record of `kind`,
    /// `None` when absent or [`NO_SCRIPT`].
    #[must_use]
    pub fn script(&self, kind: u8) -> Option<u16> {
        let id = self.records.iter().find(|r| r.kind == kind)?.payload as u16;
        (id != NO_SCRIPT).then_some(id)
    }

    /// The [`ON_TRANSITION`] script.
    #[must_use]
    pub fn on_transition(&self) -> Option<u16> {
        self.script(ON_TRANSITION)
    }

    /// The [`ON_RESUME`] script.
    #[must_use]
    pub fn on_resume(&self) -> Option<u16> {
        self.script(ON_RESUME)
    }

    /// The [`ON_LOAD`] script.
    #[must_use]
    pub fn on_load(&self) -> Option<u16> {
        self.script(ON_LOAD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_frame_table_decode() {
        // type 1 → table 6 bytes after its record; type 3 → script 10; terminator;
        // table: (0x141, 6, 0x4072) then a zero varA.
        let b = [
            0x01, 0x06, 0x00, 0x00, 0x00, 0x03, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x41, 0x01, 0x06,
            0x00, 0x72, 0x40, 0x00, 0x00,
        ];
        let s = InitScripts::parse(&b).unwrap();
        assert_eq!(s.records.len(), 2);
        assert_eq!(s.on_resume(), Some(10));
        assert_eq!(s.on_transition(), None);
        assert_eq!(
            s.frame_table,
            vec![FrameTableEntry { var_a: 0x141, var_b: 6, script_id: 0x4072 }]
        );
        assert_eq!(InitScripts::parse(&[0, 0, 0, 0]).unwrap(), InitScripts::default());
        assert!(InitScripts::parse(&[1, 0, 0, 0, 0]).is_err(), "no terminator");
        assert!(InitScripts::parse(&[1, 9, 0, 0, 0, 0]).is_err(), "table past end");
        assert_eq!(InitScripts::parse(&[2, 0xFF, 0xFF, 0, 0, 0]).unwrap().on_transition(), None);
    }
}
