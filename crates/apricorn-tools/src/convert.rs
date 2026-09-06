//! `convert <rom.nds> <out-dir>` — the conversion step: raw formats →
//! engine cache (`docs/conversion.md` has the layout).
//!
//! Everything lands under `<out-dir>/cache/`:
//!
//! * `overlay/arm9/overlay_NNNN.bin` — overlays with the 127 BLZ images
//!   decompressed (plain overlays copied as stored);
//! * `nitrofs/<path>/<id>.<ext>` — cache chunks for the graphics and
//!   text members of every NARC (`<ext>` by chunk kind);
//! * `nitrofs/<path>.<ext>` — chunks for the loose graphics/text files;
//! * `cache.json` — the manifest, keyed on the ROM's SHA-1 with a
//!   SHA-256 per output.
//!
//! Every chunk is written, read back byte-compared, and *parsed through
//! the core cache reader* before the run reports success, so a cache the
//! tool produced is a cache the engine can load. Re-running against an
//! up-to-date cache (same ROM SHA-1, manifest present) skips the
//! rebuild; the manifest is written last, so a half-finished cache is
//! never mistaken for a complete one.

use std::path::Path;
use std::process::ExitCode;

use apricorn_core::cache::{self, ChunkKind};
use apricorn_core::formats::{
    MsgBank, Nanr, Narc, Ncer, Ncgr, Nclr, Nscr, is_nanr, is_narc, is_ncer, is_ncgr, is_nclr,
    is_nscr,
};
use apricorn_core::nds::{NdsRom, blz};
use sha1::{Digest as _, Sha1};

use crate::extract::{json, sha256_hex, write_verified};

/// Converts the ROM at `path`; the cache lands in `out_dir/cache`.
///
/// # Errors
/// Prints `convert: <reason>` and returns failure on any read, parse,
/// decompression, write, or readback-verification error.
pub fn convert(path: &str, out_dir: &str) -> ExitCode {
    match run(path, out_dir) {
        Ok(Some(summary)) => {
            println!("{out_dir}/cache: {}", summary.describe());
            println!(
                "  {} bytes written and verified (every chunk re-parsed)",
                summary.bytes
            );
            println!(
                "  manifest {}",
                Path::new(out_dir).join("cache/cache.json").display()
            );
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("{out_dir}/cache: up to date (ROM SHA-1 unchanged)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("convert: {e}");
            ExitCode::FAILURE
        }
    }
}

/// What one conversion produced, per chunk kind.
struct Summary {
    kinds: [usize; 6],
    overlays: usize,
    decompressed: usize,
    bytes: u64,
}

impl Summary {
    fn describe(&self) -> String {
        let [tiles, palettes, screens, cells, anims, text] = self.kinds;
        let overlays = self.overlays;
        let decompressed = self.decompressed;
        format!(
            "{} chunks ({tiles} tiles, {palettes} palettes, {screens} screens, \
             {cells} cells, {anims} anims, {text} text), {overlays} overlays \
             ({decompressed} BLZ-decompressed)",
            self.kinds.iter().sum::<usize>(),
        )
    }
}

/// `None` when the cache is already up to date.
fn run(path: &str, out_dir: &str) -> Result<Option<Summary>, String> {
    let data = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let rom = NdsRom::parse(&data).map_err(|e| format!("{path}: {e}"))?;
    let rom_sha1 = sha1_hex(&data);
    let root = Path::new(out_dir).join("cache");

    // The manifest is written last and carries the ROM's SHA-1, so its
    // presence with a matching SHA-1 means a previous run completed on
    // this exact ROM.
    if let Ok(manifest) = std::fs::read_to_string(root.join("cache.json"))
        && manifest.contains(&format!("\"sha1\": \"{rom_sha1}\""))
    {
        return Ok(None);
    }

    let mut summary = Summary {
        kinds: [0; 6],
        overlays: 0,
        decompressed: 0,
        bytes: 0,
    };
    let mut overlay_entries = Vec::new();
    let mut chunk_entries = Vec::new();

    // The ARM9 overlays, BLZ-decompressed where compressed.
    for o in rom.overlays() {
        let stored = rom
            .file(o.fat_id)
            .map_err(|e| format!("overlay {}: {e}", o.id))?;
        let rel = format!("overlay/arm9/overlay_{:04}.bin", o.id);
        let (bytes, decompressed) = if o.is_compressed() {
            let raw = blz::decompress(stored, o.raw_size as usize)
                .map_err(|e| format!("overlay {}: {e}", o.id))?;
            (raw, true)
        } else {
            (stored.to_vec(), false)
        };
        write_verified(&root, &rel, &bytes)?;
        summary.overlays += 1;
        summary.decompressed += usize::from(decompressed);
        summary.bytes += bytes.len() as u64;
        overlay_entries.push(format!(
            concat!(
                "    {{ \"id\": {}, \"path\": {}, \"compressed\": {}, ",
                "\"raw_size\": {}, \"sha256\": {} }}"
            ),
            o.id,
            json(&rel),
            o.is_compressed(),
            o.raw_size,
            json(&sha256_hex(&bytes))
        ));
    }

    // Every NitroFS file: NARC members and loose files alike.
    let fs = rom.nitrofs();
    for file in fs.files() {
        let bytes = rom
            .file(file.fat_id)
            .map_err(|e| format!("{}: {e}", file.path))?;
        if is_narc(bytes) {
            let narc = Narc::parse(bytes).map_err(|e| format!("{}: {e}", file.path))?;
            for id in 0..narc.file_count() {
                let member = narc.file(id).expect("id in range");
                let Some((kind, chunk)) = encode_member(&file.path, member)? else {
                    continue;
                };
                let rel = format!("nitrofs/{}/{}.{ext}", file.path, id, ext = kind.extension());
                write_chunk(&root, &rel, &chunk)?;
                summary.kinds[kind.code() as usize] += 1;
                summary.bytes += chunk.len() as u64;
                chunk_entries.push(format!(
                    concat!(
                        "    {{ \"source\": {}, \"kind\": {}, \"path\": {}, ",
                        "\"sha256\": {} }}"
                    ),
                    json(&format!("{}#{id}", file.path)),
                    json(kind_name(kind)),
                    json(&rel),
                    json(&sha256_hex(&chunk))
                ));
            }
        } else if let Some((kind, chunk)) = encode_member(&file.path, bytes)? {
            let rel = format!("nitrofs/{}.{ext}", file.path, ext = kind.extension());
            write_chunk(&root, &rel, &chunk)?;
            summary.kinds[kind.code() as usize] += 1;
            summary.bytes += chunk.len() as u64;
            chunk_entries.push(format!(
                concat!(
                    "    {{ \"source\": {}, \"kind\": {}, \"path\": {}, ",
                    "\"sha256\": {} }}"
                ),
                json(&file.path),
                json(kind_name(kind)),
                json(&rel),
                json(&sha256_hex(&chunk))
            ));
        }
    }

    let manifest = format!(
        concat!(
            "{{\n",
            "  \"cache_version\": 1,\n",
            "  \"chunk_version\": {},\n",
            "  \"rom\": {{ \"sha1\": {}, \"size\": {} }},\n",
            "  \"overlays\": [\n{overlays}  ],\n",
            "  \"chunks\": [\n{chunks}  ]\n",
            "}}\n"
        ),
        cache::VERSION,
        json(&rom_sha1),
        data.len(),
        overlays = overlay_entries.join(",\n"),
        chunks = chunk_entries.join(",\n")
    );
    write_verified(&root, "cache.json", manifest.as_bytes())?;

    Ok(Some(summary))
}

/// Encodes one source member as a cache chunk; `None` when the member is
/// none of the six convertible kinds.
///
/// # Errors
/// A member whose magic matches a known format but fails to parse is a
/// corruption (or a parser bug) and is reported; the MAT sniff is a
/// heuristic, so a sniff hit that fails to parse is just "not a MAT".
fn encode_member(source: &str, member: &[u8]) -> Result<Option<(ChunkKind, Vec<u8>)>, String> {
    if member.is_empty() {
        return Ok(None);
    }
    let err = |e: apricorn_core::nds::NdsError| format!("{source}: {e}");
    if is_ncgr(member) {
        let ncgr = Ncgr::parse(member).map_err(err)?;
        Ok(Some((ChunkKind::Tiles, cache::encode_tiles(&ncgr))))
    } else if is_nclr(member) {
        let nclr = Nclr::parse(member).map_err(err)?;
        Ok(Some((ChunkKind::Palette, cache::encode_palette(&nclr))))
    } else if is_nscr(member) {
        let nscr = Nscr::parse(member).map_err(err)?;
        Ok(Some((ChunkKind::Screen, cache::encode_screen(&nscr))))
    } else if is_ncer(member) {
        let ncer = Ncer::parse(member).map_err(err)?;
        Ok(Some((ChunkKind::Cells, cache::encode_cells(&ncer))))
    } else if is_nanr(member) {
        let nanr = Nanr::parse(member).map_err(err)?;
        let chunk = cache::encode_animation(&nanr).map_err(err)?;
        Ok(Some((ChunkKind::Animation, chunk)))
    } else if mat_sniff(member) {
        match MsgBank::parse(member) {
            Ok(bank) => Ok(Some((ChunkKind::Text, cache::encode_text(&bank)))),
            // Sniff hit, parse miss: an ordinary binary that happens to
            // start with a plausible (count, key) pair.
            Err(_) => Ok(None),
        }
    } else {
        Ok(None)
    }
}

/// The MAT sniff: a u16 message count and non-zero key where the entry
/// table fits the member. The real test is [`MsgBank::parse`], which the
/// caller applies on a hit.
fn mat_sniff(member: &[u8]) -> bool {
    if member.len() < 4 {
        return false;
    }
    let count = u16::from_le_bytes([member[0], member[1]]);
    let key = u16::from_le_bytes([member[2], member[3]]);
    count > 0 && count < 4000 && 4 + 8 * usize::from(count) <= member.len() && key != 0
}

/// Writes a chunk and verifies it through the core cache reader, so a
/// cache this tool produced is a cache the engine can load.
fn write_chunk(root: &Path, rel: &str, chunk: &[u8]) -> Result<(), String> {
    write_verified(root, rel, chunk)?;
    let err = |e: apricorn_core::nds::NdsError| format!("{rel}: {e}");
    match cache::kind_of(chunk).ok_or_else(|| format!("{rel}: not a cache chunk"))? {
        ChunkKind::Tiles => cache::Tiles::parse(chunk).map(|_| ()).map_err(err),
        ChunkKind::Palette => cache::Palette::parse(chunk).map(|_| ()).map_err(err),
        ChunkKind::Screen => cache::Screen::parse(chunk).map(|_| ()).map_err(err),
        ChunkKind::Cells => cache::Cells::parse(chunk).map(|_| ()).map_err(err),
        ChunkKind::Animation => cache::Animation::parse(chunk).map(|_| ()).map_err(err),
        ChunkKind::Text => cache::Text::parse(chunk).map(|_| ()).map_err(err),
    }
}

/// The manifest's name for a chunk kind.
fn kind_name(kind: ChunkKind) -> &'static str {
    match kind {
        ChunkKind::Tiles => "tiles",
        ChunkKind::Palette => "palette",
        ChunkKind::Screen => "screen",
        ChunkKind::Cells => "cells",
        ChunkKind::Animation => "animation",
        ChunkKind::Text => "text",
    }
}

/// The lowercase hex SHA-1 of `bytes`.
fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mat_sniff_requires_sane_header() {
        // A plausible header whose entry table fits.
        let mut member = vec![0u8; 4 + 8 * 2];
        member[0..2].copy_from_slice(&2u16.to_le_bytes());
        member[2..4].copy_from_slice(&0xFEE8u16.to_le_bytes());
        assert!(mat_sniff(&member));
        // Zero key: not a MAT.
        member[2..4].copy_from_slice(&0u16.to_le_bytes());
        assert!(!mat_sniff(&member));
        // Entry table runs past the member.
        let mut big = vec![0u8; 4 + 8 * 2];
        big[0..2].copy_from_slice(&3u16.to_le_bytes());
        big[2..4].copy_from_slice(&7u16.to_le_bytes());
        assert!(!mat_sniff(&big));
        // Zero messages and oversized counts are out too.
        let mut zero = vec![0u8; 64];
        zero[0..2].copy_from_slice(&0u16.to_le_bytes());
        zero[2..4].copy_from_slice(&7u16.to_le_bytes());
        assert!(!mat_sniff(&zero));
        zero[0..2].copy_from_slice(&4000u16.to_le_bytes());
        assert!(!mat_sniff(&zero));
        // Too short to even hold the header.
        assert!(!mat_sniff(&[1, 2, 3]));
    }

    #[test]
    fn kind_names_are_one_to_one() {
        let names = [
            kind_name(ChunkKind::Tiles),
            kind_name(ChunkKind::Palette),
            kind_name(ChunkKind::Screen),
            kind_name(ChunkKind::Cells),
            kind_name(ChunkKind::Animation),
            kind_name(ChunkKind::Text),
        ];
        for (i, name) in names.iter().enumerate() {
            // Distinct, lowercase, non-empty: safe as JSON values.
            assert!(!name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase()));
            assert_eq!(names[i + 1..].iter().filter(|n| *n == name).count(), 0);
        }
    }
}
