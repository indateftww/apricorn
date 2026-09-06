//! Integration tests: every message bank (MAT) in a retail HeartGold (US)
//! dump.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM). The expected counts are ground truth established by
//! scanning the retail image while writing the parser — see
//! `docs/nitro-msg.md`.

use std::collections::HashSet;

use apricorn_core::formats::{EOS, MsgBank, Narc};
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail HeartGold (US) message-bank census, over `a/0/2/7`.
const BANKS: usize = 829;
const MESSAGES: usize = 49_984;
const UNITS: usize = 2_106_048;
const BANKS_MAX_COUNT: usize = 1_717; // bank 728
const BANKS_MIN_COUNT: usize = 1;
const UNIQUE_KEYS: usize = 786;
const LF_UNITS: usize = 32_376; // 0xE000 linefeeds; nothing else in 0xE001..0xEFFF
const EXT_CTRL: usize = 13_926; // 0xFFFE control codes (sizes 0/1/2 only)
const TRNAME_UNITS: usize = 738; // 0xF100 packed trainer-name starts

fn load_rom() -> Option<Vec<u8>> {
    match std::fs::read(ROM_PATH) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

#[test]
fn parses_every_message_bank_and_matches_census() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    let narc = Narc::parse(rom.file_by_path("a/0/2/7").expect("msg.narc is in the FAT"))
        .expect("a/0/2/7 must be a NARC");
    assert_eq!(narc.file_count(), BANKS);

    let mut messages = 0usize;
    let mut units = 0usize;
    let mut min_count = usize::MAX;
    let mut max_count = 0usize;
    let mut keys = HashSet::new();
    let mut lf = 0usize;
    let mut ext = 0usize;
    let mut trname = 0usize;

    for id in 0..narc.file_count() {
        let member = narc.file(id).expect("member id in range");
        let bank = MsgBank::parse(member)
            .unwrap_or_else(|e| panic!("a/0/2/7 member {id} failed to parse: {e}"));

        messages += bank.message_count();
        units += bank.messages().map(<[u16]>::len).sum::<usize>();
        min_count = min_count.min(bank.message_count());
        max_count = max_count.max(bank.message_count());
        keys.insert(bank.key());
        assert_ne!(bank.key(), 0, "bank {id}: retail keys are never zero");

        for message in bank.messages() {
            assert_eq!(*message.last().expect("messages have length"), EOS);
            // Walk the control-code structure: a 0xFFFE ext code consumes
            // code, size, and `size` field units; everything else is one
            // unit. Retail ext blocks never run past the EOS.
            let mut j = 0usize;
            while j < message.len() - 1 {
                let v = message[j];
                if v == 0xFFFE {
                    let size = usize::from(message[j + 2]);
                    assert!(
                        j + 3 + size <= message.len() - 1,
                        "bank {id}: ext control code overruns its message"
                    );
                    assert!(size <= 2, "bank {id}: unexpected ext size {size}");
                    ext += 1;
                    j += 3 + size;
                } else {
                    if v == 0xE000 {
                        lf += 1;
                    } else if v > 0xE000 && v < 0xF000 {
                        panic!("bank {id}: unexamined 0xE0xx unit {v:#06X}");
                    } else if v == 0xF100 {
                        trname += 1;
                    } else if v > 0xF100 && v < 0xFF00 {
                        panic!("bank {id}: unexamined 0xF1xx unit {v:#06X}");
                    }
                    j += 1;
                }
            }
        }
    }

    assert_eq!(messages, MESSAGES, "message count");
    assert_eq!(units, UNITS, "code-unit count");
    assert_eq!(min_count, BANKS_MIN_COUNT);
    assert_eq!(max_count, BANKS_MAX_COUNT);
    assert_eq!(keys.len(), UNIQUE_KEYS, "unique keys");
    assert_eq!(lf, LF_UNITS, "LF units");
    assert_eq!(ext, EXT_CTRL, "ext control codes");
    assert_eq!(trname, TRNAME_UNITS, "TRNAME units");
    eprintln!("parsed {BANKS} banks: {messages} messages, {units} units");
}

#[test]
fn spot_checks_known_banks() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");
    let fat_id = rom
        .nitrofs()
        .fat_id_by_path("a/0/2/7")
        .expect("msg.narc in the NitroFS");
    let narc =
        Narc::parse(rom.file(fat_id).expect("fat id in range")).expect("a/0/2/7 must be a NARC");
    let bank = |id: usize| {
        MsgBank::parse(narc.file(id).expect("member id in range"))
            .unwrap_or_else(|e| panic!("bank {id} must parse: {e}"))
    };

    // Bank 0: the mk file pins its key (`msg_0000.bin: -k 0xFEE8`).
    let b = bank(0);
    assert_eq!(b.message_count(), 12);
    assert_eq!(b.key(), 0xFEE8);
    assert_eq!(b.message(0), Some(&[0x12f, 0x142, 0x133, 0x13e, EOS][..]));

    // Bank 237: species names. Message 0 is a placeholder row of dashes;
    // the rest spell names out in the generation's charmap (0x12b = 'a',
    // 0x12c = 'b', ...): Bulbasaur, Lugia, Celebi.
    let b = bank(237);
    assert_eq!(b.message_count(), 496);
    assert_eq!(b.key(), 0x782C);
    assert_eq!(
        b.message(0),
        Some(&[0x1be, 0x1be, 0x1be, 0x1be, 0x1be, EOS][..])
    );
    assert_eq!(
        b.message(1), // Bulbasaur
        Some(
            &[
                0x12c, 0x13f, 0x136, 0x12c, 0x12b, 0x13d, 0x12b, 0x13f, 0x13c, EOS
            ][..]
        )
    );
    assert_eq!(
        b.message(249), // Lugia
        Some(&[0x136, 0x13f, 0x131, 0x133, 0x12b, EOS][..])
    );
    assert_eq!(
        b.message(251), // Celebi
        Some(&[0x12d, 0x12f, 0x136, 0x12f, 0x12c, 0x133, EOS][..])
    );

    // Bank 750: move names. Message 0 is a single dash.
    let b = bank(750);
    assert_eq!(b.message_count(), 468);
    assert_eq!(b.key(), 0xF8C9);
    assert_eq!(b.message(0), Some(&[0x1be, EOS][..]));

    // Bank 728: the largest bank (its 1,717 messages pin the census max).
    let b = bank(728);
    assert_eq!(b.message_count(), 1_717);
    assert_eq!(b.key(), 0x17E8);

    // Bank 729: trainer names, generated from trainers.json. The first
    // 728 messages are TRNAME-packed references (0xF100 + packed chunks).
    let b = bank(729);
    assert_eq!(b.message_count(), 738);
    assert_eq!(b.key(), 0xD8ED);
    assert_eq!(b.message(0), Some(&[0xf100, 0x7dde, 0x7ffe, EOS][..]));
    assert_eq!(
        b.message(1),
        Some(&[0xf100, 0x1b3d, 0x2a85, 0x526b, 0x7f56, EOS][..])
    );
}
