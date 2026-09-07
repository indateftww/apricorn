//! `gen-charmap <charmap.txt> <out.rs>` — regenerate the committed
//! generation charmap table `crates/apricorn-core/src/text/charmap.rs`
//! (PLAN.md Phase 4, step 2).
//!
//! Parses pret's `charmap.txt` format — `HEXCODE=<char>` or
//! `HEXCODE={<command>}` lines, `//` comments, escapes `\xHHHH`, `\n`,
//! `\r`, `\f` — and emits a deterministic, documented Rust module. The
//! output is *committed*: the engine cannot depend on a local pret clone
//! at runtime, and the table is our code expressing the observed
//! code→character mapping. `tests/charmap_hg.rs` cross-checks the
//! committed table against the source `charmap.txt` whenever the clone
//! is present, so a regenerated file cannot silently drift.
//!
//! One-time tool: rerun only when the mapping itself changes, e.g.
//! `cargo run -p apricorn-tools -- gen-charmap
//! refs/pokeheartgold/charmap.txt
//! crates/apricorn-core/src/text/charmap.rs`.

use std::process::ExitCode;

/// Generates `out_path` from the pret charmap at `charmap_path`.
///
/// # Errors
/// Prints `gen-charmap: <reason>` and returns failure on any parse or
/// write error.
pub fn generate(charmap_path: &str, out_path: &str) -> ExitCode {
    match run(charmap_path, out_path) {
        Ok(summary) => {
            println!(
                "{out_path}: {} character entries, {} command entries",
                summary.chars, summary.commands
            );
            println!("  source {charmap_path}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gen-charmap: {e}");
            ExitCode::FAILURE
        }
    }
}

/// What one generation produced.
struct Summary {
    chars: usize,
    commands: usize,
}

fn run(charmap_path: &str, out_path: &str) -> Result<Summary, String> {
    let text = std::fs::read_to_string(charmap_path)
        .map_err(|e| format!("cannot read {charmap_path}: {e}"))?;
    let parsed = parse_charmap(&text)?;
    let generated = render(&parsed)?;
    std::fs::write(out_path, generated)
        .map_err(|e| format!("cannot write {out_path}: {e}"))?;
    Ok(Summary {
        chars: parsed.chars.len(),
        commands: parsed.commands.len(),
    })
}

/// The parsed charmap: plain character entries and `{command}` entries.
struct Charmap {
    chars: Vec<(u16, char)>,
    commands: Vec<(u16, String)>,
}

/// Parses the `charmap.txt` format (see the module docs).
///
/// Leading spaces/tabs are ignored; trailing ones are not (some values
/// *are* spaces). Duplicate keys with equal values are tolerated;
/// conflicting values are an error.
fn parse_charmap(text: &str) -> Result<Charmap, String> {
    let mut chars: Vec<(u16, char)> = Vec::new();
    let mut commands: Vec<(u16, String)> = Vec::new();

    for (offset, raw) in text.lines().enumerate() {
        let line = raw.trim_start_matches([' ', '\t']);
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let line_no = offset + 1;
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {line_no}: expected HEXCODE=value"))?;
        let code = u16::from_str_radix(key, 16)
            .map_err(|_| format!("line {line_no}: bad hex code {key:?}"))?;

        if let Some(name) = value
            .strip_prefix('{')
            .and_then(|v| v.strip_suffix('}'))
        {
            if commands
                .iter()
                .any(|&(c, ref n)| c == code && n != name)
            {
                return Err(format!("line {line_no}: conflicting command for {code:04X}"));
            }
            if !commands.iter().any(|&(c, _)| c == code) {
                commands.push((code, name.to_owned()));
            }
            continue;
        }

        let ch = unescape(value)
            .map_err(|e| format!("line {line_no}: value {value:?}: {e}"))?;
        if chars.iter().any(|&(c, v)| c == code && v != ch) {
            return Err(format!("line {line_no}: conflicting mapping for {code:04X}"));
        }
        if !chars.iter().any(|&(c, _)| c == code) {
            chars.push((code, ch));
        }
    }

    chars.sort_unstable_by_key(|&(c, _)| c);
    commands.sort_unstable_by_key(|&(c, _)| c);
    Ok(Charmap { chars, commands })
}

/// Unescapes a character value, which must decode to exactly one char.
fn unescape(value: &str) -> Result<char, String> {
    let mut decoded = String::new();
    let mut rest = value;
    while !rest.is_empty() {
        let c = rest.chars().next().expect("non-empty");
        if c == '\\' {
            let mut tail = rest[1..].chars();
            match tail.next() {
                Some('x') => {
                    let hex: String = tail.by_ref().take(4).collect();
                    if hex.len() != 4 {
                        return Err("\\x escape needs 4 hex digits".to_owned());
                    }
                    let cp = u32::from_str_radix(&hex, 16)
                        .map_err(|_| "bad \\x escape")?;
                    decoded.push(
                        char::from_u32(cp).ok_or("escaped code point is not a char")?,
                    );
                    rest = &rest[2 + hex.len()..];
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
                Some(other) => return Err(format!("unknown escape \\{other}")),
                None => return Err("trailing lone backslash".to_owned()),
            }
        } else {
            decoded.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    let mut it = decoded.chars();
    let (Some(one), None) = (it.next(), it.next()) else {
        return Err("must be exactly one character".to_owned());
    };
    Ok(one)
}

/// Renders the parsed charmap as the `charmap.rs` module (deterministic:
/// no timestamps, sorted tables).
fn render(parsed: &Charmap) -> Result<String, String> {
    if parsed.chars.is_empty() || parsed.commands.is_empty() {
        return Err("charmap has no entries".to_owned());
    }
    let mut out = String::new();
    out.push_str(
        "//! The HeartGold generation character map: code unit to character.\n\
         //!\n\
         //! GENERATED FILE — do not edit. Regenerate with\n\
         //! `cargo run -p apricorn-tools -- gen-charmap <charmap.txt> <this file>`.\n\
         //! Derived from the character-mapping observations published by\n\
         //! pret/pokeheartgold (`charmap.txt`, version 2021.08.17); the\n\
         //! committed table is cross-checked against the source file by\n\
         //! `tests/charmap_hg.rs` whenever the clone is present.\n\
         //!\n\
         //! Character entries cover every unit a MAT message may carry as\n\
         //! plain text (including `0xE000` LF, which maps to `\\n`). Command\n\
         //! entries name the control codes that appear inside `0xFFFE`\n\
         //! extended blocks (and the strvar classes) — as plain units they\n\
         //! never occur in retail data. The dual-purpose codes `0x0100`\n\
         //! and `0x0400` appear in both tables: a strvar block carries the\n\
         //! command; a plain unit is the character.\n\n",
    );
    out.push_str(&format!(
        "/// Number of character entries in [`CHARS`].\n\
         pub const CHAR_ENTRIES: usize = {};\n\
         /// Number of command entries in [`COMMANDS`].\n\
         pub const COMMAND_ENTRIES: usize = {};\n\n",
        parsed.chars.len(),
        parsed.commands.len()
    ));
    out.push_str(
        "/// Every plain `(code, character)` entry, sorted by code for\n\
         /// [`char_of`]'s binary search.\n\
         pub static CHARS: [(u16, char); CHAR_ENTRIES] = [\n",
    );
    for &(code, ch) in &parsed.chars {
        out.push_str(&format!("    (0x{code:04X}, '{}'),\n", ch.escape_debug()));
    }
    out.push_str(
        "];\n\n\
         /// Every control-code `(code, pret command name)` entry, sorted by\n\
         /// code. Braces are pret's text serialization and not stored.\n\
         pub static COMMANDS: [(u16, &str); COMMAND_ENTRIES] = [\n",
    );
    for &(code, ref name) in &parsed.commands {
        out.push_str(&format!("    (0x{code:04X}, \"{name}\"),\n"));
    }
    out.push_str(
        "];\n\n\
         /// Looks up the character a plain code unit maps to.\n\
         #[must_use]\n\
         pub fn char_of(code: u16) -> Option<char> {\n\
         \x20   CHARS\n\
         \x20       .binary_search_by_key(&code, |&(c, _)| c)\n\
         \x20       .ok()\n\
         \x20       .map(|i| CHARS[i].1)\n\
         }\n\n\
         /// Looks up the pret command name of a control code (`0x0100` →\n\
         /// `\"STRVAR_1\"`, `0xFF00` → `\"COLOR\"`, ...).\n\
         #[must_use]\n\
         pub fn command_name(code: u16) -> Option<&'static str> {\n\
         \x20   COMMANDS\n\
         \x20       .iter()\n\
         \x20       .find(|&&(c, _)| c == code)\n\
         \x20       .map(|&(_, name)| name)\n\
         }\n",
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_renders_a_small_charmap() {
        let text = "// comment\n\n\
                    0000=\\x0000\n\
                    012B=a\n\
                    E000=\\n\n\
                    25BC=\\r\n\
                    25BD=\\f\n\
                    0100={STRVAR_1}\n\
                    FF00={COLOR}\n";
        let parsed = parse_charmap(text).expect("fixture parses");
        assert_eq!(
            parsed.chars,
            [
                (0x0000, '\0'),
                (0x012B, 'a'),
                (0x25BC, '\r'),
                (0x25BD, '\u{c}'),
                (0xE000, '\n'),
            ]
        );
        // 25BC/25BD are out of numeric order in the file; sorted here.
        assert_eq!(parsed.chars[2..4], [(0x25BC, '\r'), (0x25BD, '\u{c}')]);
        assert_eq!(
            parsed.commands,
            [(0x0100, "STRVAR_1".to_owned()), (0xFF00, "COLOR".to_owned())]
        );

        let rendered = render(&parsed).expect("fixture renders");
        assert!(rendered.contains("(0x012B, 'a'),"), "character entries");
        assert!(rendered.contains("(0xFF00, \"COLOR\"),"), "command entries");
        assert!(rendered.contains("CHAR_ENTRIES: usize = 5;"));
    }

    #[test]
    fn rejects_bad_lines() {
        // Not HEXCODE=value.
        assert!(parse_charmap("nonsense\n").is_err());
        // Bad hex key.
        assert!(parse_charmap("ZZZZ=a\n").is_err());
        // Multi-character value.
        assert!(parse_charmap("012B=ab\n").is_err());
        // Unknown escape.
        assert!(parse_charmap("012B=\\q\n").is_err());
        // Conflicting duplicate key (equal duplicates are fine).
        assert!(parse_charmap("012B=a\n012B=b\n").is_err());
        assert!(parse_charmap("0100={A}\n0100={B}\n").is_err());
        assert!(parse_charmap("012B=a\n012B=a\n").is_ok());
    }
}