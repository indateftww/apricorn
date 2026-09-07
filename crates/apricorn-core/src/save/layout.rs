//! The save block layout — where each of the 42 blocks lives inside a
//! save slot (`src/save.c`, `SaveData_InitSubstructs` /
//! `SaveData_InitSlotSpecs`).
//!
//! Nothing here is hand-typed. The per-block struct sizes are the
//! returns of the original `gSaveChunkHeaders` size stubs in the retail
//! ARM9 image, pinned in `apricorn-harness`'s `pins/arm9.tsv` and
//! locked draw-for-draw by `apricorn-harness`'s `tests/save_hg.rs`,
//! which calls every one of them out of the ROM via arm-runner. The
//! committed [`BLOCK_RAW_SIZES`] is that call's output, frozen so the
//! engine crate stays dependency-free and headless-testable without the
//! ROM; the harness test refuses to let it drift.
//!
//! The offsets are computed by the same arithmetic the original runs at
//! boot (ported exactly, quirks included), and the result lands on the
//! cartridge's own budget with zero slack: the main and PC chunks
//! together occupy exactly [`SAVE_PAGE_MAX`] 4-KiB flash pages — an
//! invariant the unit tests pin.

/// The 16-byte footer that seals a whole save chunk
/// (`struct SaveChunkFooter`, `include/save.h`):
/// `{ u32 count; u32 size; u32 magic; u16 slot; u16 crc }`.
pub const FOOTER_SIZE: usize = 16;

/// The save magic stamping every chunk footer (`SAVE_CHUNK_MAGIC`).
pub const SAVE_CHUNK_MAGIC: u32 = 0x2006_0623;

/// A flash sector — the card backup's write granularity
/// (`SAVE_SECTOR_SIZE`).
pub const SAVE_SECTOR_SIZE: usize = 0x1000;

/// Flash pages available to the save data (`SAVE_PAGE_MAX`): 35 ×
/// [`SAVE_SECTOR_SIZE`] = 0x23000 bytes of dynamic region, holding the
/// main chunk (blocks 0–40), the PC-storage chunk (block 41), and
/// padding. The extra chunks (Hall of Fame, battle records) start at
/// page 35, outside this window.
pub const SAVE_PAGE_MAX: usize = 35;

/// The size of the dynamic region the game keeps in RAM
/// (`SaveData.dynamic_region`): the full page window, of which the
/// chunks themselves occupy [`USED_REGION_END`] bytes.
pub const DYNAMIC_REGION_SIZE: usize = SAVE_PAGE_MAX * SAVE_SECTOR_SIZE;

/// Block ids — pret's `SAVE_*` (`include/constants/save_arrays.h`),
/// which index both [`BLOCK_RAW_SIZES`] and the layout.
pub mod block {
    /// System info: the owner's name, adventure start, version.
    pub const SYSINFO: usize = 0;
    /// Player data: money, coins, play time, location.
    pub const PLAYERDATA: usize = 1;
    /// The party (up to 6 mons).
    pub const PARTY: usize = 2;
    /// The bag.
    pub const BAG: usize = 3;
    /// Story flags and script variables.
    pub const FLAGS: usize = 4;
    /// Local field data: respawn point, weather, camera.
    pub const LOCAL_FIELD_DATA: usize = 5;
    /// Pokédex seen/caught.
    pub const POKEDEX: usize = 6;
    /// Daycare.
    pub const DAYCARE: usize = 7;
    /// Pal Pad.
    pub const PALPAD: usize = 8;
    /// Miscellaneous: extra-chunk bookkeeping among much else.
    pub const MISC: usize = 9;
    /// Saved map objects.
    pub const MAP_OBJECTS: usize = 10;
    /// Link battle rulesets.
    pub const LINK_BATTLE_RULESET: usize = 11;
    /// Dress-up (fashion) data.
    pub const DRESSUP_DATA: usize = 12;
    /// Mailbox.
    pub const MAILBOX: usize = 13;
    /// Friend group.
    pub const FRIEND_GROUP: usize = 14;
    /// Trainer card.
    pub const TRAINER_CARD: usize = 15;
    /// Game stats.
    pub const GAMESTATS: usize = 16;
    /// Seal case.
    pub const SEAL_CASE: usize = 17;
    /// Chatot cry recording.
    pub const CHATOT: usize = 18;
    /// Battle Frontier data (pret `SAVE_UNK_19`).
    pub const UNK_19: usize = 19;
    /// Special ribbons.
    pub const SPECIAL_RIBBONS: usize = 20;
    /// Roamers.
    pub const ROAMER: usize = 21;
    /// Unknown block 22.
    pub const UNK_22: usize = 22;
    /// Unknown block 23 (Safari-zone-related getter in pret).
    pub const UNK_23: usize = 23;
    /// Rankings.
    pub const RANKINGS: usize = 24;
    /// Unknown block 25.
    pub const UNK_25: usize = 25;
    /// Wi-Fi history.
    pub const WIFI_HISTORY: usize = 26;
    /// Mystery gift.
    pub const MYSTERY_GIFT: usize = 27;
    /// Migrated (Pal Park) pokémon.
    pub const UNK_28: usize = 28;
    /// Pokéathlon friendship records.
    pub const POKEATHLON_FRIENDSHIP_RECORDS: usize = 29;
    /// Easy chat vocabulary.
    pub const EASY_CHAT: usize = 30;
    /// Unknown block 31.
    pub const UNK_31: usize = 31;
    /// Unknown block 32.
    pub const UNK_32: usize = 32;
    /// Following pokémon.
    pub const FOLLOW_MON: usize = 33;
    /// Pokégear.
    pub const POKEGEAR: usize = 34;
    /// Safari zone.
    pub const SAFARI_ZONE: usize = 35;
    /// Photo album.
    pub const PHOTO_ALBUM: usize = 36;
    /// Pokéathlon.
    pub const POKEATHLON: usize = 37;
    /// Apricorn box.
    pub const APRICORN_BOX: usize = 38;
    /// Pokéwalker.
    pub const POKEWALKER: usize = 39;
    /// Trainer house.
    pub const TRAINER_HOUSE: usize = 40;
    /// PC storage: the 18 boxes. The only block of chunk 1.
    pub const PCSTORAGE: usize = 41;
}

/// Number of save blocks (`SAVE_BLOCK_NUM`).
pub const SAVE_BLOCK_NUM: usize = 42;

/// Raw struct sizes of the 42 blocks, in block-id order — the returns
/// of the retail `gSaveChunkHeaders` size stubs (`src/save_arrays.c`),
/// each one a pinned function the harness calls out of the ROM.
///
/// PCStorage alone (0x122FC) is over a third of the dynamic region;
/// its value also falls out of pret's `struct PokemonStorageSystem`
/// arithmetic exactly (18 boxes × 0x1000 + 8 + 18 × 40 + 18 + 17).
pub const BLOCK_RAW_SIZES: [u32; SAVE_BLOCK_NUM] = [
    0x5C,    // SYSINFO            Save_SysInfo_sizeof
    0x2C,    // PLAYERDATA         Save_PlayerData_sizeof
    0x5B0,   // PARTY              SaveArray_Party_sizeof
    0x79C,   // BAG                Save_Bag_sizeof
    0x44C,   // FLAGS              Save_VarsFlags_sizeof
    0x80,    // LOCAL_FIELD_DATA   Save_LocalFieldData_sizeof
    0x340,   // POKEDEX            Save_Pokedex_sizeof
    0x1E0,   // DAYCARE            Save_Daycare_sizeof
    0x880,   // PALPAD             Save_PalPad_sizeof
    0x2E0,   // MISC               Save_Misc_sizeof
    0x1400,  // MAP_OBJECTS       Save_MapObjects_sizeof
    0x20,    // LINK_BATTLE_RULESET Save_LinkBattleRuleset_sizeof
    0x834,   // DRESSUP_DATA       Save_FashionData_sizeof
    0x460,   // MAILBOX            Save_Mailbox_sizeof
    0x108,   // FRIEND_GROUP       Save_FriendGroup_sizeof
    0x620,   // TRAINER_CARD       Save_TrainerCard_sizeof
    0x1C0,   // GAMESTATS          GameStats_sizeof
    0x170,   // SEAL_CASE          Save_SealCase_sizeof
    0x3EC,   // CHATOT             Save_Chatot_sizeof
    0x1628,  // UNK_19            Save_Frontier_sizeof
    0xE,     // SPECIAL_RIBBONS     Save_SpecialRibbons_sizeof
    0x68,    // ROAMER              Save_Roamers_sizeof
    0xF8,    // UNK_22              sub_0202DB40
    0xBC8,   // UNK_23              sub_0202E41C
    0xEA0,   // RANKINGS            Save_Rankings_sizeof
    0x8C0,   // UNK_25              sub_0202C034
    0xFF8,   // WIFI_HISTORY        Save_WiFiHistory_sizeof
    0x1680,  // MYSTERY_GIFT      Save_MysteryGift_sizeof
    0x688,   // UNK_28              MigratedPokemon_GetSize
    0x28,    // POKEATHLON_FRIENDSHIP_RECORDS
    0x8,     // EASY_CHAT            Save_EasyChat_sizeof
    0x40,    // UNK_31               sub_0203170C
    0x8,     // UNK_32               sub_020318C8
    0x8,     // FOLLOW_MON           Save_FollowMon_sizeof
    0x658,   // POKEGEAR            SaveData_Pokegear_sizeof
    0x5FC,   // SAFARI_ZONE         Save_SafariZone_sizeof
    0x1294,  // PHOTO_ALBUM        Save_PhotoAlbum_sizeof
    0xB80,   // POKEATHLON           PokeathlonSave_sizeof
    0x80,    // APRICORN_BOX         Save_ApricornBox_sizeof
    0x134,   // POKEWALKER           Pokewalker_sizeof
    0xF00,   // TRAINER_HOUSE       Save_TrainerHouse_sizeof
    0x122FC, // PCSTORAGE         PCStorage_sizeof
];

/// One block's placement in the dynamic region — the load-time
/// `struct SaveArrayHeader` (`include/save.h`), plus the raw struct
/// size the header's own `size` field derives from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockLayout {
    /// The block id (`SAVE_*`).
    pub id: usize,
    /// The chunk the block belongs to: 0 = main, 1 = PC storage.
    pub slot: u8,
    /// The block's raw struct size (`GetSaveChunkSizePlusCRC`'s base).
    pub raw_size: u32,
    /// The block's stored size: the struct padded to 4 bytes, plus the
    /// u16 CRC and 2 bytes of tail padding (`((size + 3) & ~3) + 4`).
    pub size: u32,
    /// The block's offset in the dynamic region (and, given a slot
    /// base, in the card backup).
    pub offset: u32,
}

/// Where a block's stored CRC16 sits: after the padded data, before
/// the 2 tail bytes (`SaveSubstruct_UpdateCRC`:
/// `data_u16[size / 2] = crc` with `size = stored - 4`).
#[must_use]
pub fn block_crc_offset(block: &BlockLayout) -> usize {
    block.offset as usize + block.size as usize - 4
}

/// The block layout, computed at compile time by the original's own
/// boot arithmetic (`SaveData_InitSubstructs`, ported verbatim below).
pub const BLOCKS: [BlockLayout; SAVE_BLOCK_NUM] = build_block_layout();

/// A whole save chunk's placement — the load-time `struct
/// SaveSlotSpec`: chunk 0 (main, blocks 0–40) and chunk 1 (PC storage,
/// block 41).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotSpec {
    /// The chunk id: 0 main, 1 PC storage.
    pub id: u8,
    /// The chunk's offset in the dynamic region.
    pub offset: u32,
    /// The chunk's total stored size, footer included.
    pub size: u32,
    /// The first flash page the chunk occupies.
    pub first_page: u8,
    /// How many 4-KiB flash pages the chunk occupies.
    pub num_pages: u8,
}

/// The two chunk placements (`SaveData_InitSlotSpecs`, ported
/// verbatim).
pub const SLOT_SPECS: [SlotSpec; 2] = build_slot_specs();

/// The end of the last chunk in the dynamic region — everything past
/// it inside [`DYNAMIC_REGION_SIZE`] is padding.
pub const USED_REGION_END: usize = SLOT_SPECS[1].offset as usize + SLOT_SPECS[1].size as usize;

/// `((size + 3) & ~3) + 4` — `GetSaveChunkSizePlusCRC`: a block's
/// stored size is its struct padded to 4 bytes, plus a u16 CRC and
/// 2 tail bytes.
#[must_use]
pub const fn block_stored_size(raw_size: u32) -> u32 {
    ((raw_size + 3) & !3) + 4
}

/// The boot-time block layout pass (`SaveData_InitSubstructs`),
/// verbatim: walk the blocks in id order, accumulate offsets, and at
/// every chunk boundary (and at the last block) append the chunk
/// footer, aligning chunk 1 to 0x100.
///
/// The original reads `hdr[i + 1]` past the table's end at the last
/// block — guarded by `i + 1 < gNumSaveChunkHeaders` in the alignment
/// condition, so the out-of-bounds value never flows anywhere; this
/// port simply skips the read.
const fn build_block_layout() -> [BlockLayout; SAVE_BLOCK_NUM] {
    let mut blocks = [BlockLayout {
        id: 0,
        slot: 0,
        raw_size: 0,
        size: 0,
        offset: 0,
    }; SAVE_BLOCK_NUM];
    let mut adrs: u32 = 0;
    let mut i = 0;
    while i < SAVE_BLOCK_NUM {
        let raw = BLOCK_RAW_SIZES[i];
        let size = block_stored_size(raw);
        // The slot column of gSaveChunkHeaders: 0 for ids 0..=40,
        // 1 for PCSTORAGE.
        let slot: u8 = if i == block::PCSTORAGE { 1 } else { 0 };
        blocks[i] = BlockLayout {
            id: i,
            slot,
            raw_size: raw,
            size,
            offset: adrs,
        };
        adrs += size;
        // The chunk boundary test must see block i+1's *slot*, which
        // for not-yet-placed blocks is a function of the id, not the
        // half-built array: PCSTORAGE (and only it) lives in chunk 1.
        let boundary =
            i + 1 == SAVE_BLOCK_NUM || slot != if i + 1 == block::PCSTORAGE { 1 } else { 0 };
        if boundary {
            adrs += FOOTER_SIZE as u32;
            // Align chunk 1's start to 0x100 — only at the inner
            // boundary, never past the last block (the i + 1 < N guard).
            if i + 1 < SAVE_BLOCK_NUM && adrs % 0x100 != 0 {
                adrs += 0x100 - adrs % 0x100;
            }
        }
        i += 1;
    }
    blocks
}

/// The boot-time chunk pass (`SaveData_InitSlotSpecs`), verbatim: sum
/// each chunk's block sizes, add the footer, and align the running
/// address to 0x100 between chunks.
const fn build_slot_specs() -> [SlotSpec; 2] {
    let mut specs = [
        SlotSpec {
            id: 0,
            offset: 0,
            size: 0,
            first_page: 0,
            num_pages: 0,
        },
        SlotSpec {
            id: 1,
            offset: 0,
            size: 0,
            first_page: 0,
            num_pages: 0,
        },
    ];
    let mut adrs: u32 = 0;
    let mut j = 0;
    let mut i = 0;
    let mut npage: u8 = 0;
    while i < 2 {
        let mut size: u32 = 0;
        while j < SAVE_BLOCK_NUM && BLOCKS[j].slot as usize == i {
            size += BLOCKS[j].size;
            j += 1;
        }
        size += FOOTER_SIZE as u32;
        specs[i] = SlotSpec {
            id: i as u8,
            offset: adrs,
            size,
            first_page: npage,
            num_pages: ((size + SAVE_SECTOR_SIZE as u32 - 1) / SAVE_SECTOR_SIZE as u32) as u8,
        };
        npage += specs[i].num_pages;
        adrs += size;
        if adrs % 0x100 != 0 {
            adrs += 0x100 - adrs % 0x100;
        }
        i += 1;
    }
    specs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every value below was derived from the retail image (the pinned
    /// size stubs, via arm-runner) and pret's struct arithmetic —
    /// then cross-checked as a whole: the chunks must exactly fill the
    /// cartridge's 35-page save window, which no wrong table can do.
    #[test]
    fn layout_fills_the_flash_budget_exactly() {
        // The dynamic region window.
        assert_eq!(DYNAMIC_REGION_SIZE, 0x23000);
        // ...which the chunks exactly tile: 16 pages of main + 19 of PC.
        let total: usize = SLOT_SPECS.iter().map(|s| s.num_pages as usize).sum();
        assert_eq!(total, SAVE_PAGE_MAX);
        assert_eq!(SLOT_SPECS[0].num_pages, 16);
        assert_eq!(SLOT_SPECS[1].num_pages, 19);
        // And nothing spills past the window.
        assert!(USED_REGION_END <= DYNAMIC_REGION_SIZE);
        assert_eq!(USED_REGION_END, 0x21A10);
    }

    #[test]
    fn chunk_placements_match_the_originals_arithmetic() {
        // Main chunk: blocks 0..=40 + footer, from the very start.
        assert_eq!(SLOT_SPECS[0].offset, 0);
        let main_sum: u32 = BLOCKS[..block::PCSTORAGE].iter().map(|b| b.size).sum();
        assert_eq!(SLOT_SPECS[0].size, main_sum + FOOTER_SIZE as u32);

        // PC chunk: PCSTORAGE alone + footer, 0x100-aligned after main.
        assert_eq!(SLOT_SPECS[1].offset, 0xF700);
        assert_eq!(SLOT_SPECS[1].offset, BLOCKS[block::PCSTORAGE].offset);
        assert_eq!(
            SLOT_SPECS[1].size,
            BLOCKS[block::PCSTORAGE].size + FOOTER_SIZE as u32
        );

        // Both footers sit at chunk_end - 16, inside their chunks.
        for (spec, blocks) in [
            (SLOT_SPECS[0], &BLOCKS[..block::PCSTORAGE] as &[BlockLayout]),
            (SLOT_SPECS[1], &BLOCKS[block::PCSTORAGE..] as &[BlockLayout]),
        ] {
            let last = blocks.last().expect("chunk is never empty");
            // Block offsets are dynamic-region-wide, so the last
            // block's end plus the footer is the chunk's absolute end.
            assert_eq!(
                last.offset + last.size + FOOTER_SIZE as u32,
                spec.offset + spec.size
            );
        }
    }

    #[test]
    fn block_regions_pad_to_four_and_carry_a_crc_slot() {
        // GetSaveChunkSizePlusCRC: align4(raw) + 4 — even the 0xE-byte
        // SPECIAL_RIBBONS block grows a 0x14-byte region, and the
        // 0x3EC CHATOT (already 4-aligned) gets exactly +4.
        assert_eq!(block_stored_size(0xE), 0x14);
        assert_eq!(block_stored_size(0x3EC), 0x3F0);
        for block in BLOCKS {
            assert_eq!(block.size % 4, 0);
            assert!(block.raw_size <= block.size - 4 && block.size - 4 < block.raw_size + 4);
        }
        // The CRC of the first block sits at its padded end, then 2
        // tail bytes before the next block.
        let sysinfo = &BLOCKS[block::SYSINFO];
        assert_eq!(sysinfo.offset, 0);
        assert_eq!(block_crc_offset(sysinfo), 0x5C);
        assert_eq!(BLOCKS[block::PLAYERDATA].offset, 0x5C + 4);
    }

    #[test]
    fn pcstorage_size_matches_prets_struct_arithmetic() {
        // struct PokemonStorageSystem (include/pokemon_storage_system.h):
        // 18 boxes x 0x1000 (30 BoxPokemon x 0x88 + 16) + curBox (4)
        // + boxModifiedFlag (4) + 18 x 20 u16 names + 18 wallpapers +
        // unlockedWallpaper + 0x11 filler.
        let from_structs = 18 * 0x1000 + 4 + 4 + 18 * 20 * 2 + 18 + 1 + 0x11;
        assert_eq!(from_structs, 0x122FC);
        assert_eq!(BLOCK_RAW_SIZES[block::PCSTORAGE], 0x122FC);
    }

    #[test]
    fn block_ids_index_the_table_in_order() {
        for (i, block) in BLOCKS.iter().enumerate() {
            assert_eq!(block.id, i, "gSaveChunkHeaders is id-ordered");
            assert_eq!(
                block.raw_size, BLOCK_RAW_SIZES[i],
                "layout and size table agree"
            );
        }
        // Only PCSTORAGE belongs to chunk 1.
        assert!(BLOCKS[..block::PCSTORAGE].iter().all(|b| b.slot == 0));
        assert_eq!(BLOCKS[block::PCSTORAGE].slot, 1);
    }
}
