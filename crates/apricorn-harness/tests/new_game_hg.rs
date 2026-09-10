//! New-game block defaults versus the retail gSaveChunkHeaders init table.
use apricorn_core::{
    assets::AssetStore,
    rng::Mt19937,
    rtc::RtcDateTime,
    save::{
        BLOCK_RAW_SIZES,
        new_game::{ConsoleProfile, NewGameData},
    },
};
use apricorn_harness::arm::retail::RetailArm9;
use std::path::Path;
const ROM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");
#[test]
fn new_game_block_defaults_match_retail_initializers() {
    if !Path::new(ROM).exists() {
        return;
    }
    let bytes = std::fs::read(ROM).unwrap();
    let store = AssetStore::open(Path::new(ROM)).unwrap();
    let state = NewGameData::initialize(
        &store,
        RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0),
        0,
        &mut Mt19937::uninitialized(),
        ConsoleProfile::default(),
    )
    .unwrap();
    for (id, &size) in BLOCK_RAW_SIZES.iter().enumerate() {
        let mut arm = RetailArm9::load(&bytes).unwrap();
        let cpu = arm.cpu();
        // Frozen GF_RTCWork: the SDK stores years since 2000.
        cpu.mem_mut().write32(0x021d1048, 1).unwrap();
        for (i, v) in [10, 3, 14, 0, 12, 0, 0].into_iter().enumerate() {
            cpu.mem_mut().write32(0x021d1058 + i as u32 * 4, v).unwrap();
        }
        // Redirect the inline OS vblank literal to controlled scratch RAM.
        cpu.mem_mut().write32(0x0202cf4c, 0x02210000).unwrap();
        // Boundary stubs supply a frozen console's entropy/authentication
        // words. DWC's own ID construction and CRC code still execute.
        let branch = cpu.mem().read32(0x0209ff44).unwrap();
        let offset = ((branch << 8) as i32) >> 6;
        let entropy = (0x0209ff4cu32).wrapping_add(offset as u32);
        for (address, words) in [(entropy, 8), (0x0209fa40, 5)] {
            cpu.mem_mut().write32(address, 0xe3a01000).unwrap(); // mov r1,#0
            for i in 0..words {
                cpu.mem_mut()
                    .write32(address + 4 + i * 4, 0xe5801000 + i * 4)
                    .unwrap();
            }
            cpu.mem_mut()
                .write32(address + 4 + words * 4, 0xe12fff1e)
                .unwrap();
        }
        // Isolate the init function from the SaveData singleton. This
        // wrapper only updates the outer block CRC (tested in save_hg).
        cpu.mem_mut().write16(0x0202893c, 0x4770).unwrap();
        let address = cpu.mem().read32(0x020f64c4 + id as u32 * 16 + 12).unwrap();
        cpu.prepare_call(address, &[0x02220000]);
        cpu.run(2_000_000)
            .unwrap_or_else(|e| panic!("block {id}: {e}"));
        let mut expected = cpu.mem().read_block(0x02220000, size).unwrap().to_vec();
        let actual = state.block(id).unwrap();
        // The independently tested overlay mutations, pinned RTC and ROM
        // message-loader effects happen after/around the raw initializer.
        let ranges: Vec<std::ops::Range<usize>> = match id {
            1 => vec![0x18..0x1c],
            4 => vec![0x6a..0x6c, 0x40c..0x40d],
            5 => vec![0..20],
            41 => vec![0x12008..0x122d8],
            _ => vec![],
        };
        for r in ranges {
            expected[r.clone()].copy_from_slice(&actual[r]);
        }
        let mismatch = expected.iter().zip(actual).position(|(a, b)| a != b);
        assert!(
            mismatch.is_none(),
            "block {id} first differs at {mismatch:?}: original {:?}, engine {:?}",
            mismatch.map(|p| expected[p]),
            mismatch.map(|p| actual[p])
        );
    }
}
