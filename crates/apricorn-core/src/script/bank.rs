//! Script banks — the members of `NARC_fielddata_script_scr_seq`
//! (`a/0/1/2`, 965 members) — and which bank a script id names.
//!
//! A bank member is an entry table of little-endian u32 offsets, one
//! per script, terminated by the u16 `SCRDEF_END` (`0xFD13`), followed
//! by the bytecode. Each entry is relative to the byte *after* it —
//! pret's `ScrDef` macro assembles `.word target - . - 4`, and
//! `ScriptRunByIndex` (`src/script_manager.c:341`) does
//! `script_ptr += 4 * idx; script_ptr += ScriptReadWord(ctx)`, the read
//! having advanced the pointer past the word.
//!
//! Which bank a script id lives in is `LoadScriptsAndMessagesByMapId`
//! (`src/script_manager.c:194`): ids at or above a threshold in
//! [`STD_BANK_MAPPING`] (pret's `sScriptBankMapping`, scanned from the
//! highest threshold down) select a fixed script/message bank pair and
//! become an index relative to the threshold; ids `1..` below every
//! threshold are the current map's own banks (index `id - 1`, from
//! the map header); id 0 is the "everywhere" pair.

use crate::assets::{AssetStore, AssetsError};
use crate::nds::NdsError;

use super::ScriptError;

/// The script archive in NitroFS (`NARC_fielddata_script_scr_seq`).
pub const SCRIPT_NARC: &str = "a/0/1/2";
/// The message archive in NitroFS (`NARC_msgdata_msg`).
pub const MSG_NARC: &str = "a/0/2/7";
/// `SCRDEF_END` — terminates a bank's entry table
/// (`include/constants/scrcmd.h:42`).
pub const SCRDEF_END: u16 = 0xFD13;

/// One `a/0/1/2` member: its entry table resolved to absolute offsets,
/// plus the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptBank {
    data: Vec<u8>,
    entries: Vec<usize>,
}

impl ScriptBank {
    /// Parses a member: reads u32 entries until the `SCRDEF_END`
    /// halfword, resolving each to an absolute offset.
    ///
    /// # Errors
    /// [`ScriptError::BadBank`] when the table runs off the member or
    /// an entry points outside it.
    pub fn parse(data: Vec<u8>) -> Result<Self, ScriptError> {
        let mut entries = Vec::new();
        let mut at = 0usize;
        loop {
            let Some(head) = data.get(at..at + 2) else {
                return Err(ScriptError::BadBank {
                    what: "entry table runs off the member",
                });
            };
            if u16::from_le_bytes([head[0], head[1]]) == SCRDEF_END {
                break;
            }
            let Some(word) = data.get(at..at + 4) else {
                return Err(ScriptError::BadBank {
                    what: "entry table runs off the member",
                });
            };
            let rel = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
            // Pointer arithmetic on the DS is 32-bit and wraps; an entry
            // is only meaningful inside the member.
            let target = (at + 4).wrapping_add(rel as i32 as isize as usize);
            if target >= data.len() {
                return Err(ScriptError::BadBank {
                    what: "entry points outside the member",
                });
            }
            entries.push(target);
            at += 4;
        }
        Ok(Self { data, entries })
    }

    /// Loads and parses member `member` of [`SCRIPT_NARC`] from the ROM.
    ///
    /// # Errors
    /// [`AssetsError::Missing`] for no such member,
    /// [`AssetsError::Corrupt`] when it does not parse as a bank.
    pub fn load(store: &AssetStore, member: usize) -> Result<Self, AssetsError> {
        let bytes = store.member(SCRIPT_NARC, member)?;
        Self::parse(bytes.into_owned()).map_err(|err| AssetsError::Corrupt {
            what: format!("{SCRIPT_NARC}#{member}"),
            source: NdsError::Invalid {
                what: match err {
                    ScriptError::BadBank { what } => what,
                    _ => "script bank",
                },
            },
        })
    }

    /// The whole member (entry table included) — offsets and relative
    /// branches are measured against it.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// How many scripts the entry table names.
    #[must_use]
    pub fn script_count(&self) -> usize {
        self.entries.len()
    }

    /// Where the entry table ends and bytecode may begin
    /// (`4 * count + 2`).
    #[must_use]
    pub fn code_start(&self) -> usize {
        self.entries.len() * 4 + 2
    }

    /// The absolute offset of script `index` (`ScriptRunByIndex`).
    #[must_use]
    pub fn script_offset(&self, index: usize) -> Option<usize> {
        self.entries.get(index).copied()
    }

    /// Script `index`'s bytes, from its entry to the end of the member
    /// (scripts are not length-delimited; they end at `End`).
    #[must_use]
    pub fn script(&self, index: usize) -> Option<&[u8]> {
        self.script_offset(index).map(|at| &self.data[at..])
    }
}

/// One raw member of any NitroFS archive — the loader a
/// [`RecordingHost`](super::host::RecordingHost) over a ROM uses for
/// script and message banks alike (`AllocAndReadWholeNarcMemberByIdPair`).
///
/// # Errors
/// [`AssetsError::Missing`] for no such archive or member.
pub fn narc_member(store: &AssetStore, narc: &str, member: u16) -> Result<Vec<u8>, AssetsError> {
    Ok(store.member(narc, usize::from(member))?.into_owned())
}

/// The current map's script and message banks (`MapHeader.scriptsBank`
/// / `.msgBank`, `include/map_header.h`), which script ids `1..2000`
/// index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MapBanks {
    /// The `a/0/1/2` member holding the map's event scripts.
    pub scripts: u16,
    /// The `a/0/2/7` member holding the map's text.
    pub messages: u16,
}

/// One row of `sScriptBankMapping` (`src/script_manager.c:17-64`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankMapping {
    /// The lowest script id the row covers (a `_std_*` threshold,
    /// `include/constants/std_script.h`).
    pub script_id_lo: u16,
    /// The script bank (`a/0/1/2` member).
    pub script_bank: u16,
    /// The message bank (`a/0/2/7` member).
    pub msg_bank: u16,
}

/// The `_std_*` thresholds (`include/constants/std_script.h`) — the
/// script-id groups that map to fixed banks.
pub mod std_script {
    /// `_std_misc` — the shared std scripts (signposts, item receipt,
    /// mart, PC, ...), bank 3.
    pub const MISC: u16 = 2000;
    /// `_std_bookshelves`.
    pub const BOOKSHELVES: u16 = 2500;
    /// `_std_apricorn_tree`.
    pub const APRICORN_TREE: u16 = 2800;
    /// `_std_npc_trainer`.
    pub const NPC_TRAINER: u16 = 3000;
    /// `_std_npc_trainer_2`.
    pub const NPC_TRAINER_2: u16 = 5000;
    /// `_std_item_ball`.
    pub const ITEM_BALL: u16 = 7000;
    /// `_std_hidden_item`.
    pub const HIDDEN_ITEM: u16 = 8000;
    /// `_std_safari`.
    pub const SAFARI: u16 = 8800;
    /// `_std_chatot`.
    pub const CHATOT: u16 = 8900;
    /// `_std_comm_reception`.
    pub const COMM_RECEPTION: u16 = 9000;
    /// `_std_colosseum`.
    pub const COLOSSEUM: u16 = 9100;
    /// `_std_wifi_reception`.
    pub const WIFI_RECEPTION: u16 = 9200;
    /// `_std_group`.
    pub const GROUP: u16 = 9300;
    /// `_std_daycare`.
    pub const DAYCARE: u16 = 9500;
    /// `_std_init` — the new-game init script (`RunInitScript`,
    /// `src/script_manager.c:576`): bank 149, index 0.
    pub const INIT: u16 = 9600;
    /// `_std_following_mon`.
    pub const FOLLOWING_MON: u16 = 9700;
    /// `_std_pokeathlon`.
    pub const POKEATHLON: u16 = 9850;
    /// `_std_dex_evaluation`.
    pub const DEX_EVALUATION: u16 = 9950;
    /// `_std_field_move`.
    pub const FIELD_MOVE: u16 = 10000;
    /// `_std_tv`.
    pub const TV: u16 = 10100;
    /// `_std_mystery_gift`.
    pub const MYSTERY_GIFT: u16 = 10200;
    /// `_std_trainer_house`.
    pub const TRAINER_HOUSE: u16 = 10350;
    /// `_std_bug_contest`.
    pub const BUG_CONTEST: u16 = 10400;
    /// `_std_frontier_move_tutor`.
    pub const FRONTIER_MOVE_TUTOR: u16 = 10440;
    /// `_std_frontier_records`.
    pub const FRONTIER_RECORDS: u16 = 10450;
    /// `_std_scratch_card`.
    pub const SCRATCH_CARD: u16 = 10490;

    // The named std_misc scripts the early game calls (`CallStd`).
    /// `std_signpost`.
    pub const SIGNPOST: u16 = 2000;
    /// `std_obtain_item_verbose`.
    pub const OBTAIN_ITEM_VERBOSE: u16 = 2008;
    /// `std_bag_is_full`.
    pub const BAG_IS_FULL: u16 = 2009;
    /// `std_play_friend_music`.
    pub const PLAY_FRIEND_MUSIC: u16 = 2029;
    /// `std_fade_end_friend_music`.
    pub const FADE_END_FRIEND_MUSIC: u16 = 2030;
    /// `std_give_item_verbose`.
    pub const GIVE_ITEM_VERBOSE: u16 = 2033;
    /// `std_play_mom_music`.
    pub const PLAY_MOM_MUSIC: u16 = 2036;
    /// `std_fade_end_mom_music`.
    pub const FADE_END_MOM_MUSIC: u16 = 2038;
}

/// `sScriptBankMapping` (`src/script_manager.c:33-64`), in the
/// original's scan order (highest threshold first). The bank numbers
/// are the `NARC_scr_seq_scr_seq_NNNN_bin` / `NARC_msg_msg_NNNN_bin`
/// member ids.
pub const STD_BANK_MAPPING: [BankMapping; 30] = {
    const fn row(script_id_lo: u16, script_bank: u16, msg_bank: u16) -> BankMapping {
        BankMapping {
            script_id_lo,
            script_bank,
            msg_bank,
        }
    }
    [
        row(std_script::SCRATCH_CARD, 263, 433),
        row(std_script::FRONTIER_RECORDS, 264, 19),
        row(std_script::FRONTIER_MOVE_TUTOR, 2, 748),
        row(std_script::BUG_CONTEST, 151, 246),
        row(std_script::TRAINER_HOUSE, 952, 726),
        row(10300, 734, 444),
        row(std_script::MYSTERY_GIFT, 144, 209),
        row(10150, 955, 732),
        row(std_script::TV, 954, 733),
        row(std_script::FIELD_MOVE, 146, 211),
        row(std_script::DEX_EVALUATION, 148, 666),
        row(9900, 136, 40),
        row(std_script::POKEATHLON, 167, 312),
        row(9800, 166, 43),
        row(std_script::FOLLOWING_MON, 163, 266),
        row(std_script::INIT, 149, 40),
        row(std_script::DAYCARE, 265, 439),
        row(std_script::GROUP, 143, 204),
        row(std_script::WIFI_RECEPTION, 164, 267),
        row(std_script::COLOSSEUM, 0, 14),
        row(std_script::COMM_RECEPTION, 4, 46),
        row(std_script::CHATOT, 165, 268),
        row(std_script::SAFARI, 262, 427),
        row(std_script::HIDDEN_ITEM, 145, 210),
        row(std_script::ITEM_BALL, 141, 199),
        row(std_script::NPC_TRAINER_2, 953, 40),
        row(std_script::NPC_TRAINER, 953, 40),
        row(std_script::APRICORN_TREE, 150, 23),
        row(std_script::BOOKSHELVES, 1, 20),
        row(std_script::MISC, 3, 40),
    ]
};

/// The banks script id 0 loads (`LoadScriptsAndMessagesByMapId`'s
/// fallback: `scr_seq_0140`, `msg_0184`).
pub const EVERYWHERE_BANKS: MapBanks = MapBanks {
    scripts: 140,
    messages: 184,
};

/// Where a script id resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedScript {
    /// The script bank to load.
    pub script_bank: u16,
    /// The message bank to load.
    pub msg_bank: u16,
    /// The index into the bank's entry table (`ScriptRunByIndex`).
    pub index: u16,
}

/// `LoadScriptsAndMessagesByMapId` (`src/script_manager.c:194`):
/// resolves `script_id` to a bank pair and entry index, `current_map`
/// being the map header's own banks for ids `1..2000`.
#[must_use]
pub fn resolve_script(script_id: u16, current_map: MapBanks) -> ResolvedScript {
    for row in &STD_BANK_MAPPING {
        if script_id >= row.script_id_lo {
            return ResolvedScript {
                script_bank: row.script_bank,
                msg_bank: row.msg_bank,
                index: script_id - row.script_id_lo,
            };
        }
    }
    if script_id >= 1 {
        ResolvedScript {
            script_bank: current_map.scripts,
            msg_bank: current_map.messages,
            index: script_id - 1,
        }
    } else {
        ResolvedScript {
            script_bank: EVERYWHERE_BANKS.scripts,
            msg_bank: EVERYWHERE_BANKS.messages,
            index: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assembles an entry table over `scripts` (each a byte slice) the
    /// way `ScrDef`/`ScrDefEnd` do, concatenating the scripts after it.
    fn assemble(scripts: &[&[u8]]) -> Vec<u8> {
        let table_len = scripts.len() * 4 + 2;
        let mut out = Vec::new();
        let mut at = table_len;
        for (i, s) in scripts.iter().enumerate() {
            let after_word = i * 4 + 4;
            out.extend_from_slice(&((at - after_word) as u32).to_le_bytes());
            at += s.len();
        }
        out.extend_from_slice(&SCRDEF_END.to_le_bytes());
        for s in scripts {
            out.extend_from_slice(s);
        }
        out
    }

    #[test]
    fn entry_table_resolves_relative_to_after_each_word() {
        let bank = ScriptBank::parse(assemble(&[&[2, 0], &[30, 0, 1, 0, 2, 0]])).unwrap();
        assert_eq!(bank.script_count(), 2);
        assert_eq!(bank.code_start(), 10);
        assert_eq!(bank.script_offset(0), Some(10));
        assert_eq!(bank.script_offset(1), Some(12));
        assert_eq!(bank.script(1).unwrap(), &[30, 0, 1, 0, 2, 0]);
        assert_eq!(bank.script(2), None);
        // Zero scripts: just the terminator.
        let empty = ScriptBank::parse(SCRDEF_END.to_le_bytes().to_vec()).unwrap();
        assert_eq!(empty.script_count(), 0);
        assert_eq!(empty.code_start(), 2);
    }

    #[test]
    fn malformed_tables_are_refused() {
        assert!(ScriptBank::parse(vec![]).is_err(), "no terminator");
        assert!(ScriptBank::parse(vec![0x10, 0]).is_err(), "table runs off");
        // An entry past the member end.
        let mut bad = assemble(&[&[2, 0]]);
        bad[0] = 0x40;
        assert!(matches!(
            ScriptBank::parse(bad),
            Err(ScriptError::BadBank { .. })
        ));
    }

    #[test]
    fn script_ids_map_to_banks_like_the_original() {
        let map = MapBanks {
            scripts: 846,
            messages: 546,
        };
        // _std_init → bank 149 / msg 40, index 0.
        assert_eq!(
            resolve_script(std_script::INIT, map),
            ResolvedScript {
                script_bank: 149,
                msg_bank: 40,
                index: 0
            }
        );
        // std_give_item_verbose is std_misc index 33.
        assert_eq!(
            resolve_script(std_script::GIVE_ITEM_VERBOSE, map),
            ResolvedScript {
                script_bank: 3,
                msg_bank: 40,
                index: 33
            }
        );
        // Trainer scripts: the two rows share bank 953 with different bases.
        assert_eq!(resolve_script(3005, map).index, 5);
        assert_eq!(resolve_script(5005, map).index, 5);
        assert_eq!(resolve_script(4999, map).index, 1999);
        // Map events: 1-based into the current map's banks.
        assert_eq!(
            resolve_script(1, map),
            ResolvedScript {
                script_bank: 846,
                msg_bank: 546,
                index: 0
            }
        );
        assert_eq!(resolve_script(1999, map).index, 1998);
        // Script 0: the everywhere pair.
        assert_eq!(
            resolve_script(0, map),
            ResolvedScript {
                script_bank: 140,
                msg_bank: 184,
                index: 0
            }
        );
        // The table is in descending threshold order, as the scan needs.
        assert!(
            STD_BANK_MAPPING
                .windows(2)
                .all(|w| w[0].script_id_lo >= w[1].script_id_lo)
        );
    }
}
