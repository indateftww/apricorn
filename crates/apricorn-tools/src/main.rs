//! ROM and asset pipeline CLI.
//!
//! `verify <rom>` — parse the NDS container, print header/filesystem
//! facts, and check SHA-1 and CRCs.
//! `list <rom>` — print every NitroFS file with its FAT id.
//! `narc <rom> <path>` — list the members of a NitroFS NARC archive.

use std::process::ExitCode;

use apricorn_core::formats::Narc;
use apricorn_core::nds::NdsRom;
use sha1::{Digest as _, Sha1};

/// SHA-1 of the retail US HeartGold ROM (per pret/pokeheartgold's README).
const EXPECTED_HG_US_SHA1: &str = "4fcded0e2713dc03929845de631d0932ea2b5a37";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.len() {
        2 => match args[0].as_str() {
            "verify" => return verify(&args[1]),
            "list" => return list(&args[1]),
            _ => {}
        },
        3 if args[0] == "narc" => return narc(&args[1], &args[2]),
        _ => {}
    }
    println!("apricorn-tools — ROM and asset pipeline (PLAN.md Phase 1)");
    println!("usage: apricorn-tools verify <rom.nds>");
    println!("       apricorn-tools list <rom.nds>");
    println!("       apricorn-tools narc <rom.nds> <nitrofs-path>");
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
    println!(
        "FNT            offset 0x{:08X} size 0x{:X}",
        h.fnt.offset, h.fnt.size
    );
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
    println!(
        "NitroFS        {} directories, {} files",
        fs.dirs().len(),
        fs.files().len()
    );

    let mut roots: Vec<&str> = fs
        .dirs()
        .iter()
        .filter(|d| d.parent == 0xF000)
        .map(|d| d.path.as_str())
        .collect();
    roots.sort_unstable();
    println!("root dirs      {}", roots.join(", "));

    println!(
        "logo CRC       {}",
        if rom.logo_crc_ok() { "OK" } else { "MISMATCH" }
    );
    println!(
        "header CRC     {}",
        if rom.header_crc_ok() {
            "OK"
        } else {
            "MISMATCH"
        }
    );

    let hash: String = Sha1::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
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

/// Lists a NARC's members: id, size, name (BTNF archives only), and the
/// first four bytes' ASCII, which is usually the contained Nitro
/// format's reversed magic (e.g. `RGCN` = NCGR).
fn narc(path: &str, member_path: &str) -> ExitCode {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("narc: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let rom = match parse(path, &data) {
        Ok(rom) => rom,
        Err(e) => {
            eprintln!("narc: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bytes = match rom.file_by_path(member_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            eprintln!("narc: no NitroFS file named {member_path}");
            return ExitCode::FAILURE;
        }
    };
    let narc = match Narc::parse(bytes) {
        Ok(narc) => narc,
        Err(e) => {
            eprintln!("narc: {member_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("{}: {} members", member_path, narc.file_count());
    for id in 0..narc.file_count() {
        let member = narc.file(id).expect("id in range");
        let magic: String = member
            .iter()
            .take(4)
            .map(|&b| {
                if b.is_ascii_graphic() || b == b' ' {
                    char::from(b)
                } else {
                    '.'
                }
            })
            .collect();
        match narc.name(id) {
            Some(name) => println!("{id:4}  {len:8}  {name:24}  {magic}", len = member.len()),
            None => println!("{id:4}  {len:8}  {magic}", len = member.len()),
        }
    }
    ExitCode::SUCCESS
}
