//! `extract <rom.nds> <out-dir>` — unpack a ROM into a verified asset
//! tree plus a JSON manifest (`docs/extraction.md` has the layout).
//!
//! Every file is written, read back, and byte-compared with its source
//! slice before the tool reports success; the manifest's SHA-256s are
//! computed from the source bytes.

use std::path::Path;
use std::process::ExitCode;

use apricorn_core::nds::NdsRom;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;

/// Extracts the ROM at `path` into `out_dir`.
///
/// # Errors
/// Prints `extract: <reason>` and returns failure on any read, parse,
/// write, or readback-verification error.
pub fn extract(path: &str, out_dir: &str) -> ExitCode {
    match run(path, out_dir) {
        Ok(summary) => {
            println!(
                "{}: {} NitroFS files ({} directories), {} overlays, 3 binaries",
                out_dir, summary.files, summary.dirs, summary.overlays
            );
            println!(
                "  {} bytes ({:.1} MiB) written and verified",
                summary.bytes,
                summary.bytes as f64 / (1024.0 * 1024.0)
            );
            println!(
                "  manifest {}",
                Path::new(out_dir).join("manifest.json").display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("extract: {e}");
            ExitCode::FAILURE
        }
    }
}

/// What one extraction produced.
struct Summary {
    files: usize,
    dirs: usize,
    overlays: usize,
    bytes: u64,
}

fn run(path: &str, out_dir: &str) -> Result<Summary, String> {
    let data = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let rom = NdsRom::parse(&data).map_err(|e| format!("{path}: {e}"))?;
    let root = Path::new(out_dir);
    std::fs::create_dir_all(root).map_err(|e| format!("cannot create {out_dir}: {e}"))?;

    let mut bytes = 0u64;

    // The raw cartridge header and both binaries, straight slices of the
    // image.
    let h = &rom.header;
    let mut binaries = Vec::new();
    for (rel, off, size) in [
        ("header.bin", 0, apricorn_core::nds::header::Header::SIZE),
        ("arm9.bin", h.arm9.rom_offset as usize, h.arm9.size as usize),
        ("arm7.bin", h.arm7.rom_offset as usize, h.arm7.size as usize),
    ] {
        let Some(slice) = data.get(off..off + size) else {
            return Err(format!(
                "{path}: {rel} region 0x{off:X}..0x{:X} runs past the image",
                off + size
            ));
        };
        write_verified(root, rel, slice)?;
        bytes += size as u64;
        binaries.push(format!(
            "    {{ \"path\": {}, \"offset\": {off}, \"size\": {size}, \"sha256\": {} }}",
            json(rel),
            json(&sha256_hex(slice))
        ));
    }

    // The ARM9 overlays, exactly as stored (the manifest notes which are
    // LZ-compressed; decompression belongs to the conversion step).
    let mut overlays = Vec::new();
    for o in rom.overlays() {
        let slice = rom
            .file(o.fat_id)
            .map_err(|e| format!("overlay {}: {e}", o.id))?;
        let rel = format!("overlay/arm9/overlay_{:04}.bin", o.id);
        write_verified(root, &rel, slice)?;
        bytes += slice.len() as u64;
        let offset = rom.nitrofs().fat()[o.fat_id as usize].0;
        overlays.push(format!(
            concat!(
                "    {{ \"id\": {}, \"path\": {}, \"ram_address\": {}, ",
                "\"bss_size\": {}, \"sinit_start\": {}, \"sinit_end\": {}, ",
                "\"fat_id\": {}, \"compressed\": {}, \"offset\": {}, ",
                "\"size\": {}, \"raw_size\": {}, \"sha256\": {} }}"
            ),
            o.id,
            json(&rel),
            o.ram_address,
            o.bss_size,
            o.sinit_start,
            o.sinit_end,
            o.fat_id,
            o.is_compressed(),
            offset,
            slice.len(),
            o.raw_size,
            json(&sha256_hex(slice))
        ));
    }

    // The NitroFS tree, at its original paths under `nitrofs/`.
    let fs = rom.nitrofs();
    let mut files = Vec::new();
    for file in fs.files() {
        let slice = rom
            .file(file.fat_id)
            .map_err(|e| format!("{}: {e}", file.path))?;
        let rel = format!("nitrofs/{}", file.path);
        write_verified(root, &rel, slice)?;
        bytes += slice.len() as u64;
        let offset = fs.fat()[file.fat_id as usize].0;
        files.push(format!(
            concat!(
                "    {{ \"path\": {}, \"fat_id\": {}, \"offset\": {}, ",
                "\"size\": {}, \"sha256\": {} }}"
            ),
            json(&rel),
            file.fat_id,
            offset,
            slice.len(),
            json(&sha256_hex(slice))
        ));
    }

    let sha1: String = Sha1::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let dirs: Vec<String> = fs.dirs().iter().map(|d| json(&d.path)).collect();
    let manifest = format!(
        concat!(
            "{{\n",
            "  \"manifest_version\": 1,\n",
            "  \"rom\": {{\n",
            "    \"title\": {},\n",
            "    \"game_code\": {},\n",
            "    \"maker_code\": {},\n",
            "    \"unit_code\": {},\n",
            "    \"size\": {},\n",
            "    \"sha1\": {},\n",
            "    \"header_crc_ok\": {},\n",
            "    \"logo_crc_ok\": {}\n",
            "  }},\n",
            "  \"binaries\": [\n{binaries}  ],\n",
            "  \"directories\": [\n    {}\n  ],\n",
            "  \"files\": [\n{files}  ],\n",
            "  \"overlays\": [\n{overlays}  ]\n",
            "}}\n"
        ),
        json(&h.title),
        json(&h.game_code_str()),
        json(&String::from_utf8_lossy(&h.maker_code)),
        h.unit_code,
        data.len(),
        json(&sha1),
        rom.header_crc_ok(),
        rom.logo_crc_ok(),
        dirs.join(",\n    "),
        binaries = binaries.join(",\n"),
        files = files.join(",\n"),
        overlays = overlays.join(",\n")
    );
    write_verified(root, "manifest.json", manifest.as_bytes())?;

    Ok(Summary {
        files: fs.files().len(),
        dirs: fs.dirs().len(),
        overlays: rom.overlays().len(),
        bytes,
    })
}

/// Writes `bytes` to `root/rel` (creating parent directories), reads the
/// file back, and fails unless the readback is byte-identical.
fn write_verified(root: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", rel))?;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("cannot write {rel}: {e}"))?;
    let readback = std::fs::read(&path).map_err(|e| format!("cannot read back {rel}: {e}"))?;
    if readback != bytes {
        return Err(format!("{rel}: readback differs from the source bytes"));
    }
    Ok(())
}

/// The lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A JSON string literal, with `"` `\` and control characters escaped.
fn json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escapes_quotes_backslashes_and_controls() {
        assert_eq!(json(""), "\"\"");
        assert_eq!(json("data/a.bin"), "\"data/a.bin\"");
        assert_eq!(json("a\"b"), "\"a\\\"b\"");
        assert_eq!(json("a\\b"), "\"a\\\\b\"");
        assert_eq!(json("a\u{1}b"), "\"a\\u0001b\"");
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
