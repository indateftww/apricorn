//! The save format — reading and writing original HeartGold save
//! blobs (`src/save.c`, PLAN.md Phase 4, step 4).
//!
//! HeartGold saves to a 512-KiB card backup organized in **two
//! 64-KiB slot mirrors** and a belt of **extra chunks** beyond them:
//!
//! ```text
//! 0x00000  slot 0 ── main chunk  (blocks 0..=40 + footer)   0x  F628 bytes
//! 0x0F700  slot 0 ── PC chunk   (PCSTORAGE + footer)       0x 12310 bytes
//! 0x23000  extra chunks, primary copies    (sectors 35..45, see table)
//! 0x40000  slot 1 ── the same two chunks, one save generation behind
//! 0x63000  extra chunks, mirror copies     (sector + 64)
//! 0x80000  end of the card backup (128 sectors × 4 KiB)
//! ```
//!
//! Every write lands in the slot the *last good save* does not occupy,
//! so a crash mid-write always leaves the previous generation intact;
//! on boot both slots' chunk footers are validated and the newer save
//! counter wins ([`SaveData::parse`] ports `Save_GetSaveFilesStatus`
//! verbatim, wraparound quirk included). Per-block integrity is a
//! trailing u16 CRC-16/CCITT ([`crc16`]) computed over the block's
//! padded bytes (`SaveSubstruct_UpdateCRC`), and each chunk footer
//! carries its own CRC over the whole chunk.
//!
//! **Parity is differential, not asserted:** the block-size table and
//! the CRC are locked to the retail image by `apricorn-harness`'s
//! `tests/save_hg.rs` (every `gSaveChunkHeaders` size stub called out
//! of the ROM via arm-runner; `GF_CalcCRC16` fed the same streams),
//! and the container round-trips byte-identically — a blob that
//! [`SaveData::parse`] accepts, [`SaveData::to_bytes`] re-emits
//! unchanged, and a `save_game` generation matches what the original's
//! alternating-slot write leaves on the card.
//!
//! Not in this step, by design: per-block *contents* (each subsystem
//! owns its struct — Party lands with Phase 6), the new-game
//! `initFunc` defaults that fill a fresh dynamic region (Phase 4,
//! step 5 and beyond), and writing the extra chunks (Hall of Fame
//! recording arrives with the systems that use it). The container
//! reads and validates them all the way the original does.

mod crc;
mod layout;

pub use crc::crc16;
pub use layout::{
    BLOCK_RAW_SIZES, BLOCKS, BlockLayout, DYNAMIC_REGION_SIZE, FOOTER_SIZE, SAVE_BLOCK_NUM,
    SAVE_CHUNK_MAGIC, SAVE_PAGE_MAX, SAVE_SECTOR_SIZE, SLOT_SPECS, SlotSpec, USED_REGION_END,
    block, block_crc_offset, block_stored_size,
};

use core::fmt;

/// The card backup size the game formats: 128 flash sectors
/// (`Save_DeleteAllData` wipes exactly this many).
pub const CARD_BACKUP_SIZE: usize = 128 * SAVE_SECTOR_SIZE;

/// The stride between the two slot mirrors
/// (`GetChunkOffsetFromCurrentSaveSlot`).
pub const SLOT_STRIDE: usize = 0x40000;

/// The number of extra chunks (`gExtraSaveChunkHeaders`).
pub const EXTRA_CHUNK_NUM: usize = 6;

/// The flash sector of each extra chunk's primary copy —
/// `gExtraSaveChunkHeaders`' `sector` column (the Hall of Fame is the
/// only one pret names). The mirror copy lives 64 sectors later.
pub const EXTRA_CHUNK_SECTORS: [u32; EXTRA_CHUNK_NUM] = [35, 38, 39, 41, 43, 45];

/// Raw struct sizes of the extra chunks — the returns of their size
/// stubs in the retail image (`Save_HOF_sizeof`, `sub_020312A4`, and
/// `sub_0202FBCC` for the four record chunks), locked by the harness
/// differential like the block table.
pub const EXTRA_CHUNK_SIZES: [u32; EXTRA_CHUNK_NUM] =
    [0x2AB0, 0xBA0, 0x1D50, 0x1D50, 0x1D50, 0x1D50];

/// What the boot probe found on the card (`LOAD_STATUS_*`) — carried
/// by [`SaveError`] because each failure mode routes the game to a
/// different screen, and by [`SaveData::slot_degraded`] for the one
/// case that still loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadStatus {
    /// Both chunks of one slot failed: no save exists — new game.
    NotExist,
    /// Both slots loaded and agree: the normal case.
    Good,
    /// One chunk of the newest generation failed; the previous
    /// generation (or the slot's surviving chunk) loaded instead.
    /// The game proceeds behind a warning banner.
    SlotFail,
    /// Nothing loadable: the game refuses the file.
    TotalFail,
}

/// A failure to load a card backup — the terminal outcomes of the
/// boot probe, plus the structural rejection of a foreign blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveError {
    /// The blob is not a 512-KiB card backup.
    NotACardBackup {
        /// The size found, in bytes.
        got: usize,
    },
    /// `LOAD_STATUS_NOT_EXIST`: a blank (or wiped) card — the game
    /// would start a new game.
    NoSaveData,
    /// `LOAD_STATUS_TOTAL_FAIL`: no slot's chunks validate — the game
    /// would refuse to continue.
    Corrupt,
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::NotACardBackup { got } => {
                write!(
                    f,
                    "not a card backup: expected {CARD_BACKUP_SIZE} bytes, got {got}"
                )
            }
            SaveError::NoSaveData => write!(f, "no save data on the card"),
            SaveError::Corrupt => write!(f, "save data is corrupt (no slot validates)"),
        }
    }
}

impl std::error::Error for SaveError {}

/// `SaveCounterCompare`: the save-counter ordering, with the original
///'s wraparound quirk — a counter of `0xFFFF_FFFF` (an all-0xFF
/// half-written footer) is treated as *older than* `0`, because the
/// clobber pattern reads as `-1`.
#[must_use]
pub fn save_counter_compare(first: u32, second: u32) -> i32 {
    if first == u32::MAX && second == 0 {
        return -1;
    }
    if first == 0 && second == u32::MAX {
        return 1;
    }
    if first > second {
        1
    } else {
        -i32::from(first < second)
    }
}

/// A chunk footer's fields, read out of a slot window
/// (`struct SaveChunkFooter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkFooter {
    /// The save counter the chunk was written under.
    count: u32,
    /// The chunk's stored size, footer included.
    size: u32,
    /// `SAVE_CHUNK_MAGIC`.
    magic: u32,
    /// The chunk id the footer seals (0 main, 1 PC).
    slot: u16,
    /// CRC-16 over the chunk minus the footer.
    crc: u16,
}

/// An extra chunk footer's fields (`struct SaveArrayFooter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArrayFooter {
    /// `SAVE_CHUNK_MAGIC`.
    magic: u32,
    /// The writer's `lastGoodSaveNo + 1` — the extra-chunk generation.
    saveno: u32,
    /// The chunk's raw struct size.
    size: u32,
    /// The extra chunk id.
    idx: u16,
    /// CRC-16 over the raw bytes plus the footer's first 12 bytes.
    crc: u16,
}

/// A little-endian u32 at `off`, if the slice reaches.
fn u32at(data: &[u8], off: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(off..off + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// A little-endian u16 at `off`, if the slice reaches.
fn u16at(data: &[u8], off: usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(off..off + 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

impl ChunkFooter {
    /// Reads the footer sealing `spec` inside a slot's window
    /// (`GetSaveSectorFooterPtr`): at `spec.offset + spec.size - 16`.
    fn read(window: &[u8], spec: &SlotSpec) -> Option<Self> {
        let at = spec.offset as usize + spec.size as usize - FOOTER_SIZE;
        Some(Self {
            count: u32at(window, at)?,
            size: u32at(window, at + 4)?,
            magic: u32at(window, at + 8)?,
            slot: u16at(window, at + 12)?,
            crc: u16at(window, at + 14)?,
        })
    }

    /// Writes the footer back where [`Self::read`] found it.
    fn write(self, window: &mut [u8], spec: &SlotSpec) {
        let at = spec.offset as usize + spec.size as usize - FOOTER_SIZE;
        window[at..at + 4].copy_from_slice(&self.count.to_le_bytes());
        window[at + 4..at + 8].copy_from_slice(&self.size.to_le_bytes());
        window[at + 8..at + 12].copy_from_slice(&self.magic.to_le_bytes());
        window[at + 12..at + 14].copy_from_slice(&self.slot.to_le_bytes());
        window[at + 14..at + 16].copy_from_slice(&self.crc.to_le_bytes());
    }

    /// `ValidateSaveSectorFooter`: size, magic, slot id, and the CRC
    /// over the chunk's `size - 16` stored bytes.
    fn validates(self, window: &[u8], spec: &SlotSpec) -> bool {
        self.size == spec.size
            && self.magic == SAVE_CHUNK_MAGIC
            && u32::from(self.slot) == u32::from(spec.id)
            && crc16(
                &window
                    [spec.offset as usize..spec.offset as usize + self.size as usize - FOOTER_SIZE],
            ) == self.crc
    }
}

impl ArrayFooter {
    /// Reads the footer at `data[raw_size..]` of an extra chunk copy
    /// (`CreateChunkFooter` writes it just past the raw bytes).
    fn read(data: &[u8], raw_size: u32) -> Option<Self> {
        let at = raw_size as usize;
        Some(Self {
            magic: u32at(data, at)?,
            saveno: u32at(data, at + 4)?,
            size: u32at(data, at + 8)?,
            idx: u16at(data, at + 12)?,
            crc: u16at(data, at + 14)?,
        })
    }

    /// `ValidateChunk`: magic, raw size, chunk id, and the CRC over
    /// the raw bytes plus the footer's first 14 bytes
    /// (`size + offsetof(struct SaveArrayFooter, crc)` — everything
    /// up to and excluding the `crc` field itself).
    fn validates(self, data: &[u8], raw_size: u32, idx: usize) -> bool {
        self.magic == SAVE_CHUNK_MAGIC
            && self.size == raw_size
            && usize::from(self.idx) == idx
            && crc16(&data[..raw_size as usize + 14]) == self.crc
    }
}

/// One slot's boot probe of a chunk (`struct SaveSlotCheck`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SlotCheck {
    /// The footer validated.
    valid: bool,
    /// The footer's save counter (0 when invalid — the original zeroes
    /// it so the comparison orderings stay total).
    count: u32,
}

/// `SaveSlotCheck_InitFromSavedat`: probe one chunk of one slot
/// window.
fn slot_check(window: &[u8], spec: &SlotSpec) -> SlotCheck {
    match ChunkFooter::read(window, spec) {
        Some(footer) if footer.validates(window, spec) => SlotCheck {
            valid: true,
            count: footer.count,
        },
        _ => SlotCheck {
            valid: false,
            count: 0,
        },
    }
}

/// `SaveSlotCheckCompare`: how many of the two slots' probes are good,
/// and which slot index is the newer / older generation. `2` for the
/// newer/older slots means "none".
fn slot_check_compare(
    first: SlotCheck,
    second: SlotCheck,
) -> (u32, /* newer */ u32, /* older */ u32) {
    let cmp = save_counter_compare(first.count, second.count);
    if first.valid && second.valid {
        if cmp > 0 {
            (2, 0, 1)
        } else if cmp < 0 {
            (2, 1, 0)
        } else {
            (2, 0, 1)
        }
    } else if first.valid {
        (1, 0, 2)
    } else if second.valid {
        (1, 1, 2)
    } else {
        (0, 2, 2)
    }
}

/// The outcome of the boot probe: a load status plus, when anything
/// is loadable, which slot won and its save counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FilesStatus {
    status: LoadStatus,
    counter: u32,
    sector: usize,
}

/// `Save_GetSaveFilesStatus` + `Save_RecordWhichLatestGoodSector`,
/// ported verbatim (see `src/save.c`): probe both chunks of both
/// slots, order the generations, and agree main with PC before
/// declaring anything good.
fn files_status(blob: &[u8]) -> FilesStatus {
    let check = |slot: usize, spec: &SlotSpec| {
        slot_check(
            &blob[slot * SLOT_STRIDE..slot * SLOT_STRIDE + DYNAMIC_REGION_SIZE],
            spec,
        )
    };
    let checks_main = [check(0, &SLOT_SPECS[0]), check(1, &SLOT_SPECS[0])];
    let checks_sub = [check(0, &SLOT_SPECS[1]), check(1, &SLOT_SPECS[1])];

    let (num_good_main, newer_main, older_main) =
        slot_check_compare(checks_main[0], checks_main[1]);
    let (num_good_sub, _newer_sub, _older_sub) = slot_check_compare(checks_sub[0], checks_sub[1]);

    let record = |idx: usize| FilesStatus {
        status: LoadStatus::Good,
        counter: checks_main[idx].count,
        sector: idx,
    };
    let with = |status: LoadStatus, idx: usize| FilesStatus {
        status,
        counter: checks_main[idx].count,
        sector: idx,
    };
    let fail = FilesStatus {
        status: LoadStatus::TotalFail,
        counter: 0,
        sector: 0,
    };

    if num_good_main == 0 && num_good_sub == 0 {
        return FilesStatus {
            status: LoadStatus::NotExist,
            counter: 0,
            sector: 0,
        };
    }
    if num_good_main == 0 || num_good_sub == 0 {
        return fail;
    }
    debug_assert!(newer_main != 2, "both main slots failed but counted good");

    if num_good_main == 2 && num_good_sub == 2 {
        if checks_main[newer_main as usize].count == checks_sub[newer_main as usize].count {
            return record(newer_main as usize);
        }
        if checks_main[older_main as usize].count != checks_sub[older_main as usize].count {
            return fail;
        }
        return with(LoadStatus::SlotFail, older_main as usize);
    }
    if num_good_main == 1 && num_good_sub == 2 {
        if checks_main[newer_main as usize].count == checks_sub[newer_main as usize].count {
            return with(LoadStatus::SlotFail, newer_main as usize);
        }
        return fail;
    }
    if num_good_main == 2 && num_good_sub == 1 {
        if checks_main[newer_main as usize].count == checks_sub[newer_main as usize].count {
            return record(newer_main as usize);
        }
        if older_main == 2 {
            return fail;
        }
        if checks_main[older_main as usize].count == checks_sub[older_main as usize].count {
            return with(LoadStatus::SlotFail, older_main as usize);
        }
        return fail;
    }
    if num_good_main == 1 && num_good_sub == 1 {
        if newer_main == _newer_sub {
            debug_assert!(
                checks_main[newer_main as usize].count == checks_sub[_newer_sub as usize].count,
                "main and PC of the same slot disagree on their counter"
            );
            return record(newer_main as usize);
        }
    }
    fail
}

/// A loaded save: the whole card backup, plus where the good
/// generation lives.
///
/// The blob is kept verbatim — [`Self::to_bytes`] re-emits it
/// byte-identically, [`Self::save_game`] rewrites exactly the region
/// the original's alternating-slot write touches, and block and extra
/// views slice straight into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveData {
    /// The 512-KiB card backup, always complete and always consistent
    /// with the fields below.
    blob: Vec<u8>,
    /// `saveCounter` of the good generation (`Save_RecordWhichLatest
    /// GoodSector`).
    counter: u32,
    /// The slot mirror the good generation lives in
    /// (`lastGoodSector`).
    last_good_sector: usize,
    /// `LOAD_STATUS_SLOT_FAIL` — loaded, but the game would warn.
    slot_degraded: bool,
}

impl SaveData {
    /// `SaveData_New`'s load path: probe both slot mirrors like the
    /// boot code and keep the newest loadable generation.
    ///
    /// # Errors
    /// [`SaveError::NotACardBackup`] for a blob of the wrong size;
    /// [`SaveError::NoSaveData`] for a blank card (`NOT_EXIST` — the
    /// game would start a new game); [`SaveError::Corrupt`] when no
    /// slot's chunks validate (`TOTAL_FAIL` — the game would refuse).
    /// A save whose newest generation lost a chunk loads with
    /// [`Self::slot_degraded`] set, exactly the case the original
    /// banners through.
    pub fn parse(blob: &[u8]) -> Result<Self, SaveError> {
        if blob.len() != CARD_BACKUP_SIZE {
            return Err(SaveError::NotACardBackup { got: blob.len() });
        }
        let status = files_status(blob);
        match status.status {
            LoadStatus::NotExist => Err(SaveError::NoSaveData),
            LoadStatus::TotalFail => Err(SaveError::Corrupt),
            LoadStatus::Good | LoadStatus::SlotFail => {
                // Save_LoadDynamicRegion re-validates both chunks of
                // the chosen slot straight off the card.
                let window = &blob[status.sector * SLOT_STRIDE
                    ..status.sector * SLOT_STRIDE + DYNAMIC_REGION_SIZE];
                for spec in SLOT_SPECS {
                    debug_assert!(
                        ChunkFooter::read(window, &spec)
                            .is_some_and(|f| f.validates(window, &spec)),
                        "the recorded-good sector must still validate"
                    );
                }
                Ok(Self {
                    blob: blob.to_vec(),
                    counter: status.counter,
                    last_good_sector: status.sector,
                    slot_degraded: status.status == LoadStatus::SlotFail,
                })
            }
        }
    }

    /// The card backup exactly as loaded (or as the last
    /// [`Self::save_game`] left it) — byte-identical to the input of
    /// [`Self::parse`].
    #[must_use]
    pub fn to_bytes(&self) -> &[u8] {
        &self.blob
    }

    /// The card backup, owned.
    #[must_use]
    pub fn into_blob(self) -> Vec<u8> {
        self.blob
    }

    /// `saveCounter` of the loaded generation.
    #[must_use]
    pub fn counter(&self) -> u32 {
        self.counter
    }

    /// `lastGoodSector`: the slot mirror the loaded generation lives
    /// in (0 or 1) — the one the next save does *not* write.
    #[must_use]
    pub fn last_good_sector(&self) -> usize {
        self.last_good_sector
    }

    /// `LOAD_STATUS_SLOT_FAIL`: the newest generation lost a chunk
    /// and the load fell back — the game proceeds behind a warning.
    #[must_use]
    pub fn slot_degraded(&self) -> bool {
        self.slot_degraded
    }

    /// The window of the good generation inside the blob — the
    /// dynamic region the block views slice into.
    fn window(&self) -> &[u8] {
        &self.blob[self.last_good_sector * SLOT_STRIDE
            ..self.last_good_sector * SLOT_STRIDE + DYNAMIC_REGION_SIZE]
    }

    /// The mutable counterpart of [`Self::window`].
    fn window_mut(&mut self) -> &mut [u8] {
        let base = self.last_good_sector * SLOT_STRIDE;
        &mut self.blob[base..base + DYNAMIC_REGION_SIZE]
    }

    /// A block's raw struct bytes — what `SaveArray_Get` hands the
    /// subsystems. The stored CRC and tail padding live just past the
    /// end of this slice, inside the block's [`BlockLayout::size`]
    /// region.
    ///
    /// # Panics
    /// If `id` is not a valid block id (`< SAVE_BLOCK_NUM`).
    #[must_use]
    pub fn block(&self, id: usize) -> &[u8] {
        let layout = BLOCKS
            .get(id)
            .unwrap_or_else(|| panic!("save block id {id} is out of range"));
        &self.window()[layout.offset as usize..layout.offset as usize + layout.raw_size as usize]
    }

    /// The mutable counterpart of [`Self::block`].
    ///
    /// # Panics
    /// If `id` is not a valid block id.
    #[must_use]
    pub fn block_mut(&mut self, id: usize) -> &mut [u8] {
        let layout = BLOCKS
            .get(id)
            .unwrap_or_else(|| panic!("save block id {id} is out of range"));
        let raw = layout.raw_size as usize;
        let offset = layout.offset as usize;
        &mut self.window_mut()[offset..offset + raw]
    }

    /// The stored u16 CRC of a block (`SaveSubstruct_AssertCRC`'s
    /// comparison target).
    ///
    /// # Panics
    /// If `id` is not a valid block id.
    #[must_use]
    pub fn stored_block_crc(&self, id: usize) -> u16 {
        let layout = &BLOCKS[id];
        u16at(self.window(), block_crc_offset(layout))
            .expect("the CRC slot lies inside the dynamic region")
    }

    /// `SaveSubstruct_AssertCRC` as a query: does the block's stored
    /// CRC match its bytes? Like the original, the CRC covers the
    /// struct *padded* to 4 bytes — for a block whose raw size is not
    /// word-aligned, the padding bytes count too
    /// (`GetSaveChunkSizePlusCRC(id) - 4`).
    #[must_use]
    pub fn block_crc_ok(&self, id: usize) -> bool {
        let layout = &BLOCKS[id];
        self.stored_block_crc(id)
            == crc16(
                &self.window()
                    [layout.offset as usize..layout.offset as usize + layout.size as usize - 4],
            )
    }

    /// `SaveSubstruct_UpdateCRC`: recompute a block's trailing CRC
    /// over its padded bytes after a subsystem edited it. Every
    /// subsequent save carries the new value; the original calls this
    /// from each subsystem's own edit paths.
    ///
    /// # Panics
    /// If `id` is not a valid block id.
    pub fn update_block_crc(&mut self, id: usize) {
        let layout = &BLOCKS[id];
        let crc = crc16(
            &self.window()
                [layout.offset as usize..layout.offset as usize + layout.size as usize - 4],
        );
        let at = block_crc_offset(layout);
        self.window_mut()[at..at + 2].copy_from_slice(&crc.to_le_bytes());
    }

    /// `SaveGameNormal` on an existing save (the `_NowWriteFlash` +
    /// `Save_WriteManFinish` effect on the card): bump the save
    /// counter, copy both chunks of the good generation into the
    /// *other* slot with rebuilt footers, and flip the good sector.
    ///
    /// The previous generation is left untouched — that mirror is the
    /// crash protection the two-slot scheme exists for. Everything
    /// outside the written chunks (the previous mirror, the extra
    /// chunks, the padding) keeps its bytes, so the next
    /// [`Self::parse`] loads the new generation and
    /// [`Self::to_bytes`] stays a faithful card image.
    pub fn save_game(&mut self) {
        let slot = 1 - self.last_good_sector;
        self.counter = self.counter.wrapping_add(1);

        let src_base = self.last_good_sector * SLOT_STRIDE;
        let dst_base = slot * SLOT_STRIDE;
        for spec in SLOT_SPECS {
            // The chunk copies verbatim out of the good generation;
            // the footer is rebuilt with the new counter
            // (SaveSlot_BuildFooter).
            let from = self.blob[src_base + spec.offset as usize
                ..src_base + spec.offset as usize + spec.size as usize]
                .to_vec();
            let window = &mut self.blob[dst_base..dst_base + DYNAMIC_REGION_SIZE];
            window[spec.offset as usize..spec.offset as usize + spec.size as usize]
                .copy_from_slice(&from);
            ChunkFooter {
                count: self.counter,
                size: spec.size,
                magic: SAVE_CHUNK_MAGIC,
                slot: u16::from(spec.id),
                crc: crc16(
                    &window[spec.offset as usize
                        ..spec.offset as usize + spec.size as usize - FOOTER_SIZE],
                ),
            }
            .write(window, &spec);
        }

        self.last_good_sector = slot;
        self.slot_degraded = false;
    }

    /// One extra chunk, probed like `ReadExtraSaveChunk`: both copies
    /// (the primary at the chunk's sector, the mirror 64 sectors
    /// later) are validated, and the newer `saveno` wins. The four
    /// bytes before each copy's footer are the chunk's own PRandom
    /// guard word (`sub_02028230`) — validated here only as bytes.
    ///
    /// `id` indexes `gExtraSaveChunkHeaders`: 0 is the Hall of Fame.
    ///
    /// # Panics
    /// If `id` is not a valid extra chunk id (`< EXTRA_CHUNK_NUM`).
    #[must_use]
    pub fn extra_chunk(&self, id: usize) -> ExtraChunk<'_> {
        assert!(id < EXTRA_CHUNK_NUM, "extra chunk id {id} is out of range");
        let sector = EXTRA_CHUNK_SECTORS[id] as usize;
        let raw = EXTRA_CHUNK_SIZES[id] as usize;
        let stored = raw + FOOTER_SIZE;

        let copy = |base: usize| -> Option<(ArrayFooter, usize)> {
            let data = self.blob.get(base..base + stored)?;
            let footer = ArrayFooter::read(data, raw as u32)?;
            footer
                .validates(data, raw as u32, id)
                .then_some((footer, base))
        };
        let first = copy(sector * SAVE_SECTOR_SIZE);
        let second = copy((sector + 64) * SAVE_SECTOR_SIZE);

        let chosen = match (first, second) {
            (Some((footer, base)), None) => Some((footer.saveno, base)),
            (None, Some((footer, base))) => Some((footer.saveno, base)),
            (Some((f1, b1)), Some((f2, b2))) => {
                if save_counter_compare(f1.saveno, f2.saveno) != -1 {
                    Some((f1.saveno, b1))
                } else {
                    Some((f2.saveno, b2))
                }
            }
            (None, None) => None,
        };
        ExtraChunk {
            data: chosen
                .map(|(_, base)| &self.blob[base..base + raw])
                .unwrap_or(&[]),
            saveno: chosen.map_or(0, |(saveno, _)| saveno),
            present: chosen.is_some(),
        }
    }
}

/// One extra chunk's load outcome — [`SaveData::extra_chunk`]'s view.
///
/// The original returns a status integer alongside the buffer; here
/// [`Self::data`] is empty and [`Self::saveno`] is 0 exactly when
/// neither copy validated (`ret_p == 2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtraChunk<'a> {
    /// The chunk's raw bytes from the winning copy (empty if neither
    /// copy validates).
    data: &'a [u8],
    /// The winning copy's `saveno` (0 with no winner).
    saveno: u32,
    /// Whether any copy validated.
    present: bool,
}

impl ExtraChunk<'_> {
    /// The chunk's raw struct bytes, when a copy validated.
    #[must_use]
    pub fn data(&self) -> Option<&[u8]> {
        self.present.then_some(self.data)
    }

    /// The winning copy's `saveno` generation counter.
    #[must_use]
    pub fn saveno(&self) -> u32 {
        self.saveno
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_compare_orders_with_the_wraparound_quirk() {
        // Plain ordering.
        assert_eq!(save_counter_compare(3, 2), 1);
        assert_eq!(save_counter_compare(2, 3), -1);
        assert_eq!(save_counter_compare(7, 7), 0);
        // The quirk: 0xFFFFFFFF (an all-0xFF clobbered footer) is
        // older than a wrapped-around 0.
        assert_eq!(save_counter_compare(u32::MAX, 0), -1);
        assert_eq!(save_counter_compare(0, u32::MAX), 1);
        // But it is still newer than everything else (so a stale
        // 0xFFFFFFFF footer can only lose to a fresh wrap).
        assert_eq!(save_counter_compare(u32::MAX, 5), 1);
    }

    #[test]
    fn slot_check_compare_matches_the_original_matrix() {
        let good = |count: u32| SlotCheck { valid: true, count };
        let bad = SlotCheck {
            valid: false,
            count: 0,
        };
        // Both good: the newer slot is first on ties (the original
        // picks slot 0 when the counters are equal).
        assert_eq!(slot_check_compare(good(5), good(3)), (2, 0, 1));
        assert_eq!(slot_check_compare(good(3), good(5)), (2, 1, 0));
        assert_eq!(slot_check_compare(good(4), good(4)), (2, 0, 1));
        // Exactly one good: it is the newer, none is older (the 2
        // sentinel).
        assert_eq!(slot_check_compare(good(1), bad), (1, 0, 2));
        assert_eq!(slot_check_compare(bad, good(9)), (1, 1, 2));
        // None good.
        assert_eq!(slot_check_compare(bad, bad), (0, 2, 2));
        // The wraparound quirk flows through the comparison: a
        // wrapped 0 beats a 0xFFFFFFFF footer.
        assert_eq!(slot_check_compare(good(0), good(u32::MAX)), (2, 0, 1));
        assert_eq!(slot_check_compare(good(u32::MAX), good(0)), (2, 1, 0));
    }

    #[test]
    fn wrong_size_is_rejected_before_any_probe() {
        assert_eq!(
            SaveData::parse(&[0xFF; 0x40000]).unwrap_err(),
            SaveError::NotACardBackup { got: 0x40000 }
        );
        assert_eq!(
            SaveData::parse(&[]).unwrap_err(),
            SaveError::NotACardBackup { got: 0 }
        );
    }

    #[test]
    fn a_wiped_card_has_no_save_data() {
        // Save_DeleteAllData fills the whole backup with 0xFF.
        assert_eq!(
            SaveData::parse(&[0xFF; CARD_BACKUP_SIZE]).unwrap_err(),
            SaveError::NoSaveData
        );
        // So does a fresh card (never written reads as all-0xFF on
        // this flash), and the empty-region zero blob likewise.
        assert_eq!(
            SaveData::parse(&[0x00; CARD_BACKUP_SIZE]).unwrap_err(),
            SaveError::NoSaveData
        );
    }
}
