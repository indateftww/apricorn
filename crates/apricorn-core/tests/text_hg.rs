//! Integration tests: decoding every message in a retail HeartGold (US)
//! dump against the generation charmap (PLAN.md Phase 4, step 2).
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM). The expected counts are ground truth established by
//! scanning the retail image while writing the decoder — see
//! `docs/text.md`. The decode itself is strict: this suite passing
//! means every one of the 49,984 messages decoded with zero violations
//! and reassembled byte-identically.

use std::collections::{BTreeMap, HashSet};

use apricorn_core::formats::{MsgBank, Narc};
use apricorn_core::nds::NdsRom;
use apricorn_core::text::ctrl::is_strvar_code;
use apricorn_core::text::format::MessageFormat;
use apricorn_core::text::{Token, decode};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Retail HeartGold (US) message-bank census over `a/0/2/7`.
const MESSAGES: usize = 49_984;
/// Distinct plain-character units across all messages.
const UNIQUE_CHAR_UNITS: usize = 334;
/// `0xE000` linefeeds.
const LF_UNITS: usize = 32_376;
/// Distinct `0xFFFE` control codes.
const EXT_CTRL_CODES: usize = 60;
/// Total `0xFFFE` blocks (msg_hg.rs pins the same count by raw walk).
const EXT_CTRL_BLOCKS: usize = 13_926;
/// Strvar blocks, their distinct codes, and the buffer index max.
const STRVAR_BLOCKS: usize = 9_210;
const STRVAR_CODES: usize = 50;
const STRVAR_FIELD_MAX: u16 = 18;
/// Distinct strvar codes per class (0x0100/0x0300/0x0400/0x3400).
const STRVAR_CLASS_CODES: [(u16, usize); 4] = [(0x0100, 34), (0x0300, 9), (0x0400, 3), (0x3400, 4)];
/// Every non-strvar control code with its retail block count and size
/// (constant across its occurrences).
const NON_STRVAR_CODES: [(u16, usize, usize); 10] = [
    (0x0200, 738, 1),
    (0x0201, 65, 1),
    (0x0202, 37, 1),
    (0x0203, 34, 1),
    (0x0204, 3, 1),
    (0x0205, 233, 0),
    (0x0206, 6, 0),
    (0xFF00, 3_568, 1),
    (0xFF01, 31, 1),
    (0xFF02, 1, 2),
];
/// Packed trainer-name blocks: 738 total, in exactly two banks.
const TRNAME_BLOCKS: usize = 738;
const TRNAME_PER_BANK: [(usize, usize); 2] = [(246, 10), (729, 728)];
/// Total span units across every TRNAME block (marker through the last
/// block unit, message EOS excluded).
const TRNAME_SPAN_UNITS: usize = 3_355;

fn load_rom() -> Option<Vec<u8>> {
    match std::fs::read(ROM_PATH) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

fn banks(data: &[u8]) -> Vec<MsgBank<'_>> {
    let rom = NdsRom::parse(data).expect("retail ROM must parse");
    let narc = Narc::parse(rom.file_by_path("a/0/2/7").expect("msg.narc is in the FAT"))
        .expect("a/0/2/7 must be a NARC");
    (0..narc.file_count())
        .map(|id| {
            MsgBank::parse(narc.file(id).expect("member id in range"))
                .unwrap_or_else(|e| panic!("a/0/2/7 member {id} failed to parse: {e}"))
        })
        .collect()
}

/// The unpacked name of a message that is one TRNAME block.
fn trname_text(bank: &MsgBank<'_>, mid: usize) -> String {
    let decoded = decode(bank.message(mid).expect("message id in range"))
        .expect("retail TRNAME message decodes");
    let [Token::TrainerName { chars, .. }] = decoded.tokens() else {
        panic!("message {mid} is not one trainer-name block");
    };
    chars
        .iter()
        .map(|&c| apricorn_core::text::charmap::char_of(c).expect("name chars are mapped"))
        .collect()
}

#[test]
fn decodes_every_retail_message_and_matches_census() {
    let Some(data) = load_rom() else { return };
    let banks = banks(&data);

    let mut messages = 0usize;
    let mut unique_chars: HashSet<u16> = HashSet::new();
    let mut lf = 0usize;
    let mut ext_blocks = 0usize;
    // code -> block count; for strvars also the field[0] max.
    let mut ext_counts: BTreeMap<u16, usize> = BTreeMap::new();
    let mut ext_sizes: BTreeMap<u16, usize> = BTreeMap::new();
    let mut strvar_codes: HashSet<u16> = HashSet::new();
    let mut strvar_field_max = 0u16;
    let mut trname_per_bank: BTreeMap<usize, usize> = BTreeMap::new();
    let mut trname_span_units = 0usize;
    let mut trname_char_min = u16::MAX;
    let mut trname_char_max = 0u16;

    for (id, bank) in banks.iter().enumerate() {
        for (mid, message) in bank.messages().enumerate() {
            messages += 1;
            let decoded = decode(message)
                .unwrap_or_else(|e| panic!("bank {id} msg {mid} failed to decode: {e}"));
            assert_eq!(
                decoded.to_units(),
                message,
                "bank {id} msg {mid} must reassemble byte-identically"
            );
            for token in decoded.tokens() {
                match token {
                    Token::Char { unit } => {
                        unique_chars.insert(*unit);
                        if *unit == 0xE000 {
                            lf += 1;
                        }
                    }
                    Token::Ctrl { code, fields } => {
                        ext_blocks += 1;
                        *ext_counts.entry(*code).or_default() += 1;
                        let size = ext_sizes.entry(*code).or_insert(fields.len());
                        assert_eq!(
                            *size, fields.len(),
                            "bank {id} msg {mid}: code 0x{code:04X} has mixed sizes"
                        );
                        if is_strvar_code(*code) {
                            strvar_codes.insert(*code);
                            assert_eq!(
                                fields.len(),
                                2,
                                "bank {id} msg {mid}: strvar 0x{code:04X} is not size 2"
                            );
                            assert_eq!(
                                fields[1], 0,
                                "bank {id} msg {mid}: strvar 0x{code:04X} field[1] is not 0"
                            );
                            strvar_field_max = strvar_field_max.max(fields[0]);
                        }
                    }
                    Token::TrainerName { chars, units } => {
                        *trname_per_bank.entry(id).or_default() += 1;
                        trname_span_units += units.len();
                        for &c in chars {
                            trname_char_min = trname_char_min.min(c);
                            trname_char_max = trname_char_max.max(c);
                        }
                    }
                }
            }
        }
    }

    assert_eq!(messages, MESSAGES);
    assert_eq!(unique_chars.len(), UNIQUE_CHAR_UNITS, "unique char units");
    assert_eq!(lf, LF_UNITS, "LF units");
    assert_eq!(ext_blocks, EXT_CTRL_BLOCKS, "ext control blocks");
    assert_eq!(ext_counts.len(), EXT_CTRL_CODES, "distinct ext codes");

    let strvar_total: usize = ext_counts
        .iter()
        .filter(|&(&code, _)| is_strvar_code(code))
        .map(|(_, &n)| n)
        .sum();
    assert_eq!(strvar_total, STRVAR_BLOCKS, "strvar blocks");
    assert_eq!(strvar_codes.len(), STRVAR_CODES, "distinct strvar codes");
    assert_eq!(strvar_field_max, STRVAR_FIELD_MAX, "strvar field max");
    for (class, codes) in STRVAR_CLASS_CODES {
        assert_eq!(
            strvar_codes.iter().filter(|c| *c & 0xFF00 == class).count(),
            codes,
            "strvar codes in class 0x{class:04X}"
        );
    }

    let mut non_strvar = Vec::new();
    for (&code, &count) in &ext_counts {
        if !is_strvar_code(code) {
            non_strvar.push((code, count, ext_sizes[&code]));
        }
    }
    assert_eq!(non_strvar, NON_STRVAR_CODES, "non-strvar code census");

    let mut per_bank: Vec<(usize, usize)> = trname_per_bank.iter().map(|(&b, &n)| (b, n)).collect();
    per_bank.sort_unstable();
    assert_eq!(per_bank, TRNAME_PER_BANK.to_vec(), "TRNAME blocks per bank");
    let blocks: usize = trname_per_bank.values().sum();
    assert_eq!(blocks, TRNAME_BLOCKS);
    assert_eq!(trname_span_units, TRNAME_SPAN_UNITS, "TRNAME span units");
    assert_eq!(
        (trname_char_min, trname_char_max),
        (0x12b, 0x1de),
        "unpacked name chars stay in the EN glyph range"
    );
}

#[test]
fn spot_checks_pinned_strings() {
    let Some(data) = load_rom() else { return };
    let banks = banks(&data);

    let text = |bank: usize, mid: usize| {
        decode(banks[bank].message(mid).expect("message id in range"))
            .expect("retail message decodes")
            .to_text()
    };

    // Bank 0: menu strings.
    assert_eq!(text(0, 0), "EXIT");
    // Bank 237: species names (uppercase in the source).
    assert_eq!(text(237, 0), "-----");
    assert_eq!(text(237, 1), "BULBASAUR");
    assert_eq!(text(237, 249), "LUGIA");
    assert_eq!(text(237, 250), "HO-OH");
    assert_eq!(text(237, 251), "CELEBI");
    // Bank 750: move names.
    assert_eq!(text(750, 1), "Pound");
    // Bank 3: battle text. Message 3 is Pound's usage line — the
    // retail strvar demo: `<field 0> used\nPound!`.
    assert_eq!(text(3, 3), "{STRVAR#0} used\nPound!");
    assert_eq!(text(3, 4), "The wild {STRVAR#0} used\nPound!");
    assert_eq!(text(3, 5), "The foe’s {STRVAR#0} used\nPound!");
}

#[test]
fn expands_retail_placeholders_into_battle_text() {
    let Some(data) = load_rom() else { return };
    let banks = banks(&data);
    let pound = banks[3].message(3).expect("message 3 exists");

    // A species name bound from bank 237 substitutes verbatim.
    let mut fmt = MessageFormat::new(19); // retail field max is 18
    fmt.set_message(0, &banks[237], 1); // BULBASAUR
    let expanded = fmt.expand_placeholders(pound).expect("field 0 is bound");
    assert_eq!(expanded.to_text(), "BULBASAUR used\nPound!");

    // A packed trainer-name field (bank 729's TRNAME blocks) unpacks
    // in place through String_Cat_HandleTrainerName.
    fmt.set_message(0, &banks[729], 261); // Blue
    let expanded = fmt.expand_placeholders(pound).expect("field 0 is bound");
    assert_eq!(expanded.to_text(), "Blue used\nPound!");

    // An empty field substitutes nothing, exactly as in the game (an
    // out-of-range field index is the game's assert — the unit tests
    // pin that path).
    let fmt = MessageFormat::new(19);
    let expanded = fmt.expand_placeholders(pound).expect("field 0 exists");
    assert_eq!(expanded.to_text(), " used\nPound!");
}

#[test]
fn trainer_name_banks_unpack() {
    let Some(data) = load_rom() else { return };
    let banks = banks(&data);

    // Bank 729: trainer names from trainers.json — 728 packed TRNAME
    // messages out of 738 (the rest are plain text, interspersed), so
    // the names are pinned by raw message id.
    let bank = &banks[729];
    assert_eq!(bank.message_count(), 738);
    let trname_ids: Vec<usize> = (0..bank.message_count())
        .filter(|&mid| bank.message(mid).expect("id in range")[0] == 0xF100)
        .collect();
    assert_eq!(trname_ids.len(), 728);
    assert_eq!(trname_text(bank, 0), " -", "the placeholder row");
    assert_eq!(trname_text(bank, 1), "Silver");
    assert_eq!(trname_text(bank, 261), "Blue");
    assert_eq!(trname_text(bank, 727), "Blue");

    // Bank 246: ten link-battle opponent names in message order.
    let bank = &banks[246];
    assert_eq!(bank.message_count(), 88);
    let names: Vec<String> = (78..88).map(|mid| trname_text(bank, mid)).collect();
    assert_eq!(
        names,
        [
            "Don", "Ed", "Abby", "William", "Benny", "Barry", "Cindy", "Josh", "Samuel", "Kipp",
        ]
    );
}