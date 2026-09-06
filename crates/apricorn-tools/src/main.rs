//! ROM and asset pipeline CLI.
//!
//! `verify <rom>` — parse the NDS container, print header/filesystem
//! facts, and check SHA-1 and CRCs.
//! `list <rom>` — print every NitroFS file with its FAT id.

use std::process::ExitCode;

use apricorn_core::nds::NdsRom;
use sha1::{Digest as _, Sha1};

/// SHA-1 of the retail US HeartGold ROM (per pret/pokeheartgold's README).
const EXPECTED_HG_US_SHA1: &str = "4fcded0e2713dc03929845de631d0932ea2b5a37";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 2 {
        match args[0].as_str() {
            "verify" => return verify(&args[1]),
            "list" => return list(&args[1]),
            _ => {}
        }
    }
    println!("apricorn-tools — ROM and asset pipeline (PLAN.md Phase 1)");
    println!("usage: apricorn-tools verify <rom.nds>");
    println!("       apricorn-tools list <rom.nds>");
    ExitCode::from(64)
}

fn parse<'a>(path: &str, data: &'a [u8]) -> Result<NdsRom<'a>, String> {
    NdsRom::parse(data).map_err(|e| format!("{path}: {e}"))
}

fn verify(path: &str) -> ExitCode {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("verify: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let rom = match parse(path, &data) {
        Ok(rom) => rom,
        Err(e) => {
            eprintln!("verify: {e}");
            return ExitCode::FAILURE;
        }
    };

    let h = &rom.header;
    println!("title          {}", h.title);
    println!(
        "game code      {}  (maker {})",
        h.game_code_str(),
        String::from_utf8_lossy(&h.maker_code)
    );
    println!("unit code      {}", h.unit_code);
    println!(
        "ROM size       {} bytes ({:.1} MiB)",
        data.len(),
        data.len() as f64 / (1024.0 * 1024.0)
    );
    println!(
        "ARM9           rom 0x{:08X} entry 0x{:08X} ram 0x{:08X} size 0x{:X}",
        h.arm9.rom_offset, h.arm9.entry_address, h.arm9.ram_address, h.arm9.size
    );
    println!(
        "ARM7           rom 0x{:08X} entry 0x{:08X} ram 0x{:08X} size 0x{:X}",
        h.arm7.rom_offset, h.arm7.entry_address, h.arm7.ram_address, h.arm7.size
    );
    println!("FNT            offset 0x{:08X} size 0x{:X}", h.fnt.offset, h.fnt.size);
    println!(
        "FAT            offset 0x{:08X} size 0x{:X} ({} entries)",
        h.fat.offset,
        h.fat.size,
        rom.nitrofs().fat().len()
    );
    println!(
        "ARM9 overlays  {} ({} compressed)",
        rom.overlays().len(),
        rom.overlays().iter().filter(|o| o.is_compressed()).count()
    );
    println!("banner offset  0x{:08X}", h.banner_offset);

    let fs = rom.nitrofs();
    println!("NitroFS        {} directories, {} files", fs.dirs().len(), fs.files().len());

    let mut roots: Vec<&str> =
        fs.dirs().iter().filter(|d| d.parent == 0xF000).map(|d| d.path.as_str()).collect();
    roots.sort_unstable();
    println!("root dirs      {}", roots.join(", "));

    println!("logo CRC       {}", if rom.logo_crc_ok() { "OK" } else { "MISMATCH" });
    println!("header CRC     {}", if rom.header_crc_ok() { "OK" } else { "MISMATCH" });

    let hash: String = Sha1::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    println!("SHA-1          {hash}");
    println!("               expected {EXPECTED_HG_US_SHA1} (retail US HeartGold)");
    if hash == EXPECTED_HG_US_SHA1 {
        println!("               MATCH — retail US HeartGold dump");
    } else {
        println!("               (differs — fine for other games/dumps)");
    }

    ExitCode::SUCCESS
}

fn list(path: &str) -> ExitCode {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("list: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match parse(path, &data) {
        Ok(rom) => {
            for file in rom.nitrofs().files() {
                println!("{:4}  {}", file.fat_id, file.path);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("list: {e}");
            ExitCode::FAILURE
        }
    }
}