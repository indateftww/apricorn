//! Phase 4, step 4: the save format (`apricorn_core::save`) locked to
//! the retail image, value by value.
//!
//! The container's two committed constant tables are exactly what the
//! original derives at boot, so both get a differential here:
//!
//! * **The block sizes.** Every `gSaveChunkHeaders` size stub is a
//!   pinned function; each one is called through arm-runner and its
//!   return must equal the committed `BLOCK_RAW_SIZES` entry (and the
//!   three extra-chunk stubs their `EXTRA_CHUNK_SIZES` entries). The
//!   chunk offsets need no such call — they are plain arithmetic over
//!   these sizes, and their tiling of the 35-page flash window is
//!   pinned by the core unit tests, which no wrong size table can
//!   satisfy.
//! * **The CRC.** `GF_CalcCRC16` is called on the original's own
//!   terms: `MATHi_CRC16InitTable` builds a real table in scratch
//!   memory with the CCITT polynomial, the game's `sCRC16TablePtr`
//!   global is pointed at it, and every test stream must hash to what
//!   `save::crc16` computes — including the exact vectors the core
//!   unit tests carry, so a constant can't be right in one place and
//!   wrong in the other.
//!
//! Runs only when `hg_usa.nds` sits at the repo root (same policy as
//! `rng_hg.rs`; CI has no ROM and skips silently). Every load
//! re-verifies the pin table, so a wrong dump fails before any call.

use apricorn_core::rng::Lcrng;
use apricorn_core::save::{BLOCK_RAW_SIZES, EXTRA_CHUNK_SIZES, crc16};
use apricorn_harness::arm::retail::RetailArm9;
use apricorn_harness::pins::{PinMode, PinTable};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Scratch RAM above the loaded image (ends 0x0211_1EF8) and below
/// the stack (0x0230_0000): the CRC table and the hashed buffers.
const CRC_TABLE: u32 = 0x0220_0000;
const BUFFER: u32 = 0x0221_0000;

/// `sCRC16TablePtr` (the game's global the original `GF_CalcCRC16`
/// reads its table pointer from).
const SCRATCH_TABLE_PTR_ADDR: u32 = 0x021D_15A4;

/// The CCITT polynomial `MATH_CRC16InitTable` seeds the internal
/// initializer with (`MATH_CRC16_CCITT_POLY`).
const CRC16_CCITT_POLY: u32 = 0x1021;

fn load() -> Option<RetailArm9> {
    let data = match std::fs::read(ROM_PATH) {
        Ok(data) => data,
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            return None;
        }
    };
    let arm9 = match RetailArm9::load(&data) {
        Ok(arm9) => arm9,
        Err(e) => panic!("retail ROM failed to load: {e}"),
    };
    Some(arm9)
}

/// Calls the pinned function `name` with `args`, returning r0.
fn call(arm9: &mut RetailArm9, name: &str, args: &[u32]) -> u32 {
    let table = PinTable::arm9();
    let pin = table.get(name).unwrap_or_else(|| panic!("pin {name}"));
    assert_ne!(pin.mode, PinMode::Data, "{name} is a data pin");
    let entry = pin.address | u32::from(pin.mode == PinMode::Thumb);
    let cpu = arm9.cpu();
    cpu.prepare_call(entry, args);
    let result = cpu
        .run_default()
        .unwrap_or_else(|e| panic!("{name} faulted: {e}"));
    result.r0
}

/// Plants `bytes` at `addr` in scratch RAM (zero-padded to a word).
fn write_bytes(arm9: &mut RetailArm9, addr: u32, bytes: &[u8]) {
    let mut padded = bytes.to_vec();
    while !padded.len().is_multiple_of(4) {
        padded.push(0);
    }
    for (i, word) in padded.chunks(4).enumerate() {
        let value = u32::from_le_bytes(word.try_into().expect("chunk of 4"));
        arm9.cpu()
            .mem_mut()
            .write32(addr + (4 * i) as u32, value)
            .expect("scratch write");
    }
}

// ===== The block-size table =============================================

/// The pinned size stub of each block, in block-id order —
/// `gSaveChunkHeaders`' `sizeFunc` column.
const BLOCK_SIZE_STUBS: [&str; 42] = [
    "Save_SysInfo_sizeof",
    "Save_PlayerData_sizeof",
    "SaveArray_Party_sizeof",
    "Save_Bag_sizeof",
    "Save_VarsFlags_sizeof",
    "Save_LocalFieldData_sizeof",
    "Save_Pokedex_sizeof",
    "Save_Daycare_sizeof",
    "Save_PalPad_sizeof",
    "Save_Misc_sizeof",
    "Save_MapObjects_sizeof",
    "Save_LinkBattleRuleset_sizeof",
    "Save_FashionData_sizeof",
    "Save_Mailbox_sizeof",
    "Save_FriendGroup_sizeof",
    "Save_TrainerCard_sizeof",
    "GameStats_sizeof",
    "Save_SealCase_sizeof",
    "Save_Chatot_sizeof",
    "Save_Frontier_sizeof",
    "Save_SpecialRibbons_sizeof",
    "Save_Roamers_sizeof",
    "sub_0202DB40",
    "sub_0202E41C",
    "Save_Rankings_sizeof",
    "sub_0202C034",
    "Save_WiFiHistory_sizeof",
    "Save_MysteryGift_sizeof",
    "MigratedPokemon_GetSize",
    "PokeathlonSave_FriendshipRecords_sizeof",
    "Save_EasyChat_sizeof",
    "sub_0203170C",
    "sub_020318C8",
    "Save_FollowMon_sizeof",
    "SaveData_Pokegear_sizeof",
    "Save_SafariZone_sizeof",
    "Save_PhotoAlbum_sizeof",
    "PokeathlonSave_sizeof",
    "Save_ApricornBox_sizeof",
    "Pokewalker_sizeof",
    "Save_TrainerHouse_sizeof",
    "PCStorage_sizeof",
];

#[test]
fn block_sizes_match_the_pinned_stubs() {
    let Some(mut arm9) = load() else { return };

    for (id, name) in BLOCK_SIZE_STUBS.into_iter().enumerate() {
        let got = call(&mut arm9, name, &[]);
        assert_eq!(
            got, BLOCK_RAW_SIZES[id],
            "{name} (block {id}) disagrees with the committed table"
        );
    }
}

#[test]
fn extra_chunk_sizes_match_the_pinned_stubs() {
    let Some(mut arm9) = load() else { return };

    // gExtraSaveChunkHeaders' sizeFunc column: the Hall of Fame, then
    // the battle-record chunk, then the one stub shared by the four
    // record chunks.
    assert_eq!(
        call(&mut arm9, "Save_HOF_sizeof", &[]),
        EXTRA_CHUNK_SIZES[0],
        "Save_HOF_sizeof"
    );
    assert_eq!(
        call(&mut arm9, "sub_020312A4", &[]),
        EXTRA_CHUNK_SIZES[1],
        "sub_020312A4"
    );
    let shared = call(&mut arm9, "sub_0202FBCC", &[]);
    for (id, size) in EXTRA_CHUNK_SIZES.iter().enumerate().skip(2) {
        assert_eq!(shared, *size, "extra chunk {id} shares sub_0202FBCC's size");
    }
}

// ===== The CRC ==========================================================

#[test]
fn crc16_matches_the_original_gf_calccrc16() {
    let Some(mut arm9) = load() else { return };

    // Build the CCITT table in scratch RAM exactly like GF_CRC16Init
    // would (the internal initializer takes the polynomial directly;
    // MATH_CRC16InitTable is a static inline over it), then point the
    // game's own global at it so GF_CalcCRC16 runs its real path.
    call(
        &mut arm9,
        "MATHi_CRC16InitTable",
        &[CRC_TABLE, CRC16_CCITT_POLY],
    );
    arm9.cpu()
        .mem_mut()
        .write32(SCRATCH_TABLE_PTR_ADDR, CRC_TABLE)
        .expect("sCRC16TablePtr write");

    // The vectors the core unit tests carry — same bytes, both
    // implementations — plus odd lengths (the update walks bytes),
    // the empty stream, and an LCG-generated flash page.
    let mut page = [0u8; 0x1000];
    let mut rng = Lcrng::new(0x1234);
    for word in page.chunks_mut(4) {
        let a = rng.next_u16().to_le_bytes();
        let b = rng.next_u16().to_le_bytes();
        word.copy_from_slice(&[a[0], a[1], b[0], b[1]]);
    }

    let streams: &[&[u8]] = &[
        b"123456789",
        b" HeartGold",
        &[0x00],
        &[0xFF; 4],
        &[0u8; 0x5C],
        &[0u8; 0xE], // odd length
        b"odd",      // odd length, nonzero
        &[],         // nothing hashed: the initial register
        &page,
    ];
    for bytes in streams {
        write_bytes(&mut arm9, BUFFER, bytes);
        let len = bytes.len() as u32;
        let got = call(&mut arm9, "GF_CalcCRC16", &[BUFFER, len]);
        let want = crc16(bytes);
        assert_eq!(
            got,
            u32::from(want),
            "GF_CalcCRC16 disagrees over {} bytes",
            bytes.len()
        );
    }
}
