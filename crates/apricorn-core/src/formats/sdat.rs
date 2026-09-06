//! SDAT — the sound data archive: the ROM's whole audio index.
//!
//! Both of HeartGold's SDATs (`data/sound/gs_sound_data.sdat`, the main
//! archive, and `pbr/sound_data.sdat`) are pure containers: an allocation
//! table of SSEQ sequences, SBNK banks, and SWAR sample archives, plus the
//! INFO/SYMB metadata that names and cross-references them. Parsing the
//! records those files contain (SSEQ programs, SWAR wave banks) is Phase 7
//! work; this module resolves archive structure only.
//!
//! # Layout
//!
//! The container opens with the SDK's 0x10-byte `SNDBinaryFileHeader`
//! extended to 0x40 bytes, ending in four `(offset, size)` pairs (offsets
//! from SDAT start):
//!
//! ```text
//! 0x00 4   magic "SDAT"
//! 0x04 2   byte-order mark 0xFEFF
//! 0x06 2   version (0x0100 in HeartGold)
//! 0x08 4   total file size
//! 0x0C 2   header size (0x40)
//! 0x0E 2   block count (4 with a SYMB block, 3 without)
//! 0x10 4+4 symbol block (SYMB) offset + size, or (0, 0)
//! 0x18 4+4 info block (INFO) offset + size
//! 0x20 4+4 allocation block (FAT ) offset + size
//! 0x28 4+4 file image (FILE) offset + size
//! 0x30 0x10 reserved (zero)
//! ```
//!
//! The blocks then chain by their **own** 8-byte headers (`magic`,
//! `u32 size`) starting at 0x40: own sizes are 4-byte aligned and may
//! exceed the header's pair by up to 3 bytes of padding (the main
//! archive's SYMB owns 0xD7D4 against a pair of 0xD7D1). The FILE block
//! covers everything to EOF.
//!
//! - **SYMB** — the symbol block: eight sub-lists of label offsets (seq,
//!   seqArc, bank, waveArc, player, group, strmPlayer, strm) tiling
//!   back-to-back exactly from 0x40, then one shared string pool. Label
//!   offsets are relative to the block start; 0 means unnamed.
//! - **INFO** — the info block: the same eight sub-lists, but each holds
//!   `{u32 count, u32 record_offset[count]}` (0 = absent record, offsets
//!   relative to the block start), and the records interleave in the gap
//!   between one list's offsets and the next list's start. Record
//!   layouts follow GBATEK and `nnsys.s`: SEQ `{u16 file, u16 -, u16
//!   bank, u8 vol, u8 channel_prio, u8 player_prio, u8 player, u16 -}`,
//!   SEQARC `{u16 file, u16 -}`, BANK `{u16 file, u16 -, u16 swar[4]}`
//!   (0xFFFF = slot unused), SWAR `{u16 file, u16 -}`, PLAYER `{u8
//!   seq_count, u8 -, u16 channels, u32 heap}`, GROUP `{u32 count,
//!   item[count]}` with each item `{u8 type, u8 flags, u16 -, u32 index}`
//!   — the encoding `NNSi_SndArcLoadGroup`'s jump table reads: type 0 =
//!   SEQ, 1 = BANK, 2 = WAVEARC, 3 = SEQARC — plus STRMPLAYER (0x18
//!   bytes, parsed structurally but not exposed) and STRM `{u16 file,
//!   u16 -, u8 vol, u8 pri, u8 player, u8 -[5]}`.
//! - **FAT ** — `{magic, u32 size, u32 count, entry[count]}` where each
//!   entry is `{u32 offset, u32 size, u32 mem, u32 reserved}` with
//!   absolute SDAT offsets; mem/reserved are zero and `(0, 0)` = empty.
//! - **FILE** — `{magic, u32 size, u32 count, 12 reserved bytes}`, then
//!   the file data from +0x18. `count` equals the FAT count.
//!
//! In a retail archive a named SYMB entry is exactly a present INFO
//! record, and every SEQ/BANK/SWAR record points at a file of its own
//! magic — invariants this parser enforces. Retail HeartGold (US): the
//! main archive holds 2,353 files (1,231 SSEQ + 561 SBNK + 561 SWAR),
//! `pbr/sound_data.sdat` 1,846 (812/517/517); neither contains STRM
//! streams — all audio is SSEQ sequences played through SBNK/SWAR. See
//! `docs/nitro-sdat.md` and the integration tests for the worked ground
//! truth.

use crate::nds::{NdsError, u16le, u32le};

/// The number of SYMB/INFO sub-lists (seq, seqArc, bank, waveArc, player,
/// group, strmPlayer, strm).
const LISTS: usize = 8;

/// Sub-list index of the SEQ records.
const SEQ: usize = 0;
/// Sub-list index of the SEQARC records.
const SEQ_ARC: usize = 1;
/// Sub-list index of the BANK records.
const BANK: usize = 2;
/// Sub-list index of the WAVEARC records.
const WAVE_ARC: usize = 3;
/// Sub-list index of the PLAYER records.
const PLAYER: usize = 4;
/// Sub-list index of the GROUP records.
const GROUP: usize = 5;
/// Sub-list index of the STRM records. (Index 6 is the STRMPLAYER
/// records, which are validated structurally only and never exposed.)
const STRM: usize = 7;

/// Fixed INFO record sizes, by sub-list. The GROUP record (index 5) is
/// variable-length; the STRMPLAYER record is validated structurally only.
const RECORD_SIZES: [usize; LISTS] = [12, 4, 12, 4, 8, 0, 0x18, 12];

/// A SEQ info record: one SSEQ sequence and how to play it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SseqInfo {
    /// The FAT file id of the SSEQ program.
    pub file_id: u16,
    /// The SBNK bank the sequence plays through. Never 0xFFFF in
    /// retail, though the SDK allows it.
    pub bank: u16,
    /// Playback volume (0..=127, `SND_SetupPlayerSeq`'s default).
    pub volume: u8,
    /// Channel priority (0..=127).
    pub channel_priority: u8,
    /// Player priority (0..=127).
    pub player_priority: u8,
    /// The player (0..7 in retail) that performs the sequence.
    pub player: u8,
}

/// A SEQARC info record: one `SSAR` sequence archive file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqArcInfo {
    /// The FAT file id of the SSAR archive.
    pub file_id: u16,
}

/// A BANK info record: an `SBNK` and the sample archives it draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankInfo {
    /// The FAT file id of the SBNK bank.
    pub file_id: u16,
    /// Up to four WAVEARC indices the bank's instruments sample from;
    /// 0xFFFF = slot unused.
    pub swar: [u16; 4],
}

/// A WAVEARC info record: one `SWAR` sample archive file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwarInfo {
    /// The FAT file id of the SWAR archive.
    pub file_id: u16,
}

/// A PLAYER info record: the hardware allocation of one of the game's
/// named players (`PLAYER_PV`, `PLAYER_BGM`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerInfo {
    /// How many sequences may play simultaneously
    /// (`SetPlayableSeqCount`).
    pub seq_count: u8,
    /// Which of the 16 hardware channels the player may allocate
    /// (`SetAllocatableChannel`), as a bit flag.
    pub channels: u16,
    /// The player's note-heap size in bytes (`CreateHeap`).
    pub heap_size: u32,
}

/// What a GROUP item loads; the cases of `NNSi_SndArcLoadGroup`'s jump
/// table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupItemKind {
    /// Load a SEQ (type 0, `NNSi_SndArcLoadSeq`).
    Seq,
    /// Load a BANK (type 1, `NNSi_SndArcLoadBank`).
    Bank,
    /// Load a WAVEARC (type 2, `NNSi_SndArcLoadWaveArc`).
    WaveArc,
    /// Load a SEQARC (type 3, `NNSi_SndArcLoadSeqArc`).
    SeqArc,
}

impl GroupItemKind {
    /// Decodes the type byte of a group item.
    ///
    /// # Errors
    /// Returns an [`NdsError`] for any type above 3.
    fn from_raw(raw: u8) -> Result<Self, NdsError> {
        match raw {
            0 => Ok(Self::Seq),
            1 => Ok(Self::Bank),
            2 => Ok(Self::WaveArc),
            3 => Ok(Self::SeqArc),
            _ => Err(NdsError::Invalid {
                what: "SDAT group item type",
            }),
        }
    }

    /// The type byte as stored in the group item.
    fn raw(self) -> u8 {
        match self {
            Self::Seq => 0,
            Self::Bank => 1,
            Self::WaveArc => 2,
            Self::SeqArc => 3,
        }
    }
}

/// One member of a load GROUP (`GROUP_GLOBAL`, `GROUP_SE_FIELD`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupItem {
    /// What to load.
    pub kind: GroupItemKind,
    /// The flag argument of the `NNSi_SndArcLoad*` call (7 in retail,
    /// except one `pbr` item using 6).
    pub flags: u8,
    /// The index of the entry to load, in the sub-list named by `kind`.
    pub index: u32,
}

/// A STRM info record: one streamed `STRM` file. HeartGold's archives
/// carry none (count 0) — the layout follows GBATEK, verified by
/// synthetic test only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrmInfo {
    /// The FAT file id of the STRM stream.
    pub file_id: u16,
    /// Playback volume.
    pub volume: u8,
    /// Playback priority.
    pub priority: u8,
    /// The player that performs the stream.
    pub player: u8,
}

/// Whether `data` begins with an SDAT header.
#[must_use]
pub fn is_sdat(data: &[u8]) -> bool {
    data.get(0..6) == Some(&[b'S', b'D', b'A', b'T', 0xFF, 0xFE])
}

/// One SYMB/INFO sub-list: the symbol per entry (when the archive has a
/// SYMB block) and the parsed INFO record per entry.
#[derive(Debug)]
struct SdatList<'a, T> {
    /// The SYMB label per entry; empty when the archive has no SYMB.
    names: Vec<Option<&'a str>>,
    /// The parsed INFO record per entry; `None` when absent.
    records: Vec<Option<T>>,
}

impl<'a, T> SdatList<'a, T> {
    /// The SYMB label of entry `id`, if the archive names it.
    fn name(&self, id: usize) -> Option<&'a str> {
        self.names.get(id).copied().flatten()
    }
}

/// A parsed SDAT archive. Borrows the archive bytes; see [`Sdat::parse`].
///
/// All structure — header, blocks, symbol tables, info records, and the
/// allocation table — is validated at parse time, including the retail
/// invariant that a named entry is exactly a present INFO record.
#[derive(Debug)]
pub struct Sdat<'a> {
    /// The raw archive (header, blocks, and file image).
    data: &'a [u8],
    /// FAT entries as `(offset, size)` pairs; size 0 = empty.
    files: Vec<(u32, u32)>,
    /// Whether the archive carries a SYMB block.
    has_symbols: bool,
    seq: SdatList<'a, SseqInfo>,
    seq_arc: SdatList<'a, SeqArcInfo>,
    bank: SdatList<'a, BankInfo>,
    wave_arc: SdatList<'a, SwarInfo>,
    player: SdatList<'a, PlayerInfo>,
    group: SdatList<'a, Vec<GroupItem>>,
    strm: SdatList<'a, StrmInfo>,
}

impl<'a> Sdat<'a> {
    /// Parses a complete SDAT archive.
    ///
    /// # Errors
    /// Returns an [`NdsError`] if the header, the block chain, the
    /// symbol or info tables, or the allocation table are truncated,
    /// inconsistent, or violate the retail invariants (named entries
    /// matching present records, records pointing at files of the right
    /// magic).
    pub fn parse(data: &'a [u8]) -> Result<Self, NdsError> {
        // ---- header ----
        if data.len() < 0x40 {
            return Err(NdsError::Truncated {
                what: "SDAT header",
                need: 0x40,
                got: data.len(),
            });
        }
        if &data[0..4] != b"SDAT" {
            return Err(NdsError::Invalid {
                what: "not an SDAT archive",
            });
        }
        if u16le(data, 0x04)? != 0xFEFF {
            return Err(NdsError::Invalid {
                what: "SDAT byte-order mark",
            });
        }
        if u16le(data, 0x06)? != 0x0100 {
            return Err(NdsError::Invalid {
                what: "SDAT version (only 0x0100 is supported)",
            });
        }
        let file_size = u32le(data, 0x08)? as usize;
        if file_size != data.len() {
            return Err(NdsError::Invalid {
                what: "SDAT file size does not match its data",
            });
        }
        if u16le(data, 0x0C)? != 0x40 {
            return Err(NdsError::Invalid {
                what: "SDAT header size",
            });
        }
        let block_count = usize::from(u16le(data, 0x0E)?);
        if data[0x30..0x40].iter().any(|&b| b != 0) {
            return Err(NdsError::Invalid {
                what: "SDAT header padding",
            });
        }
        let mut pairs = [(0usize, 0usize); 4];
        for (i, pair) in pairs.iter_mut().enumerate() {
            *pair = (
                u32le(data, 0x10 + 8 * i)? as usize,
                u32le(data, 0x14 + 8 * i)? as usize,
            );
        }
        let has_symbols = pairs[0] != (0, 0);
        if block_count != if has_symbols { 4 } else { 3 } {
            return Err(NdsError::Invalid {
                what: "SDAT block count disagrees with the symbol block",
            });
        }

        // ---- block walk: blocks start at the header end and chain by
        // their own sizes ----
        let mut off = 0x40;
        let mut symb_block = None;
        if has_symbols {
            let own = block_header(data, off, pairs[0], b"SYMB")?;
            symb_block = Some((off, own));
            off += own;
        }
        let info_own = block_header(data, off, pairs[1], b"INFO")?;
        let info_off = off;
        off += info_own;
        let fat_own = block_header(data, off, pairs[2], b"FAT ")?;
        let fat_off = off;
        off += fat_own;
        let file_own = block_header(data, off, pairs[3], b"FILE")?;
        let file_off = off;
        // The FILE block covers the rest of the archive exactly.
        if pairs[3].1 != file_own || file_off + file_own != data.len() {
            return Err(NdsError::Invalid {
                what: "SDAT file image does not cover the archive",
            });
        }
        if data
            .get(file_off + 0xC..file_off + 0x18)
            .is_none_or(|r| r.iter().any(|&b| b != 0))
        {
            return Err(NdsError::Invalid {
                what: "SDAT FILE header padding",
            });
        }
        let file_body = file_off + 0x18;

        // ---- FAT: size must be exactly 12 + 16*count, and every file
        // lies 4-byte aligned inside the image, in ascending order ----
        if fat_own < 12 {
            return Err(NdsError::Truncated {
                what: "SDAT FAT block",
                need: 12,
                got: fat_own,
            });
        }
        let fat_count = u32le(data, fat_off + 8)? as usize;
        if fat_own != 12 + fat_count * 16 {
            return Err(NdsError::Invalid {
                what: "SDAT FAT block size disagrees with the file count",
            });
        }
        if u32le(data, file_off + 8)? as usize != fat_count {
            return Err(NdsError::Invalid {
                what: "SDAT FILE count disagrees with the FAT count",
            });
        }
        let mut files = Vec::with_capacity(fat_count);
        let mut prev_end = file_body;
        for i in 0..fat_count {
            let o = fat_off + 12 + 16 * i;
            let offset = u32le(data, o)? as usize;
            let size = u32le(data, o + 4)? as usize;
            if u32le(data, o + 8)? != 0 || u32le(data, o + 12)? != 0 {
                return Err(NdsError::Invalid {
                    what: "SDAT FAT entry has a nonzero mem/reserved field",
                });
            }
            if offset == 0 && size == 0 {
                files.push((0, 0));
                continue;
            }
            let end = offset.checked_add(size);
            if !offset.is_multiple_of(4) || offset < file_body || end.is_none_or(|e| e > data.len())
            {
                return Err(NdsError::Invalid {
                    what: "SDAT FAT entry lies outside the file image",
                });
            }
            if offset < prev_end {
                return Err(NdsError::Invalid {
                    what: "SDAT FAT entries are not in ascending order",
                });
            }
            prev_end = offset + size;
            files.push((offset as u32, size as u32));
        }

        // ---- SYMB: eight label sub-lists tiling back-to-back from 0x40,
        // then one shared string pool ----
        let mut names: [Vec<Option<&'a str>>; LISTS] = Default::default();
        if let Some((sym_off, sym_own)) = symb_block {
            names = parse_symbols(data, sym_off, sym_own)?;
        }

        // ---- INFO: list arrays at each rel, records in the gaps ----
        let (info_rels, info) = parse_info_lists(data, info_off, info_own)?;
        let mut region_ends = [0usize; LISTS];
        for i in 0..LISTS {
            region_ends[i] = if i + 1 < LISTS {
                info_rels[i + 1] as usize
            } else {
                info_own
            };
        }
        for i in 0..LISTS {
            let array_end = info_rels[i] as usize + 4 + 4 * info[i].offsets.len();
            for &o in &info[i].offsets {
                if o == 0 {
                    continue;
                }
                let rel = o as usize;
                if rel < array_end {
                    return Err(NdsError::Invalid {
                        what: "SDAT INFO record sits inside its offsets array",
                    });
                }
                let size = RECORD_SIZES[i];
                if size > 0 && rel + size > region_ends[i] {
                    return Err(NdsError::Invalid {
                        what: "SDAT INFO record overruns its region",
                    });
                }
            }
        }

        // ---- records ----
        let seq_count = info[SEQ].offsets.len();
        let seq_arc_count = info[SEQ_ARC].offsets.len();
        let bank_count = info[BANK].offsets.len();
        let wave_arc_count = info[WAVE_ARC].offsets.len();
        let player_count = info[PLAYER].offsets.len();

        let mut seq_records = Vec::with_capacity(seq_count);
        for &o in &info[SEQ].offsets {
            seq_records.push(if o == 0 {
                None
            } else {
                Some(parse_seq_record(
                    data,
                    info_off,
                    o as usize,
                    &files,
                    bank_count,
                    player_count,
                )?)
            });
        }
        let mut seq_arc_records = Vec::with_capacity(seq_arc_count);
        for &o in &info[SEQ_ARC].offsets {
            seq_arc_records.push(if o == 0 {
                None
            } else {
                Some(parse_seq_arc_record(data, info_off, o as usize, &files)?)
            });
        }
        let mut bank_records = Vec::with_capacity(bank_count);
        for &o in &info[BANK].offsets {
            bank_records.push(if o == 0 {
                None
            } else {
                Some(parse_bank_record(
                    data,
                    info_off,
                    o as usize,
                    &files,
                    wave_arc_count,
                )?)
            });
        }
        let mut wave_arc_records = Vec::with_capacity(wave_arc_count);
        for &o in &info[WAVE_ARC].offsets {
            wave_arc_records.push(if o == 0 {
                None
            } else {
                Some(parse_swar_record(data, info_off, o as usize, &files)?)
            });
        }
        let mut player_records = Vec::with_capacity(player_count);
        for &o in &info[PLAYER].offsets {
            player_records.push(if o == 0 {
                None
            } else {
                Some(parse_player_record(data, info_off, o as usize)?)
            });
        }
        let mut group_records = Vec::with_capacity(info[GROUP].offsets.len());
        // Per-kind entry counts, indexed by the group item type byte.
        let limits = [seq_count, bank_count, wave_arc_count, seq_arc_count];
        for &o in &info[GROUP].offsets {
            group_records.push(if o == 0 {
                None
            } else {
                Some(parse_group_record(
                    data,
                    info_off,
                    o as usize,
                    region_ends[GROUP],
                    &limits,
                )?)
            });
        }
        // STRMPLAYER records are structural only: their 0x18-byte size is
        // validated above (and their names enter the consistency check via
        // the local arrays); HeartGold never carries any, so nothing is kept.
        let mut strm_records = Vec::with_capacity(info[STRM].offsets.len());
        for &o in &info[STRM].offsets {
            strm_records.push(if o == 0 {
                None
            } else {
                Some(parse_strm_record(data, info_off, o as usize, &files)?)
            });
        }

        // ---- SYMB/INFO consistency: counts match, and a named entry is
        // exactly a present record ----
        if has_symbols {
            for i in 0..LISTS {
                if names[i].len() != info[i].offsets.len() {
                    return Err(NdsError::Invalid {
                        what: "SDAT symbol and info list counts disagree",
                    });
                }
                for (label, offset) in names[i].iter().zip(&info[i].offsets) {
                    if label.is_some() != (*offset != 0) {
                        return Err(NdsError::Invalid {
                            what: "SDAT symbol names an entry whose record is absent (or vice versa)",
                        });
                    }
                }
            }
        }

        let [
            seq_names,
            seq_arc_names,
            bank_names,
            wave_arc_names,
            player_names,
            group_names,
            _strm_player_names,
            strm_names,
        ] = names;
        Ok(Self {
            data,
            files,
            has_symbols,
            seq: SdatList {
                names: seq_names,
                records: seq_records,
            },
            seq_arc: SdatList {
                names: seq_arc_names,
                records: seq_arc_records,
            },
            bank: SdatList {
                names: bank_names,
                records: bank_records,
            },
            wave_arc: SdatList {
                names: wave_arc_names,
                records: wave_arc_records,
            },
            player: SdatList {
                names: player_names,
                records: player_records,
            },
            group: SdatList {
                names: group_names,
                records: group_records,
            },
            strm: SdatList {
                names: strm_names,
                records: strm_records,
            },
        })
    }

    /// The raw archive bytes.
    #[must_use]
    pub fn raw(&self) -> &'a [u8] {
        self.data
    }

    /// Whether the archive carries a SYMB symbol block (both retail
    /// archives do).
    #[must_use]
    pub fn has_symbols(&self) -> bool {
        self.has_symbols
    }

    /// The number of files in the archive's FAT.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// The bytes of FAT file `id`, or `None` for an empty entry or an
    /// out-of-range id.
    #[must_use]
    pub fn file(&self, id: usize) -> Option<&'a [u8]> {
        let &(off, size) = self.files.get(id)?;
        if size == 0 {
            return None;
        }
        Some(&self.data[off as usize..off as usize + size as usize])
    }

    /// The number of SEQ entries (named and unnamed alike).
    #[must_use]
    pub fn seq_count(&self) -> usize {
        self.seq.records.len()
    }

    /// The SEQ record of entry `id`, if its INFO record is present.
    #[must_use]
    pub fn seq(&self, id: usize) -> Option<SseqInfo> {
        self.seq.records.get(id).copied().flatten()
    }

    /// The SYMB label of SEQ entry `id`, if the archive names it.
    #[must_use]
    pub fn seq_name(&self, id: usize) -> Option<&'a str> {
        self.seq.name(id)
    }

    /// The number of SEQARC entries.
    #[must_use]
    pub fn seq_arc_count(&self) -> usize {
        self.seq_arc.records.len()
    }

    /// The SEQARC record of entry `id`, if present.
    #[must_use]
    pub fn seq_arc(&self, id: usize) -> Option<SeqArcInfo> {
        self.seq_arc.records.get(id).copied().flatten()
    }

    /// The SYMB label of SEQARC entry `id`, if named.
    #[must_use]
    pub fn seq_arc_name(&self, id: usize) -> Option<&'a str> {
        self.seq_arc.name(id)
    }

    /// The number of BANK entries.
    #[must_use]
    pub fn bank_count(&self) -> usize {
        self.bank.records.len()
    }

    /// The BANK record of entry `id`, if present.
    #[must_use]
    pub fn bank(&self, id: usize) -> Option<BankInfo> {
        self.bank.records.get(id).copied().flatten()
    }

    /// The SYMB label of BANK entry `id`, if named.
    #[must_use]
    pub fn bank_name(&self, id: usize) -> Option<&'a str> {
        self.bank.name(id)
    }

    /// The number of WAVEARC entries.
    #[must_use]
    pub fn wave_arc_count(&self) -> usize {
        self.wave_arc.records.len()
    }

    /// The WAVEARC record of entry `id`, if present.
    #[must_use]
    pub fn wave_arc(&self, id: usize) -> Option<SwarInfo> {
        self.wave_arc.records.get(id).copied().flatten()
    }

    /// The SYMB label of WAVEARC entry `id`, if named.
    #[must_use]
    pub fn wave_arc_name(&self, id: usize) -> Option<&'a str> {
        self.wave_arc.name(id)
    }

    /// The number of PLAYER entries.
    #[must_use]
    pub fn player_count(&self) -> usize {
        self.player.records.len()
    }

    /// The PLAYER record of entry `id`, if present.
    #[must_use]
    pub fn player(&self, id: usize) -> Option<PlayerInfo> {
        self.player.records.get(id).copied().flatten()
    }

    /// The SYMB label of PLAYER entry `id`, if named.
    #[must_use]
    pub fn player_name(&self, id: usize) -> Option<&'a str> {
        self.player.name(id)
    }

    /// The number of GROUP entries.
    #[must_use]
    pub fn group_count(&self) -> usize {
        self.group.records.len()
    }

    /// The items of GROUP entry `id`, if present.
    #[must_use]
    pub fn group(&self, id: usize) -> Option<&[GroupItem]> {
        self.group
            .records
            .get(id)
            .and_then(Option::as_ref)
            .map(Vec::as_slice)
    }

    /// The SYMB label of GROUP entry `id`, if named.
    #[must_use]
    pub fn group_name(&self, id: usize) -> Option<&'a str> {
        self.group.name(id)
    }

    /// The number of STRM entries (zero in HeartGold).
    #[must_use]
    pub fn strm_count(&self) -> usize {
        self.strm.records.len()
    }

    /// The STRM record of entry `id`, if present.
    #[must_use]
    pub fn strm(&self, id: usize) -> Option<StrmInfo> {
        self.strm.records.get(id).copied().flatten()
    }

    /// The SYMB label of STRM entry `id`, if named.
    #[must_use]
    pub fn strm_name(&self, id: usize) -> Option<&'a str> {
        self.strm.name(id)
    }
}

/// Checks one SDAT block header — magic, 4-byte alignment, and own size
/// against the header's offset/size pair (own may exceed the pair by up
/// to 3 bytes of padding) — and returns the block's own size.
fn block_header(
    data: &[u8],
    off: usize,
    pair: (usize, usize),
    magic: &[u8; 4],
) -> Result<usize, NdsError> {
    if data.get(off..off + 4) != Some(&magic[..]) {
        return Err(NdsError::Invalid {
            what: "SDAT block magic or order",
        });
    }
    let own = u32le(data, off + 4)? as usize;
    if !own.is_multiple_of(4) {
        return Err(NdsError::Invalid {
            what: "SDAT block size is not 4-byte aligned",
        });
    }
    if own < pair.1 || own > pair.1 + 3 {
        return Err(NdsError::Invalid {
            what: "SDAT block size disagrees with its header pair",
        });
    }
    if off.checked_add(own).is_none_or(|end| end > data.len()) {
        return Err(NdsError::Invalid {
            what: "SDAT block overruns the archive",
        });
    }
    Ok(own)
}

/// Parses the SYMB block into per-list labels.
///
/// The eight sub-lists tile back-to-back exactly from block offset 0x40,
/// the string pool follows the last one, and label offsets are relative
/// to the block start with 0 meaning unnamed. HeartGold archives carry
/// all eight sub-lists, and their seqArc sub-list is empty — folder
/// labels inside an SSAR are not supported.
fn parse_symbols<'a>(
    data: &'a [u8],
    off: usize,
    own: usize,
) -> Result<[Vec<Option<&'a str>>; LISTS], NdsError> {
    if data
        .get(off + 0x28..off + 0x40)
        .is_none_or(|r| r.iter().any(|&b| b != 0))
    {
        return Err(NdsError::Invalid {
            what: "SDAT SYMB header padding",
        });
    }
    let mut rels = [0u32; LISTS];
    for (i, rel) in rels.iter_mut().enumerate() {
        *rel = u32le(data, off + 8 + 4 * i)?;
    }
    if rels.contains(&0) {
        return Err(NdsError::Invalid {
            what: "SDAT SYMB sub-list is absent",
        });
    }
    let mut names: [Vec<Option<&'a str>>; LISTS] = Default::default();
    let mut cur = 0x40u32;
    for (i, &rel) in rels.iter().enumerate() {
        if rel != cur {
            return Err(NdsError::Invalid {
                what: "SDAT SYMB sub-lists do not tile back-to-back",
            });
        }
        let list = off + rel as usize;
        let count = u32le(data, list)? as usize;
        if i == SEQ_ARC && count > 0 {
            return Err(NdsError::Invalid {
                what: "SSAR folder symbol lists are not supported",
            });
        }
        let list_end = list + 4 + 4 * count;
        if list_end > off + own {
            return Err(NdsError::Truncated {
                what: "SDAT SYMB sub-list",
                need: list_end,
                got: off + own,
            });
        }
        let mut labels = Vec::with_capacity(count);
        for j in 0..count {
            let value = u32le(data, list + 4 + 4 * j)? as usize;
            if value == 0 {
                labels.push(None);
                continue;
            }
            if value >= own {
                return Err(NdsError::Invalid {
                    what: "SDAT symbol lies outside the SYMB block",
                });
            }
            let start = off + value;
            let nul =
                data[start..off + own]
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or(NdsError::Invalid {
                        what: "SDAT symbol is not NUL-terminated",
                    })?;
            let bytes = &data[start..start + nul];
            if bytes.is_empty() || bytes.iter().any(|&b| !(32..127).contains(&b)) {
                return Err(NdsError::Invalid {
                    what: "SDAT symbol is not printable ASCII",
                });
            }
            labels.push(Some(core::str::from_utf8(bytes).map_err(|_| {
                NdsError::Invalid {
                    what: "SDAT symbol is not ASCII",
                }
            })?));
        }
        names[i] = labels;
        cur = rel + 4 + 4 * count as u32;
    }
    Ok(names)
}

/// One INFO sub-list: `{u32 count, u32 record_offset[count]}` with
/// offsets relative to the block start (0 = absent record).
struct InfoList {
    /// The record offset per entry; 0 = absent.
    offsets: Vec<u32>,
}

/// Parses the INFO block's eight sub-list headers.
///
/// Unlike SYMB, the lists do not tile: each list's records interleave in
/// the gap between its offsets array and the next list's start (the
/// block end for the last). This validates the list order and that
/// present record offsets are strictly ascending.
fn parse_info_lists(
    data: &[u8],
    off: usize,
    own: usize,
) -> Result<([u32; LISTS], Vec<InfoList>), NdsError> {
    if data
        .get(off + 0x28..off + 0x40)
        .is_none_or(|r| r.iter().any(|&b| b != 0))
    {
        return Err(NdsError::Invalid {
            what: "SDAT INFO header padding",
        });
    }
    let mut rels = [0u32; LISTS];
    for (i, rel) in rels.iter_mut().enumerate() {
        *rel = u32le(data, off + 8 + 4 * i)?;
    }
    if rels.contains(&0) {
        return Err(NdsError::Invalid {
            what: "SDAT INFO sub-list is absent",
        });
    }
    if rels[0] != 0x40 || rels.windows(2).any(|w| w[0] >= w[1]) {
        return Err(NdsError::Invalid {
            what: "SDAT INFO sub-lists are out of order",
        });
    }
    let mut lists = Vec::with_capacity(LISTS);
    for &rel in rels.iter() {
        let list = off + rel as usize;
        // A count-0 list is just its own count word — seqArc, strmPlayer,
        // and strm are all empty in retail.
        if list + 4 > off + own {
            return Err(NdsError::Truncated {
                what: "SDAT INFO sub-list",
                need: list + 4,
                got: off + own,
            });
        }
        let count = u32le(data, list)? as usize;
        let array_end = list + 4 + 4 * count;
        if array_end > off + own {
            return Err(NdsError::Truncated {
                what: "SDAT INFO offsets array",
                need: array_end,
                got: off + own,
            });
        }
        let mut offsets = Vec::with_capacity(count);
        let mut prev = 0u32;
        for j in 0..count {
            let o = u32le(data, list + 4 + 4 * j)?;
            if o != 0 {
                if o <= prev {
                    return Err(NdsError::Invalid {
                        what: "SDAT INFO records are out of order",
                    });
                }
                prev = o;
            }
            offsets.push(o);
        }
        lists.push(InfoList { offsets });
    }
    Ok((rels, lists))
}

/// Checks that FAT file `id` exists and begins with the expected magic.
fn check_file(
    data: &[u8],
    files: &[(u32, u32)],
    id: u16,
    magic: &[u8; 4],
    what: &'static str,
) -> Result<(), NdsError> {
    let &(off, size) = files
        .get(usize::from(id))
        .ok_or(NdsError::Invalid { what })?;
    if size == 0 || data.get(off as usize..off as usize + 4) != Some(&magic[..]) {
        return Err(NdsError::Invalid { what });
    }
    Ok(())
}

/// Parses one SEQ record (12 bytes): `{u16 file, u16 -, u16 bank, u8
/// vol, u8 channel_prio, u8 player_prio, u8 player, u16 -}`.
fn parse_seq_record(
    data: &[u8],
    inf: usize,
    rel: usize,
    files: &[(u32, u32)],
    bank_count: usize,
    player_count: usize,
) -> Result<SseqInfo, NdsError> {
    let abs = inf + rel;
    let rec = data
        .get(abs..abs + RECORD_SIZES[SEQ])
        .ok_or(NdsError::Truncated {
            what: "SDAT SEQ record",
            need: abs + RECORD_SIZES[SEQ],
            got: data.len(),
        })?;
    let file_id = u16le(rec, 0)?;
    if u16le(rec, 2)? != 0 {
        return Err(NdsError::Invalid {
            what: "SDAT SEQ record reserved field",
        });
    }
    let bank = u16le(rec, 4)?;
    let volume = rec[6];
    let channel_priority = rec[7];
    let player_priority = rec[8];
    let player = rec[9];
    if u16le(rec, 10)? != 0 {
        return Err(NdsError::Invalid {
            what: "SDAT SEQ record reserved field",
        });
    }
    check_file(
        data,
        files,
        file_id,
        b"SSEQ",
        "SDAT SEQ record does not point at an SSEQ file",
    )?;
    if bank != 0xFFFF && usize::from(bank) >= bank_count {
        return Err(NdsError::Invalid {
            what: "SDAT SEQ record bank is out of range",
        });
    }
    if volume > 127 {
        return Err(NdsError::Invalid {
            what: "SDAT SEQ record volume exceeds 127",
        });
    }
    if usize::from(player) >= player_count {
        return Err(NdsError::Invalid {
            what: "SDAT SEQ record player is out of range",
        });
    }
    Ok(SseqInfo {
        file_id,
        bank,
        volume,
        channel_priority,
        player_priority,
        player,
    })
}

/// Parses one SEQARC record (4 bytes): `{u16 file, u16 -}`.
fn parse_seq_arc_record(
    data: &[u8],
    inf: usize,
    rel: usize,
    files: &[(u32, u32)],
) -> Result<SeqArcInfo, NdsError> {
    let abs = inf + rel;
    let rec = data
        .get(abs..abs + RECORD_SIZES[SEQ_ARC])
        .ok_or(NdsError::Truncated {
            what: "SDAT SEQARC record",
            need: abs + RECORD_SIZES[SEQ_ARC],
            got: data.len(),
        })?;
    let file_id = u16le(rec, 0)?;
    if u16le(rec, 2)? != 0 {
        return Err(NdsError::Invalid {
            what: "SDAT SEQARC record reserved field",
        });
    }
    check_file(
        data,
        files,
        file_id,
        b"SSAR",
        "SDAT SEQARC record does not point at an SSAR file",
    )?;
    Ok(SeqArcInfo { file_id })
}

/// Parses one BANK record (12 bytes): `{u16 file, u16 -, u16 swar[4]}`.
fn parse_bank_record(
    data: &[u8],
    inf: usize,
    rel: usize,
    files: &[(u32, u32)],
    wave_arc_count: usize,
) -> Result<BankInfo, NdsError> {
    let abs = inf + rel;
    let rec = data
        .get(abs..abs + RECORD_SIZES[BANK])
        .ok_or(NdsError::Truncated {
            what: "SDAT BANK record",
            need: abs + RECORD_SIZES[BANK],
            got: data.len(),
        })?;
    let file_id = u16le(rec, 0)?;
    if u16le(rec, 2)? != 0 {
        return Err(NdsError::Invalid {
            what: "SDAT BANK record reserved field",
        });
    }
    let mut swar = [0u16; 4];
    for (k, slot) in swar.iter_mut().enumerate() {
        *slot = u16le(rec, 4 + 2 * k)?;
        if *slot != 0xFFFF && usize::from(*slot) >= wave_arc_count {
            return Err(NdsError::Invalid {
                what: "SDAT BANK record swar slot is out of range",
            });
        }
    }
    check_file(
        data,
        files,
        file_id,
        b"SBNK",
        "SDAT BANK record does not point at an SBNK file",
    )?;
    Ok(BankInfo { file_id, swar })
}

/// Parses one WAVEARC record (4 bytes): `{u16 file, u16 -}`.
fn parse_swar_record(
    data: &[u8],
    inf: usize,
    rel: usize,
    files: &[(u32, u32)],
) -> Result<SwarInfo, NdsError> {
    let abs = inf + rel;
    let rec = data
        .get(abs..abs + RECORD_SIZES[WAVE_ARC])
        .ok_or(NdsError::Truncated {
            what: "SDAT WAVEARC record",
            need: abs + RECORD_SIZES[WAVE_ARC],
            got: data.len(),
        })?;
    let file_id = u16le(rec, 0)?;
    if u16le(rec, 2)? != 0 {
        return Err(NdsError::Invalid {
            what: "SDAT WAVEARC record reserved field",
        });
    }
    check_file(
        data,
        files,
        file_id,
        b"SWAR",
        "SDAT WAVEARC record does not point at a SWAR file",
    )?;
    Ok(SwarInfo { file_id })
}

/// Parses one PLAYER record (8 bytes): `{u8 seq_count, u8 -, u16
/// channels, u32 heap}`.
fn parse_player_record(data: &[u8], inf: usize, rel: usize) -> Result<PlayerInfo, NdsError> {
    let abs = inf + rel;
    let rec = data
        .get(abs..abs + RECORD_SIZES[PLAYER])
        .ok_or(NdsError::Truncated {
            what: "SDAT PLAYER record",
            need: abs + RECORD_SIZES[PLAYER],
            got: data.len(),
        })?;
    let seq_count = rec[0];
    if rec[1] != 0 {
        return Err(NdsError::Invalid {
            what: "SDAT PLAYER record reserved field",
        });
    }
    let channels = u16le(rec, 2)?;
    let heap_size = u32le(rec, 4)?;
    Ok(PlayerInfo {
        seq_count,
        channels,
        heap_size,
    })
}

/// Parses one GROUP record: `{u32 count, item[count]}` with each item
/// `{u8 type, u8 flags, u16 -, u32 index}`.
///
/// `end` is the record region's end (block-relative) — the next list's
/// start or the block end — bounding the variable-length body.
/// `limits` is the per-kind entry count, indexed by the item type byte:
/// seq, bank, waveArc, seqArc.
fn parse_group_record(
    data: &[u8],
    inf: usize,
    rel: usize,
    end: usize,
    limits: &[usize; 4],
) -> Result<Vec<GroupItem>, NdsError> {
    let abs = inf + rel;
    let count = u32le(data, abs)? as usize;
    if rel + 4 + 8 * count > end {
        return Err(NdsError::Invalid {
            what: "SDAT GROUP record overruns its region",
        });
    }
    let mut items = Vec::with_capacity(count);
    for j in 0..count {
        let a = abs + 4 + 8 * j;
        let kind = GroupItemKind::from_raw(data.get(a).copied().ok_or(NdsError::Invalid {
            what: "SDAT GROUP item",
        })?)?;
        let flags = data.get(a + 1).copied().unwrap_or(0);
        if u16le(data, a + 2)? != 0 {
            return Err(NdsError::Invalid {
                what: "SDAT GROUP item reserved field",
            });
        }
        let index = u32le(data, a + 4)?;
        if (index as usize) >= limits[usize::from(kind.raw())] {
            return Err(NdsError::Invalid {
                what: "SDAT GROUP item index is out of range",
            });
        }
        items.push(GroupItem { kind, flags, index });
    }
    Ok(items)
}

/// Parses one STRM record (12 bytes): `{u16 file, u16 -, u8 vol, u8 pri,
/// u8 player, u8 -[5]}`.
fn parse_strm_record(
    data: &[u8],
    inf: usize,
    rel: usize,
    files: &[(u32, u32)],
) -> Result<StrmInfo, NdsError> {
    let abs = inf + rel;
    let rec = data
        .get(abs..abs + RECORD_SIZES[STRM])
        .ok_or(NdsError::Truncated {
            what: "SDAT STRM record",
            need: abs + RECORD_SIZES[STRM],
            got: data.len(),
        })?;
    let file_id = u16le(rec, 0)?;
    if u16le(rec, 2)? != 0 {
        return Err(NdsError::Invalid {
            what: "SDAT STRM record reserved field",
        });
    }
    check_file(
        data,
        files,
        file_id,
        b"STRM",
        "SDAT STRM record does not point at a STRM file",
    )?;
    Ok(StrmInfo {
        file_id,
        volume: rec[4],
        priority: rec[5],
        player: rec[6],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal, fully valid SDAT with one SSEQ/SBNK/SWAR file
    /// and one entry per list (seqArc/strmPlayer/strm stay empty),
    /// mirroring the retail SYMB back-to-back tiling and INFO record
    /// interleaving. `with_symbols` toggles the SYMB block (block count
    /// 4 vs 3).
    ///
    /// The resulting layout, for reference (mutations below depend on
    /// these offsets):
    ///
    /// ```text
    /// 0x040  SYMB  own 0xA8   (when present)
    /// 0x0E8  INFO  own 0xA4
    /// 0x18C  FAT   own 0x3C   (3 files)
    /// 0x1C8  FILE  own 0x54   (files at 0x1E0, 0x20C, 0x214)
    /// ```
    fn build_sdat(with_symbols: bool) -> Vec<u8> {
        // ---- file image: an SSEQ of retail file 0's size, then SBNK/SWAR
        let mut seq_file = b"SSEQ".to_vec();
        seq_file.resize(0x2C, 0);
        let mut bank_file = b"SBNK".to_vec();
        bank_file.resize(8, 0);
        let mut war_file = b"SWAR".to_vec();
        war_file.resize(8, 0);
        let file_body: Vec<u8> = [seq_file, bank_file, war_file].concat();

        // ---- SYMB: sub-lists tile back-to-back from 0x40, then the pool
        let labels = [
            "SEQ_TEST\0",
            "BANK_TEST\0",
            "WAR_TEST\0",
            "PLAYER_TEST\0",
            "GROUP_TEST\0",
        ];
        let pool: String = labels.concat();
        let counts = [1u32, 0, 1, 1, 1, 1, 0, 0];
        // Which pool string each non-empty list points at (seq, bank,
        // waveArc, player, group in file order).
        let label_of = [
            Some(0),
            None,
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            None,
            None,
        ];

        let mut rels = [0u32; LISTS];
        let mut cur = 0x40u32;
        for i in 0..LISTS {
            rels[i] = cur;
            cur += 4 + 4 * counts[i];
        }
        let pool_off = cur as usize;

        let mut symb = vec![0u8; 0x40];
        symb[0..4].copy_from_slice(b"SYMB");
        for (i, &r) in rels.iter().enumerate() {
            symb[8 + 4 * i..12 + 4 * i].copy_from_slice(&r.to_le_bytes());
        }
        let mut body = Vec::new();
        for i in 0..LISTS {
            body.extend_from_slice(&counts[i].to_le_bytes());
            if let Some(k) = label_of[i] {
                let at = pool_off + pool.find(labels[k]).expect("label in pool");
                body.extend_from_slice(&(at as u32).to_le_bytes());
            }
        }
        symb.extend_from_slice(&body);
        symb.extend_from_slice(pool.as_bytes());
        while !symb.len().is_multiple_of(4) {
            symb.push(0); // pad to the 4-byte alignment blocks walk by
        }
        let symb_own = symb.len() as u32;
        symb[4..8].copy_from_slice(&symb_own.to_le_bytes());

        // ---- INFO: list arrays at their rels, records in the gaps ----
        let info_rels: [u32; LISTS] = [0x40, 0x54, 0x58, 0x6C, 0x78, 0x88, 0x9C, 0xA0];
        let mut info = vec![0u8; 0x40];
        info[0..4].copy_from_slice(b"INFO");
        for (i, &r) in info_rels.iter().enumerate() {
            info[8 + 4 * i..12 + 4 * i].copy_from_slice(&r.to_le_bytes());
        }
        let u32b = |v: u32| v.to_le_bytes().to_vec();
        let u16b = |v: u16| v.to_le_bytes().to_vec();
        let mut b = Vec::new();
        // seq: count 1, record at 0x48
        b.extend(u32b(1));
        b.extend(u32b(0x48));
        // seq record: file 0, -, bank 0, vol 100, cpr 64, ppr 64, player 0, -
        b.extend(u16b(0));
        b.extend(u16b(0));
        b.extend(u16b(0));
        b.extend([100, 64, 64, 0]);
        b.extend(u16b(0));
        // seqArc: count 0
        b.extend(u32b(0));
        // bank: count 1, record at 0x60
        b.extend(u32b(1));
        b.extend(u32b(0x60));
        // bank record: file 1, -, swar 0, unused, unused, unused
        b.extend(u16b(1));
        b.extend(u16b(0));
        b.extend(u16b(0));
        b.extend(u16b(0xFFFF));
        b.extend(u16b(0xFFFF));
        b.extend(u16b(0xFFFF));
        // waveArc: count 1, record at 0x74
        b.extend(u32b(1));
        b.extend(u32b(0x74));
        // waveArc record: file 2, -
        b.extend(u16b(2));
        b.extend(u16b(0));
        // player: count 1, record at 0x80
        b.extend(u32b(1));
        b.extend(u32b(0x80));
        // player record: seq_count 1, -, channels 0xC000, heap 24200
        b.extend([1, 0]);
        b.extend(u16b(0xC000));
        b.extend(u32b(24_200));
        // group: count 1, record at 0x90
        b.extend(u32b(1));
        b.extend(u32b(0x90));
        // group record: 1 item {Seq, flags 7, -, index 0}
        b.extend(u32b(1));
        b.extend([0, 7]);
        b.extend(u16b(0));
        b.extend(u32b(0));
        // strmPlayer: count 0; strm: count 0
        b.extend(u32b(0));
        b.extend(u32b(0));
        info.extend_from_slice(&b);
        let info_own = info.len() as u32;
        info[4..8].copy_from_slice(&info_own.to_le_bytes());

        // ---- FAT: 3 entries, absolute offsets patched after assembly
        let fat_size = 12 + 16 * 3;
        let mut fat = Vec::with_capacity(fat_size);
        fat.extend_from_slice(b"FAT ");
        fat.extend_from_slice(&(fat_size as u32).to_le_bytes());
        fat.extend_from_slice(&3u32.to_le_bytes());
        for &(rel, size) in &[(0, 0x2Cusize), (0x2C, 8), (0x34, 8)] {
            fat.extend_from_slice(&((rel + 0x1000) as u32).to_le_bytes()); // patched below
            fat.extend_from_slice(&(size as u32).to_le_bytes());
            fat.extend_from_slice(&0u32.to_le_bytes());
            fat.extend_from_slice(&0u32.to_le_bytes());
        }

        // ---- FILE
        let file_block_size = 0x18 + file_body.len();
        let mut file_block = Vec::with_capacity(file_block_size);
        file_block.extend_from_slice(b"FILE");
        file_block.extend_from_slice(&(file_block_size as u32).to_le_bytes());
        file_block.extend_from_slice(&3u32.to_le_bytes());
        file_block.extend_from_slice(&[0u8; 12]);
        file_block.extend_from_slice(&file_body);

        // ---- assembly
        let symb_len = if with_symbols { symb.len() } else { 0 };
        let total = 0x40 + symb_len + info.len() + fat.len() + file_block.len();
        let file_off = 0x40 + symb_len + info.len() + fat.len();
        // Patch the FAT entries with absolute offsets (remove the marker).
        for (i, &(_, rel)) in [(0usize, 0), (1, 0x2C), (2, 0x34)].iter().enumerate() {
            let at = 12 + 16 * i;
            fat[at..at + 4].copy_from_slice(&((file_off + 0x18 + rel) as u32).to_le_bytes());
        }

        let mut out = vec![0u8; 0x40];
        out[0..4].copy_from_slice(b"SDAT");
        out[4..6].copy_from_slice(&0xFEFFu16.to_le_bytes());
        out[6..8].copy_from_slice(&0x0100u16.to_le_bytes());
        out[8..12].copy_from_slice(&(total as u32).to_le_bytes());
        out[0x0C..0x0E].copy_from_slice(&0x40u16.to_le_bytes());
        out[0x0E..0x10].copy_from_slice(&if with_symbols { 4u16 } else { 3u16 }.to_le_bytes());
        let mut cur = 0x40;
        if with_symbols {
            out[0x10..0x14].copy_from_slice(&(cur as u32).to_le_bytes());
            out[0x14..0x18].copy_from_slice(&(symb.len() as u32).to_le_bytes());
            cur += symb.len();
        }
        out[0x18..0x1C].copy_from_slice(&(cur as u32).to_le_bytes());
        out[0x1C..0x20].copy_from_slice(&(info.len() as u32).to_le_bytes());
        cur += info.len();
        out[0x20..0x24].copy_from_slice(&(cur as u32).to_le_bytes());
        out[0x24..0x28].copy_from_slice(&(fat.len() as u32).to_le_bytes());
        cur += fat.len();
        out[0x28..0x2C].copy_from_slice(&(cur as u32).to_le_bytes());
        out[0x2C..0x30].copy_from_slice(&(file_block.len() as u32).to_le_bytes());

        if with_symbols {
            out.extend_from_slice(&symb);
        }
        out.extend_from_slice(&info);
        out.extend_from_slice(&fat);
        out.extend_from_slice(&file_block);
        assert_eq!(out.len(), total);
        out
    }

    #[test]
    fn parses_synthetic_archive() {
        let data = build_sdat(true);
        assert!(is_sdat(&data));
        let sdat = Sdat::parse(&data).expect("synthetic SDAT must parse");

        assert!(sdat.has_symbols());
        assert_eq!(sdat.file_count(), 3);
        assert_eq!(sdat.file(0).map(<[u8]>::len), Some(0x2C));
        assert_eq!(&sdat.file(0).unwrap()[0..4], b"SSEQ");
        assert_eq!(&sdat.file(1).unwrap()[0..4], b"SBNK");
        assert_eq!(&sdat.file(2).unwrap()[0..4], b"SWAR");
        assert!(sdat.file(3).is_none(), "out of range");
        assert_eq!(sdat.raw().len(), data.len());

        assert_eq!(sdat.seq_count(), 1);
        assert_eq!(sdat.seq_name(0), Some("SEQ_TEST"));
        assert_eq!(
            sdat.seq(0),
            Some(SseqInfo {
                file_id: 0,
                bank: 0,
                volume: 100,
                channel_priority: 64,
                player_priority: 64,
                player: 0,
            })
        );
        assert_eq!(sdat.bank_name(0), Some("BANK_TEST"));
        assert_eq!(
            sdat.bank(0),
            Some(BankInfo {
                file_id: 1,
                swar: [0, 0xFFFF, 0xFFFF, 0xFFFF],
            })
        );
        assert_eq!(sdat.wave_arc_name(0), Some("WAR_TEST"));
        assert_eq!(sdat.wave_arc(0), Some(SwarInfo { file_id: 2 }));
        assert_eq!(sdat.player_name(0), Some("PLAYER_TEST"));
        assert_eq!(
            sdat.player(0),
            Some(PlayerInfo {
                seq_count: 1,
                channels: 0xC000,
                heap_size: 24_200,
            })
        );
        assert_eq!(sdat.group_name(0), Some("GROUP_TEST"));
        let items = sdat.group(0).expect("group record is present");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            GroupItem {
                kind: GroupItemKind::Seq,
                flags: 7,
                index: 0,
            }
        );
        assert_eq!(sdat.seq_arc_count(), 0);
        assert_eq!(sdat.strm_count(), 0);
    }

    #[test]
    fn parses_symbol_free_archive() {
        let data = build_sdat(false);
        assert!(is_sdat(&data));
        let sdat = Sdat::parse(&data).expect("symbol-free SDAT must parse");

        assert!(!sdat.has_symbols());
        assert_eq!(sdat.seq_name(0), None);
        assert_eq!(sdat.bank_name(0), None);
        assert_eq!(sdat.player(0).map(|p| p.channels), Some(0xC000));
    }

    #[test]
    fn rejects_broken_archives() {
        let good = build_sdat(true);
        let bad = |mutate: fn(&mut Vec<u8>)| {
            let mut bad = good.clone();
            mutate(&mut bad);
            assert!(Sdat::parse(&bad).is_err());
        };

        bad(|d| d[0] = b'X'); // magic
        bad(|d| d[4] = 0x00); // byte-order mark
        bad(|d| d[6] = 0x02); // version
        bad(|d| d[8] -= 1); // file size
        bad(|d| d[0x0C] = 0x10); // header size
        bad(|d| d[0x0E] = 3); // block count vs present SYMB
        bad(|d| d[0x30] = 1); // header padding
        bad(|d| d[0x14] += 1); // SYMB pair size beyond its own size
        bad(|d| d[0x44] += 1); // SYMB own size not 4-byte aligned
        bad(|d| d[0x48] = 0); // SYMB sub-list absent
        bad(|d| d[0x48] = 0x44); // SYMB sub-lists do not tile
        bad(|d| d[0x88] = 1); // SSAR folder symbol list (seqArc count)
        bad(|d| d[0x84..0x88].copy_from_slice(&4u32.to_le_bytes())); // symbol inside the header
        bad(|d| d[0x110] = 1); // INFO header padding
        bad(|d| d[0xF4] = 0x30); // INFO sub-lists out of order
        bad(|d| d[0x128] = 2); // INFO count disagrees with SYMB
        bad(|d| d[0x12C] = 0); // symbol names an absent record
        bad(|d| d[0x12C..0x130].copy_from_slice(&0xFFFFu32.to_le_bytes())); // record outside its region
        bad(|d| d[0x136] = 128); // SEQ volume
        bad(|d| d[0x139] = 5); // SEQ player out of range
        bad(|d| d[0x17C] = 4); // GROUP item type
        bad(|d| d[0x194] = 4); // FAT count disagrees with block size
        bad(|d| d[0x1A8] += 1); // FAT file unaligned
        bad(|d| d[0x1D0] = 2); // FILE count disagrees with FAT
        bad(|d| d[0x1D8] = 1); // FILE header padding
        bad(|d| d[0x20C..0x210].copy_from_slice(b"XBNK")); // SBNK file magic
        bad(|d| d.truncate(0x21C - 4)); // truncation

        // The seqArc INFO mutation above leaves a zero count list — it
        // only exercises the offsets-array walk; the named check needs a
        // real mutation. Point the seq record at the SBNK file instead:
        bad(|d| d[0x130..0x132].copy_from_slice(&1u16.to_le_bytes())); // SEQ file is not an SSEQ

        assert!(!is_sdat(b"SDAT not really an archive"));
        assert!(Sdat::parse(b"SDAT").is_err());
    }
}
