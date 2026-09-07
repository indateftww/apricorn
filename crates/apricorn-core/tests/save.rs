//! The save container, exercised end to end on synthesized
//! retail-shaped card backups (`PLAN.md` Phase 4, step 4).
//!
//! The blobs are built the way the original writes them — block
//! regions with per-block CRCs, chunk footers sealed over their whole
//! chunk, extra chunks with `SaveArrayFooter`s — so every test drives
//! the real probe logic, not a mock of it: the boot matrix of
//! `Save_GetSaveFilesStatus`, the counter wraparound quirk, the
//! alternating-slot `save_game`, and the byte-identical round-trip
//! that is this step's exit criterion.
//!
//! The constants these blobs are built from (block sizes, chunk
//! placements, CRC) are themselves locked to the retail image by
//! `apricorn-harness`'s `tests/save_hg.rs`, and the last test here
//! takes a real retail `hg.sav` dump at the repo root when one
//! exists (same skip-silently policy as the ROM-gated tests).

use apricorn_core::rng::Lcrng;
use apricorn_core::save::{
    BLOCKS, CARD_BACKUP_SIZE, DYNAMIC_REGION_SIZE, EXTRA_CHUNK_SECTORS, EXTRA_CHUNK_SIZES,
    FOOTER_SIZE, SAVE_CHUNK_MAGIC, SLOT_SPECS, SLOT_STRIDE, SaveData, SaveError, block,
    block_crc_offset, crc16,
};

/// The retail `.sav` the last test loads when present (gitignored,
/// like the ROM — supply your own dump).
const SAV_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg.sav");

/// Builds one slot's dynamic region the way the original writes one:
/// every block's stored region filled with pseudo-random bytes, its
/// trailing CRC computed over them, and both chunk footers sealed
/// with the given save counters.
fn build_window(seed: u32, main_count: u32, pc_count: u32) -> Vec<u8> {
    let mut region = vec![0u8; DYNAMIC_REGION_SIZE];
    for layout in BLOCKS {
        // The CRC-covered span: data, padding, and the CRC slot
        // itself up to the two tail bytes.
        let start = layout.offset as usize;
        let crc_at = block_crc_offset(&layout);
        let mut rng = Lcrng::new(seed ^ u32::try_from(layout.id).expect("id").wrapping_mul(0x7919));
        for byte in &mut region[start..crc_at + 2] {
            *byte = rng.next_u16() as u8;
        }
        let crc = crc16(&region[start..crc_at]);
        region[crc_at..crc_at + 2].copy_from_slice(&crc.to_le_bytes());
    }
    for (spec, count) in [(SLOT_SPECS[0], main_count), (SLOT_SPECS[1], pc_count)] {
        seal_chunk(&mut region, &spec, count);
    }
    region
}

/// Writes a chunk footer at the end of `spec`'s region with the same
/// fields `SaveSlot_BuildFooter` stamps.
fn seal_chunk(region: &mut [u8], spec: &apricorn_core::save::SlotSpec, count: u32) {
    let at = spec.offset as usize + spec.size as usize - FOOTER_SIZE;
    let body = spec.offset as usize..spec.offset as usize + spec.size as usize - FOOTER_SIZE;
    let crc = crc16(&region[body]);
    region[at..at + 4].copy_from_slice(&count.to_le_bytes());
    region[at + 4..at + 8].copy_from_slice(&spec.size.to_le_bytes());
    region[at + 8..at + 12].copy_from_slice(&SAVE_CHUNK_MAGIC.to_le_bytes());
    region[at + 12..at + 14].copy_from_slice(&u16::from(spec.id).to_le_bytes());
    region[at + 14..at + 16].copy_from_slice(&crc.to_le_bytes());
}

/// A full card backup with the two slot generations carrying the
/// given `(main counter, PC counter)` pairs.
fn synth_save(slot0: (u32, u32), slot1: (u32, u32)) -> Vec<u8> {
    let mut blob = vec![0xFFu8; CARD_BACKUP_SIZE];
    blob[..DYNAMIC_REGION_SIZE].copy_from_slice(&build_window(0x1000, slot0.0, slot0.1));
    blob[SLOT_STRIDE..SLOT_STRIDE + DYNAMIC_REGION_SIZE]
        .copy_from_slice(&build_window(0x2000, slot1.0, slot1.1));
    blob
}

/// `FlashClobberChunkFooter`: invalidates a chunk by 0xFF-filling its
/// footer, exactly what the original does before rewriting a sector.
fn clobber_footer(blob: &mut [u8], slot: usize, chunk: usize) {
    let spec = SLOT_SPECS[chunk];
    let at = slot * SLOT_STRIDE + spec.offset as usize + spec.size as usize - FOOTER_SIZE;
    blob[at..at + FOOTER_SIZE].fill(0xFF);
}

/// Writes an extra chunk copy (raw bytes + `SaveArrayFooter`, the
/// fields `CreateChunkFooter` stamps) at `base`.
fn plant_extra_copy(blob: &mut [u8], base: usize, id: usize, saveno: u32, raw: &[u8]) {
    let at = base;
    blob[at..at + raw.len()].copy_from_slice(raw);
    let footer = at + raw.len();
    blob[footer..footer + 4].copy_from_slice(&SAVE_CHUNK_MAGIC.to_le_bytes());
    blob[footer + 4..footer + 8].copy_from_slice(&saveno.to_le_bytes());
    blob[footer + 8..footer + 12].copy_from_slice(&(raw.len() as u32).to_le_bytes());
    blob[footer + 12..footer + 14].copy_from_slice(&(id as u16).to_le_bytes());
    // The CRC covers the raw bytes plus every footer field up to the
    // crc itself (size + offsetof(struct SaveArrayFooter, crc)).
    let mut covered = raw.to_vec();
    covered.extend_from_slice(&blob[footer..footer + 14]);
    blob[footer + 14..footer + 16].copy_from_slice(&crc16(&covered).to_le_bytes());
}

/// Raw bytes for an extra chunk, pseudo-random per seed.
fn extra_raw(id: usize, seed: u32) -> Vec<u8> {
    let mut rng = Lcrng::new(seed);
    (0..EXTRA_CHUNK_SIZES[id])
        .map(|_| rng.next_u16() as u8)
        .collect()
}

// ===== Load + round-trip ================================================

#[test]
fn loads_and_round_trips_byte_identically() {
    let blob = synth_save((5, 5), (7, 7));
    let save = SaveData::parse(&blob).expect("a well-formed card loads");
    // Slot 1's generation is the newer one.
    assert_eq!(save.counter(), 7);
    assert_eq!(save.last_good_sector(), 1);
    assert!(!save.slot_degraded());
    // The exit criterion: the blob re-emits unchanged, and re-parsing
    // the emission lands on the same generation.
    assert_eq!(save.to_bytes(), blob);
    let again = SaveData::parse(save.to_bytes()).expect("re-parse");
    assert_eq!(again.counter(), save.counter());
    assert_eq!(again.last_good_sector(), save.last_good_sector());
}

#[test]
fn the_newest_generation_wins_including_the_counter_wraparound() {
    // Plain ordering, both directions.
    let save = SaveData::parse(&synth_save((9, 9), (4, 4))).expect("load");
    assert_eq!((save.counter(), save.last_good_sector()), (9, 0));
    let save = SaveData::parse(&synth_save((4, 4), (9, 9))).expect("load");
    assert_eq!((save.counter(), save.last_good_sector()), (9, 1));

    // The wraparound quirk: a clobbered-all-0xFF footer reads as
    // 0xFFFFFFFF, which counts as *older* than a wrapped-around 0.
    let save = SaveData::parse(&synth_save((0, 0), (u32::MAX, u32::MAX))).expect("load");
    assert_eq!((save.counter(), save.last_good_sector()), (0, 0));
    let save = SaveData::parse(&synth_save((u32::MAX, u32::MAX), (0, 0))).expect("load");
    assert_eq!((save.counter(), save.last_good_sector()), (0, 1));
}

// ===== The boot probe's status matrix ===================================

#[test]
fn the_status_matrix_of_save_getsavefilesstatus() {
    // Both slots good and agreeing: the normal case.
    let save = SaveData::parse(&synth_save((5, 5), (7, 7))).expect("load");
    assert!(!save.slot_degraded());

    // Both good, but the newer generation's chunks disagree on their
    // counter (a torn write): fall back to the older, degraded.
    let blob = synth_save((5, 5), (6, 9));
    let save = SaveData::parse(&blob).expect("the older generation still loads");
    assert_eq!(save.counter(), 5);
    assert_eq!(save.last_good_sector(), 0);
    assert!(save.slot_degraded());

    // Main of one slot lost, both PC chunks good, and the survivor
    // agrees with the same slot's PC: degraded load of it.
    let mut blob = synth_save((0, 0), (6, 6));
    clobber_footer(&mut blob, 0, 0);
    let save = SaveData::parse(&blob).expect("the surviving main chunk loads");
    assert_eq!((save.counter(), save.last_good_sector()), (6, 1));
    assert!(save.slot_degraded());

    // PC of the newer slot lost: the main chunk that still agrees
    // with the older slot's PC chunk is the load, degraded.
    let mut blob = synth_save((5, 5), (6, 6));
    clobber_footer(&mut blob, 1, 1);
    let save = SaveData::parse(&blob).expect("the older agreement loads");
    assert_eq!((save.counter(), save.last_good_sector()), (5, 0));
    assert!(save.slot_degraded());

    // Exactly one good main and one good PC, in the same slot: that
    // slot loads clean (the original returns IS_GOOD here).
    let mut blob = synth_save((5, 5), (7, 7));
    clobber_footer(&mut blob, 1, 0);
    clobber_footer(&mut blob, 1, 1);
    let save = SaveData::parse(&blob).expect("the one whole slot loads");
    assert_eq!((save.counter(), save.last_good_sector()), (5, 0));
    assert!(!save.slot_degraded());

    // Exactly one good main and one good PC, in *different* slots:
    // nothing agrees, nothing loads.
    let mut blob = synth_save((5, 5), (7, 7));
    clobber_footer(&mut blob, 0, 1);
    clobber_footer(&mut blob, 1, 0);
    assert_eq!(SaveData::parse(&blob).unwrap_err(), SaveError::Corrupt);

    // A chunk kind lost in *both* slots: the card is refused.
    let mut blob = synth_save((5, 5), (7, 7));
    clobber_footer(&mut blob, 0, 0);
    clobber_footer(&mut blob, 1, 0);
    assert_eq!(SaveData::parse(&blob).unwrap_err(), SaveError::Corrupt);
    let mut blob = synth_save((5, 5), (7, 7));
    clobber_footer(&mut blob, 0, 1);
    clobber_footer(&mut blob, 1, 1);
    assert_eq!(SaveData::parse(&blob).unwrap_err(), SaveError::Corrupt);
}

// ===== save_game: the alternating-slot write =============================

#[test]
fn save_game_alternates_slots_and_keeps_the_previous_generation() {
    let blob = synth_save((5, 5), (7, 7));
    let mut save = SaveData::parse(&blob).expect("load");
    assert_eq!(save.last_good_sector(), 1);

    save.save_game();

    // Counter bumped, sector flipped, the save re-loads clean.
    assert_eq!(save.counter(), 8);
    assert_eq!(save.last_good_sector(), 0);
    let blob = save.to_bytes();
    let reloaded = SaveData::parse(blob).expect("the new generation re-loads");
    assert_eq!(reloaded.counter(), 8);
    assert_eq!(reloaded.last_good_sector(), 0);
    assert!(!reloaded.slot_degraded());

    // The previous generation (slot 1) is untouched — the crash
    // protection the two-slot scheme exists for.
    assert_eq!(
        &blob[SLOT_STRIDE..SLOT_STRIDE + DYNAMIC_REGION_SIZE],
        &synth_save((5, 5), (7, 7))[SLOT_STRIDE..SLOT_STRIDE + DYNAMIC_REGION_SIZE],
        "the previous slot mirror must survive a save"
    );

    // And the new generation carries the same block bytes under a
    // rebuilt footer (SaveSlot_BuildFooter with the bumped counter):
    // compare against slot 1 of the pristine blob it was copied from.
    let fresh = synth_save((5, 5), (7, 7));
    for layout in BLOCKS {
        let offset = SLOT_STRIDE + layout.offset as usize;
        assert_eq!(
            reloaded.block(layout.id),
            &fresh[offset..offset + layout.raw_size as usize],
            "block {} bytes must copy through save_game",
            layout.id
        );
    }

    // The write alternates: the next save goes back to slot 1.
    let mut save = reloaded;
    save.save_game();
    assert_eq!(save.counter(), 9);
    assert_eq!(save.last_good_sector(), 1);
}

// ===== Block views and CRC upkeep ========================================

#[test]
fn block_views_slice_the_dynamic_region_and_upkeep_their_crcs() {
    let blob = synth_save((2, 2), (1, 1));
    let mut save = SaveData::parse(&blob).expect("load");

    // Every block's stored CRC matches its bytes as written.
    for layout in BLOCKS {
        assert!(save.block_crc_ok(layout.id), "block {}", layout.id);
    }
    assert_eq!(save.block(block::SYSINFO).len(), 0x5C);
    assert_eq!(save.block(block::PCSTORAGE).len(), 0x122FC);

    // An edit breaks the stored CRC until SaveSubstruct_UpdateCRC's
    // port recomputes it — and the fix is visible through to_bytes.
    save.block_mut(block::PLAYERDATA)[0] ^= 0xFF;
    assert!(!save.block_crc_ok(block::PLAYERDATA));
    save.update_block_crc(block::PLAYERDATA);
    assert!(save.block_crc_ok(block::PLAYERDATA));
    let blob = save.to_bytes();
    // PLAYERDATA lives at SYSINFO's stored end, 0x60 (see the layout
    // tests); the edit is visible straight through the blob.
    assert_eq!(blob[0x60], save.block(block::PLAYERDATA)[0]);
}

// ===== Extra chunks ======================================================

#[test]
fn extra_chunks_read_like_readextrasavechunk() {
    let mut blob = synth_save((3, 3), (5, 5));
    let save = SaveData::parse(&blob).expect("load");

    // Nothing planted: neither copy validates.
    let none = save.extra_chunk(0);
    assert!(none.data().is_none());

    // Primary only: its saveno and bytes.
    let raw = extra_raw(0, 0x40);
    let primary = EXTRA_CHUNK_SECTORS[0] as usize * 0x1000;
    plant_extra_copy(&mut blob, primary, 0, 4, &raw);
    let save = SaveData::parse(&blob).expect("re-load");
    let chunk = save.extra_chunk(0);
    assert_eq!(chunk.data(), Some(&raw[..]));
    assert_eq!(chunk.saveno(), 4);

    // Mirror only.
    let raw = extra_raw(0, 0x41);
    let mirror = (EXTRA_CHUNK_SECTORS[0] as usize + 64) * 0x1000;
    plant_extra_copy(&mut blob, mirror, 0, 9, &raw);
    let save = SaveData::parse(&blob).expect("re-load");
    let chunk = save.extra_chunk(0);
    assert_eq!(chunk.data(), Some(&raw[..]));
    assert_eq!(chunk.saveno(), 9);

    // Both valid: the newer saveno wins, both directions.
    let newer_primary = extra_raw(0, 0x42);
    plant_extra_copy(&mut blob, primary, 0, 9, &newer_primary);
    let save = SaveData::parse(&blob).expect("re-load");
    let chunk = save.extra_chunk(0);
    assert_eq!(chunk.saveno(), 9);
    assert_eq!(chunk.data(), Some(&newer_primary[..]));

    let newer_mirror = extra_raw(0, 0x43);
    plant_extra_copy(&mut blob, mirror, 0, 10, &newer_mirror);
    let save = SaveData::parse(&blob).expect("re-load");
    let chunk = save.extra_chunk(0);
    assert_eq!(chunk.saveno(), 10);
    assert_eq!(chunk.data(), Some(&newer_mirror[..]));

    // The wraparound quirk governs the savenos too: a wrapped 0 beats
    // an all-0xFF clobbered footer.
    plant_extra_copy(&mut blob, primary, 0, u32::MAX, &raw);
    plant_extra_copy(&mut blob, mirror, 0, 0, &newer_mirror);
    let save = SaveData::parse(&blob).expect("re-load");
    assert_eq!(save.extra_chunk(0).saveno(), 0);
}

// ===== A real retail dump ================================================

#[test]
fn a_retail_sav_loads_and_round_trips() {
    let blob = match std::fs::read(SAV_PATH) {
        Ok(blob) => blob,
        Err(_) => {
            eprintln!("skipping: {SAV_PATH} not found (supply your own retail save dump)");
            return;
        }
    };
    // A real HeartGold card backup must load — not blank, not corrupt.
    let save = SaveData::parse(&blob).expect("the retail save must load");
    // The exit criterion, on real data: byte-identical round-trip.
    assert_eq!(save.to_bytes(), blob);
    // And the container can hold its own next generation: the save
    // the original would write next re-loads with the bumped counter.
    let before = (save.counter(), save.last_good_sector());
    let mut save = save;
    save.save_game();
    let reloaded = SaveData::parse(save.to_bytes()).expect("the rewritten card re-loads");
    assert_eq!(reloaded.counter(), before.0 + 1);
    assert_eq!(reloaded.last_good_sector(), 1 - before.1);
}
