//! `save::vars_flags` against the new-game defaults: the typed view
//! must read the two entries `NewGame_InitSaveData` writes into block 4
//! (`VAR_MAGIKARP_SIZE_RECORD` = 56150, `FLAG_UNK_960` set) exactly
//! where `save::new_game` puts them by raw offset — which pins the
//! `SaveVarsFlags` layout (vars at 0, flags at 0x2E0) against code that
//! was itself measured out of the ROM.
//!
//! The ROM-gated half skips silently when `hg_usa.nds` is absent.

use apricorn_core::assets::AssetStore;
use apricorn_core::rng::Mt19937;
use apricorn_core::rtc::RtcDateTime;
use apricorn_core::save::block;
use apricorn_core::save::new_game::{ConsoleProfile, NewGameData};
use apricorn_core::save::vars_flags::{
    FLAG_UNK_960, FLAGS_OFFSET, NUM_FLAGS, NUM_VARS, SIZE, VAR_BASE, VAR_MAGIKARP_SIZE_RECORD,
    VARS_END, VarsFlags,
};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

#[test]
fn new_game_block_4_reads_through_the_typed_view() {
    let path = std::path::Path::new(ROM_PATH);
    if !path.exists() {
        eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
        return;
    }
    let store = AssetStore::open(path).unwrap();
    let rtc = RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0);
    let mut mt = Mt19937::new(12345);
    let data =
        NewGameData::initialize(&store, rtc, 287, &mut mt, ConsoleProfile::default()).unwrap();
    let raw = data.block(block::FLAGS).unwrap();
    assert!(raw.len() >= SIZE);
    let view = VarsFlags::new(raw).unwrap();

    // The fishing record: var 0x4035 at byte 0x6A.
    assert_eq!(view.var(VAR_MAGIKARP_SIZE_RECORD), Some(56150));
    assert_eq!(view.var_index(0x35), Some(56150));
    assert_eq!(&raw[0x6A..0x6C], &56150u16.to_le_bytes());
    // The one flag: 0x960 at byte 0x2E0 + 0x12C, bit 0.
    assert!(view.flag(FLAG_UNK_960));
    assert_eq!(raw[FLAGS_OFFSET + usize::from(FLAG_UNK_960 / 8)] & 1, 1);
    assert_eq!(raw[0x40C] & 1, 1);

    // Nothing else in the block: every other var is 0, every other flag
    // clear, so the two entries are the whole of the new game's block 4.
    let others_set: Vec<u16> = (VAR_BASE..=VARS_END)
        .filter(|&v| v != VAR_MAGIKARP_SIZE_RECORD && view.var(v) != Some(0))
        .collect();
    assert!(others_set.is_empty(), "unexpected vars: {others_set:x?}");
    let flags_set: Vec<u16> = (1..NUM_FLAGS as u16).filter(|&f| view.flag(f)).collect();
    assert_eq!(flags_set, vec![FLAG_UNK_960]);
    assert_eq!(NUM_VARS * 2, FLAGS_OFFSET);
}

#[test]
fn view_over_a_borrowed_block_writes_back() {
    let mut raw = vec![0u8; SIZE + 4];
    {
        let mut view = VarsFlags::new(raw.as_mut_slice()).unwrap();
        assert!(view.set_var(VAR_MAGIKARP_SIZE_RECORD, 56150));
        assert!(view.set_flag(FLAG_UNK_960));
    }
    assert_eq!(&raw[0x6A..0x6C], &56150u16.to_le_bytes());
    assert_eq!(raw[0x40C], 1);
    let view = VarsFlags::new(raw.as_slice()).unwrap();
    assert_eq!(view.var(VAR_MAGIKARP_SIZE_RECORD), Some(56150));
    assert!(view.flag(FLAG_UNK_960));
    // The padding/CRC tail past SIZE is never touched.
    assert_eq!(&raw[SIZE..], &[0, 0, 0, 0]);
}
