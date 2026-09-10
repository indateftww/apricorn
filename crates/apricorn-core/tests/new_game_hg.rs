//! New-game handoff: identity, RNG order, ancillary data and card roundtrip.
use apricorn_core::{
    assets::AssetStore,
    rng::{Lcrng, Mt19937, prandom},
    rtc::RtcDateTime,
    save::{
        SaveData, block,
        new_game::{ConsoleProfile, NewGameData},
    },
    text::string::GameString,
};
#[test]
fn post_oak_state_survives_retail_card_roundtrip() {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds"));
    if !path.exists() {
        return;
    }
    let store = AssetStore::open(path).unwrap();
    let rtc = RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0);
    let console = ConsoleProfile {
        rtc_offset: -1234,
        mac: [1, 2, 3, 4, 5, 6],
        birthday: [3, 14],
        auth_id: 0x123456789ab,
        entropy: 0x87654321,
    };
    for gender in 0..2 {
        let mut mt = Mt19937::new(12345);
        let mut expected_mt = mt.clone();
        let mut data = NewGameData::initialize(&store, rtc, 287, &mut mt, console).unwrap();
        assert_eq!(&data.block(0).unwrap()[..16], &[0; 16]);
        let u32at = |b: &[u8], p: usize| u32::from_le_bytes(b[p..p + 4].try_into().unwrap());
        for i in 0..2 {
            assert_eq!(
                u32at(data.block(block::ROAMER).unwrap(), i * 4),
                expected_mt.next_u32()
            );
        }
        assert_eq!(mt, expected_mt);
        assert_eq!(data.money(), 3000);
        assert_eq!(data.position(), [64, u32::MAX, 6, 6, 1]);
        let flags = data.block(block::FLAGS).unwrap();
        assert_eq!(
            u16::from_le_bytes(flags[0x6a..0x6c].try_into().unwrap()),
            56150
        );
        assert_eq!(flags[0x40c] & 1, 1);
        mt.set_seed(98765);
        expected_mt.set_seed(98765);
        let group = expected_mt.next_u32();
        let id = expected_mt.next_u32();
        let mut lc = Lcrng::new(24680);
        let mut expected_lc = lc;
        expected_lc.next_u16();
        expected_lc.next_u16();
        let name = GameString::from_units(&[311, 299, 318, 318, 317]);
        data.finish_oak(&name, gender, rtc, &mut mt, &mut lc, console);
        let system = data.block(0).unwrap();
        assert_eq!(&system[..8], &console.rtc_offset.to_le_bytes());
        assert_eq!(&system[8..14], &console.mac);
        assert_eq!(&system[14..16], &console.birthday);
        assert_eq!(u32at(system, 20), 10, "SDK RTC year is relative to 2000");
        let profile = data.block(1).unwrap();
        assert_eq!(profile[0x1c], gender);
        assert_eq!(&profile[4..14], &[55, 1, 43, 1, 62, 1, 62, 1, 61, 1]);
        assert_eq!(data.trainer_id(), id);
        assert!(data.oak_complete);
        assert_eq!(u32at(data.block(block::FRIEND_GROUP).unwrap(), 80), group);
        assert_eq!(
            u32at(data.block(block::FRIEND_GROUP).unwrap(), 84),
            prandom(group)
        );
        for i in 0..10 {
            assert_eq!(
                u32at(data.block(block::POKEWALKER).unwrap(), 0xfc + i * 4),
                expected_mt.next_u32()
            );
        }
        assert_eq!(mt, expected_mt);
        assert_eq!(lc, expected_lc);
        let mail = data.block(block::MAILBOX).unwrap();
        assert_eq!(mail[4], 1 - gender);
        assert_eq!(mail[7], 9);
        assert_eq!(
            u16::from_le_bytes(mail[24..26].try_into().unwrap()),
            183 | (2 << 12)
        );
        let snapshot = data.snapshot();
        for i in 0..42 {
            assert!(snapshot.block_crc_ok(i), "block {i}");
            assert_eq!(snapshot.block(i), data.block(i).unwrap());
        }
        let mut loaded = SaveData::parse(snapshot.to_bytes()).unwrap();
        assert_eq!(loaded.to_bytes(), snapshot.to_bytes());
        loaded.save_game();
        let loaded = SaveData::parse(loaded.to_bytes()).unwrap();
        for i in 0..42 {
            assert_eq!(loaded.block(i), data.block(i).unwrap());
        }
    }
}
