//! Cross-checks the committed charmap table against pret's source
//! `charmap.txt` whenever the (uncommitted) pret clone is present —
//! so a regenerated `charmap.rs` cannot silently drift from the
//! published mapping (PLAN.md Phase 4, step 2).
//!
//! The parser here is deliberately independent of the
//! `apricorn-tools` generator: two code paths must agree on the same
//! file before the committed table is trusted.

use std::collections::HashMap;

use apricorn_core::text::charmap::{CHARS, CHAR_ENTRIES, COMMANDS, COMMAND_ENTRIES};

const CHARMAP_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../refs/pokeheartgold/charmap.txt"
);

fn load_charmap() -> Option<String> {
    match std::fs::read_to_string(CHARMAP_PATH) {
        Ok(text) => Some(text),
        Err(_) => {
            eprintln!("skipping: {CHARMAP_PATH} not found (pret clone not checked out)");
            None
        }
    }
}

/// Decodes one `charmap.txt` value (escapes `\xHHHH`, `\n`, `\r`, `\f`,
/// `\\`) to exactly one character.
fn unescape(value: &str) -> Option<char> {
    let mut decoded = String::new();
    let mut rest = value;
    while !rest.is_empty() {
        let c = rest.chars().next().expect("non-empty");
        if c != '\\' {
            decoded.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let mut tail = rest[1..].chars();
        match tail.next() {
            Some('x') => {
                let hex: String = tail.take(4).collect();
                let cp = u32::from_str_radix(&hex, 16).ok()?;
                decoded.push(char::from_u32(cp)?);
                rest = &rest[6..];
            }
            Some('n') => {
                decoded.push('\n');
                rest = &rest[2..];
            }
            Some('r') => {
                decoded.push('\r');
                rest = &rest[2..];
            }
            Some('f') => {
                decoded.push('\u{c}');
                rest = &rest[2..];
            }
            Some('\\') => {
                decoded.push('\\');
                rest = &rest[2..];
            }
            _ => return None,
        }
    }
    let mut chars = decoded.chars();
    match (chars.next(), chars.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

#[test]
fn committed_table_matches_pret_charmap() {
    let Some(text) = load_charmap() else { return };

    let mut chars: HashMap<u16, char> = HashMap::new();
    let mut commands: HashMap<u16, String> = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim_start_matches([' ', '\t']);
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let (key, value) = line.split_once('=').expect("HEXCODE=value line");
        let code = u16::from_str_radix(key, 16).expect("hex code");
        if let Some(name) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
            commands.insert(code, name.to_owned());
        } else {
            chars.insert(code, unescape(value).expect("one-character value"));
        }
    }

    assert_eq!(CHAR_ENTRIES, 2_876, "character entry count");
    assert_eq!(COMMAND_ENTRIES, 16, "command entry count");
    assert_eq!(chars.len(), 2_876, "source character entry count");
    assert_eq!(commands.len(), 16, "source command entry count");

    // Every committed entry matches the source, in both directions.
    let committed_chars: HashMap<u16, char> = CHARS.iter().copied().collect();
    assert_eq!(committed_chars, chars, "character tables agree");
    let committed_commands: HashMap<u16, &str> =
        COMMANDS.iter().copied().collect();
    let source_commands: HashMap<u16, &str> = commands
        .iter()
        .map(|(&code, name)| (code, name.as_str()))
        .collect();
    assert_eq!(committed_commands, source_commands, "command tables agree");

    // The dual-purpose codes appear in both tables (a strvar block
    // carries the command; a plain unit is the character).
    assert_eq!(chars[&0x0100], '○');
    assert_eq!(commands[&0x0100], "STRVAR_1");
    assert_eq!(chars[&0x0400], '가');
    assert_eq!(commands[&0x0400], "STRVAR_4");
}