//! Integration tests: both sound data archives in a retail HeartGold
//! (US) dump.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM). The expected counts and records are ground truth
//! established by scanning the retail image while writing the parser —
//! see `docs/nitro-sdat.md`.

use apricorn_core::formats::{
    BankInfo, GroupItem, GroupItemKind, PlayerInfo, Sdat, SseqInfo, SwarInfo,
};
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail HeartGold (US) census of `data/sound/gs_sound_data.sdat`.
const GS: Census = Census {
    path: "data/sound/gs_sound_data.sdat",
    bytes: 8_660_096,
    files: 2_353, // 1,231 SSEQ + 561 SBNK + 561 SWAR
    file_bytes: 8_488_588,
    seq: 2_379,    // entries; 1,372 present (and named)
    bank: 778,     // 561 present
    wave_arc: 778, // 561 present
    player: 9,
    group: 17,
};
/// Retail census of `pbr/sound_data.sdat`.
const PBR: Census = Census {
    path: "pbr/sound_data.sdat",
    bytes: 7_484_768,
    files: 1_846, // 812 SSEQ + 517 SBNK + 517 SWAR
    file_bytes: 7_337_508,
    seq: 2_133,      // 829 present
    bank: 1_519,     // 517 present
    wave_arc: 1_519, // 517 present
    player: 8,
    group: 16,
};

/// The expected shape of one retail archive.
struct Census {
    path: &'static str,
    bytes: usize,
    files: usize,
    file_bytes: u64,
    seq: usize,
    bank: usize,
    wave_arc: usize,
    player: usize,
    group: usize,
}

fn load_rom() -> Option<Vec<u8>> {
    match std::fs::read(ROM_PATH) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

/// Runs the census checks every retail SDAT must satisfy: files counted
/// by magic, and a named entry exactly a present record on every list.
fn census<'a>(data: &'a [u8], expect: &Census) -> Sdat<'a> {
    let path = expect.path;
    let sdat = Sdat::parse(data).unwrap_or_else(|e| panic!("{path} must parse: {e}"));
    assert!(
        sdat.has_symbols(),
        "{path}: retail archives carry a SYMB block"
    );
    assert_eq!(sdat.file_count(), expect.files, "{path}: file count");

    let mut sseq = 0usize;
    let mut sbnk = 0usize;
    let mut swar = 0usize;
    let mut bytes = 0u64;
    for id in 0..sdat.file_count() {
        let file = sdat.file(id).expect("retail FAT entries are never empty");
        bytes += file.len() as u64;
        match &file[0..4] {
            b"SSEQ" => sseq += 1,
            b"SBNK" => sbnk += 1,
            b"SWAR" => swar += 1,
            other => panic!("{path}: file {id} has magic {other:?}"),
        }
    }
    let split = if expect.files == GS.files {
        (1_231, 561, 561)
    } else {
        (812, 517, 517)
    };
    assert_eq!((sseq, sbnk, swar), split, "{path}: file magics");
    assert_eq!(bytes, expect.file_bytes, "{path}: file bytes");

    assert_eq!(sdat.seq_count(), expect.seq, "{path}: seq entries");
    assert_eq!(sdat.bank_count(), expect.bank, "{path}: bank entries");
    assert_eq!(
        sdat.wave_arc_count(),
        expect.wave_arc,
        "{path}: waveArc entries"
    );
    assert_eq!(sdat.player_count(), expect.player, "{path}: player entries");
    assert_eq!(sdat.group_count(), expect.group, "{path}: group entries");
    assert_eq!(sdat.seq_arc_count(), 0, "{path}: no SSAR archives");
    assert_eq!(sdat.strm_count(), 0, "{path}: no STRM streams");

    // A named entry is exactly a present record, on every list.
    for i in 0..expect.seq {
        assert_eq!(
            sdat.seq_name(i).is_some(),
            sdat.seq(i).is_some(),
            "{path}: seq entry {i} named/present mismatch"
        );
    }
    for i in 0..expect.bank {
        assert_eq!(
            sdat.bank_name(i).is_some(),
            sdat.bank(i).is_some(),
            "{path}: bank entry {i} named/present mismatch"
        );
    }
    for i in 0..expect.wave_arc {
        assert_eq!(
            sdat.wave_arc_name(i).is_some(),
            sdat.wave_arc(i).is_some(),
            "{path}: waveArc entry {i} named/present mismatch"
        );
    }
    for i in 0..expect.player {
        assert_eq!(
            sdat.player_name(i).is_some(),
            sdat.player(i).is_some(),
            "{path}: player entry {i} named/present mismatch"
        );
    }
    for i in 0..expect.group {
        assert_eq!(
            sdat.group_name(i).is_some(),
            sdat.group(i).is_some(),
            "{path}: group entry {i} named/present mismatch"
        );
    }
    eprintln!(
        "{path}: {} files ({} bytes), {} seq, {} bank, {} waveArc, {} player, {} group",
        expect.files, bytes, expect.seq, expect.bank, expect.wave_arc, expect.player, expect.group
    );
    sdat
}

#[test]
fn parses_both_sound_archives_and_matches_census() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    for expect in [&GS, &PBR] {
        let archive = rom
            .file_by_path(expect.path)
            .unwrap_or_else(|_| panic!("{} is in the FAT", expect.path));
        assert_eq!(archive.len(), expect.bytes);
        census(archive, expect);
    }
}

#[test]
fn spot_checks_main_archive() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let sdat = Sdat::parse(rom.file_by_path("data/sound/gs_sound_data.sdat").unwrap())
        .expect("the main SDAT must parse");

    // The archive opens with the first cry sequence, played through the
    // first bank and wave arc.
    assert_eq!(sdat.seq_name(1), Some("SEQ_PV001"));
    assert_eq!(
        sdat.seq(1),
        Some(SseqInfo {
            file_id: 0,
            bank: 1,
            volume: 120,
            channel_priority: 127,
            player_priority: 64,
            player: 0,
        })
    );
    assert_eq!(
        sdat.file(0).map(<[u8]>::len),
        Some(0x2C),
        "SSEQ file 0's size"
    );
    // Entry 0 is the unnamed, absent seq slot before the labels start.
    assert_eq!(sdat.seq_name(0), None);
    assert_eq!(sdat.seq(0), None);

    // The title sequences, resolved by name.
    let by_name = |name: &str| {
        (0..sdat.seq_count())
            .find(|&i| sdat.seq_name(i) == Some(name))
            .unwrap_or_else(|| panic!("{name} is in the SYMB block"))
    };
    let title = by_name("SEQ_TEST_TITLE");
    assert_eq!(
        sdat.seq(title),
        Some(SseqInfo {
            file_id: 1,
            bank: 700,
            volume: 50,
            channel_priority: 30,
            player_priority: 64,
            player: 7,
        })
    );
    assert_eq!(sdat.file(1).map(<[u8]>::len), Some(9_428));
    let gs_title = by_name("SEQ_GS_TITLE");
    assert_eq!(
        sdat.seq(gs_title),
        Some(SseqInfo {
            file_id: 3,
            bank: 737,
            volume: 122,
            channel_priority: 30,
            player_priority: 64,
            player: 7, // PLAYER_BGM
        })
    );
    assert_eq!(sdat.file(3).map(<[u8]>::len), Some(17_820));

    // The cry bank and its one wave arc.
    assert_eq!(sdat.bank_name(1), Some("BANK_PV001"));
    assert_eq!(
        sdat.bank(1),
        Some(BankInfo {
            file_id: 1_231,
            swar: [1, 0xFFFF, 0xFFFF, 0xFFFF],
        })
    );
    assert_eq!(sdat.wave_arc_name(1), Some("WAVE_ARC_PV001"));
    assert_eq!(sdat.wave_arc(1), Some(SwarInfo { file_id: 1_792 }));

    // The players: PV gets two simultaneous sequences and the largest
    // heap; FIELD one.
    assert_eq!(sdat.player_name(0), Some("PLAYER_PV"));
    assert_eq!(
        sdat.player(0),
        Some(PlayerInfo {
            seq_count: 2,
            channels: 0xC000,
            heap_size: 24_200,
        })
    );
    assert_eq!(sdat.player_name(1), Some("PLAYER_FIELD"));
    assert_eq!(
        sdat.player(1),
        Some(PlayerInfo {
            seq_count: 1,
            channels: 0xA7FE,
            heap_size: 15_500,
        })
    );

    // The global group loads the nine shared field sequences.
    assert_eq!(sdat.group_name(0), Some("GROUP_GLOBAL"));
    let items = sdat.group(0).expect("GROUP_GLOBAL is present");
    assert_eq!(items.len(), 9);
    assert_eq!(
        items[0],
        GroupItem {
            kind: GroupItemKind::Seq,
            flags: 7,
            index: 1_500,
        }
    );
    assert_eq!(items[1].index, 1_501);
    assert_eq!(items[2].index, 1_506);
    assert!(
        items
            .iter()
            .all(|item| item.kind == GroupItemKind::Seq && item.flags == 7)
    );
}

#[test]
fn spot_checks_pbr_archive() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let sdat = Sdat::parse(rom.file_by_path("pbr/sound_data.sdat").unwrap())
        .expect("the pbr SDAT must parse");

    // The same first cry sequence, byte-identical.
    let pv001 = (0..sdat.seq_count())
        .find(|&i| sdat.seq_name(i) == Some("SEQ_PV001"))
        .expect("SEQ_PV001 is in the SYMB block");
    assert_eq!(
        sdat.seq(pv001),
        Some(SseqInfo {
            file_id: 0,
            bank: 1,
            volume: 120,
            channel_priority: 127,
            player_priority: 64,
            player: 0,
        })
    );
    assert_eq!(
        sdat.bank(1),
        Some(BankInfo {
            file_id: 812,
            swar: [1, 0xFFFF, 0xFFFF, 0xFFFF],
        })
    );
    assert_eq!(sdat.wave_arc(1), Some(SwarInfo { file_id: 1_329 }));
    assert_eq!(
        sdat.player(1).map(|p| p.channels),
        Some(0x07FF),
        "PLAYER_FIELD"
    );
}
