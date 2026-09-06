//! ROM and asset pipeline CLI.
//!
//! `verify <rom>` — parse the NDS container, print header/filesystem
//! facts, and check SHA-1 and CRCs.
//! `list <rom>` — print every NitroFS file with its FAT id.
//! `narc <rom> <path>` — list the members of a NitroFS NARC archive.
//! `gfx <rom> <path> [id]` — summarize the NCGR/NCLR/NSCR/NCER/NANR/BTX
//! members of a NARC (one line each), or dump every field of member
//! `id`. A path that is itself a BTX (a loose `.nsbtx`) dumps directly.
//! `msg <rom> <path> [id]` — summarize the MAT message banks of a NARC
//! (one line each), or dump every message of bank `id` as raw code units.

use std::process::ExitCode;

use apricorn_core::formats::{Btx, MsgBank, Nanr, Narc, Ncer, Ncgr, Nclr, Nscr, is_btx};
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
        3 if args[0] == "gfx" => return gfx(&args[1], &args[2], None),
        4 if args[0] == "gfx" => {
            if let Ok(id) = args[3].parse::<usize>() {
                return gfx(&args[1], &args[2], Some(id));
            }
        }
        3 if args[0] == "msg" => return msg(&args[1], &args[2], None),
        4 if args[0] == "msg" => {
            if let Ok(id) = args[3].parse::<usize>() {
                return msg(&args[1], &args[2], Some(id));
            }
        }
        _ => {}
    }
    println!("apricorn-tools — ROM and asset pipeline (PLAN.md Phase 1)");
    println!("usage: apricorn-tools verify <rom.nds>");
    println!("       apricorn-tools list <rom.nds>");
    println!("       apricorn-tools narc <rom.nds> <nitrofs-path>");
    println!("       apricorn-tools gfx <rom.nds> <nitrofs-path> [member-id]");
    println!("       apricorn-tools msg <rom.nds> <nitrofs-path> [bank-id]");
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

/// Summarizes the MAT message banks of a NARC (one line each), or dumps
/// every message of one bank as raw code units (decrypted; the trailing
/// EOS included).
fn msg(path: &str, member_path: &str, detail_id: Option<usize>) -> ExitCode {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("msg: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let rom = match parse(path, &data) {
        Ok(rom) => rom,
        Err(e) => {
            eprintln!("msg: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bytes = match rom.file_by_path(member_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            eprintln!("msg: no NitroFS file named {member_path}");
            return ExitCode::FAILURE;
        }
    };
    let narc = match Narc::parse(bytes) {
        Ok(narc) => narc,
        Err(e) => {
            eprintln!("msg: {member_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(id) = detail_id {
        if id >= narc.file_count() {
            eprintln!("msg: {member_path} has no bank {id}");
            return ExitCode::FAILURE;
        }
        let bank = match MsgBank::parse(narc.file(id).expect("id checked")) {
            Ok(bank) => bank,
            Err(e) => {
                eprintln!("msg: {member_path} bank {id}: {e}");
                return ExitCode::FAILURE;
            }
        };
        println!(
            "{member_path} bank {id}: {} messages, key 0x{:04X}",
            bank.message_count(),
            bank.key()
        );
        for (i, message) in bank.messages().enumerate() {
            let units: Vec<String> = message.iter().map(|u| format!("{u:04X}")).collect();
            println!(
                "msg {i:5}  {:3} units  [{}]",
                message.len(),
                units.join(" ")
            );
        }
        return ExitCode::SUCCESS;
    }

    println!("{}: {} banks", member_path, narc.file_count());
    for id in 0..narc.file_count() {
        let member = narc.file(id).expect("id in range");
        match MsgBank::parse(member) {
            Ok(bank) => println!(
                "{id:4}  {len:8}  MAT  {count:5} msgs  key 0x{key:04X}",
                len = member.len(),
                count = bank.message_count(),
                key = bank.key()
            ),
            Err(e) => println!("{id:4}  {len:8}  (not a MAT: {e})", len = member.len()),
        }
    }
    ExitCode::SUCCESS
}

/// Dumps one BTX archive in full.
fn dump_btx(path: &str, btx: &Btx<'_>) {
    println!("{path}: BTX");
    println!(
        "  textures      {} (data {} bytes){}",
        btx.texture_count(),
        btx.texture_data().len(),
        if btx.uses_pltt4() { " PLTT4" } else { "" }
    );
    println!(
        "  palettes      {} (data {} bytes)",
        btx.palette_count(),
        btx.palette_data().len()
    );
    for tex in btx.textures() {
        println!(
            "  tex {:?}   {}x{} {:?}  off 0x{:X}  data {} of {} bytes{}",
            tex.name(),
            tex.width(),
            tex.height(),
            tex.fmt(),
            tex.offset(),
            tex.data().len(),
            tex.declared_size(),
            if tex.color0_transparent() {
                "  color0-transp"
            } else {
                ""
            }
        );
    }
    for pltt in btx.palettes() {
        println!(
            "  pltt {:?}  off 0x{:X}  word1 {}  data {} bytes",
            pltt.name(),
            pltt.offset(),
            pltt.word1(),
            pltt.data().len()
        );
    }
}

/// Summarizes the graphics members of a NARC, or dumps every field of
/// one member. A path that is itself a BTX (a loose `.nsbtx`) dumps
/// directly. Members that are none of the known formats print as a
/// bare magic.
fn gfx(path: &str, member_path: &str, detail_id: Option<usize>) -> ExitCode {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("gfx: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let rom = match parse(path, &data) {
        Ok(rom) => rom,
        Err(e) => {
            eprintln!("gfx: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bytes = match rom.file_by_path(member_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            eprintln!("gfx: no NitroFS file named {member_path}");
            return ExitCode::FAILURE;
        }
    };
    // A loose NSBTX (e.g. data/dun_sea.nsbtx) is dumped directly; member
    // ids only apply to NARC archives.
    if is_btx(bytes) {
        let btx = match Btx::parse(bytes) {
            Ok(btx) => btx,
            Err(e) => {
                eprintln!("gfx: {member_path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        dump_btx(member_path, &btx);
        return ExitCode::SUCCESS;
    }
    let narc = match Narc::parse(bytes) {
        Ok(narc) => narc,
        Err(e) => {
            eprintln!("gfx: {member_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(id) = detail_id {
        if id >= narc.file_count() {
            eprintln!("gfx: {member_path} has no member {id}");
            return ExitCode::FAILURE;
        }
        let member = narc.file(id).expect("id checked");
        if let Ok(ncgr) = Ncgr::parse(member) {
            println!("{member_path} member {id}: NCGR");
            println!("  version       0x{:04X}", ncgr.version());
            println!(
                "  grid          {}",
                match (ncgr.height(), ncgr.width()) {
                    (Some(h), Some(w)) => format!("{w}x{h} tiles"),
                    _ => "none (linear OBJ data)".to_owned(),
                }
            );
            println!("  pixel format  {}bpp", ncgr.bpp());
            println!("  mapping       {:?}", ncgr.mapping());
            println!(
                "  characterFmt  0x{:08X}{}",
                ncgr.character_fmt(),
                if ncgr.has_vram_transfer() {
                    " (VRAM-transfer)"
                } else {
                    ""
                }
            );
            println!(
                "  tile data     {} bytes ({} tiles)",
                ncgr.tile_data().len(),
                ncgr.tile_count()
            );
            println!(
                "  CPOS         {}",
                match ncgr.cpos() {
                    Some((w, h)) => format!("{w}x{h} tiles"),
                    None => "none".to_owned(),
                }
            );
        } else if let Ok(nclr) = Nclr::parse(member) {
            println!("{member_path} member {id}: NCLR");
            println!("  version       0x{:04X}", nclr.version());
            println!(
                "  fmt           0x{:08X} ({}bpp)",
                nclr.fmt_raw(),
                nclr.bpp()
            );
            println!("  extended      {}", nclr.is_extended());
            println!(
                "  palette       {} stored colors ({} bytes), logical {} bytes",
                nclr.color_count(),
                nclr.palette_data().len(),
                nclr.logical_size()
            );
            if nclr.is_compressed() {
                println!("  compressed    yes (logical size exceeds stored bytes)");
            }
            if let Some(pmcp) = nclr.pmcp() {
                let indices: Vec<String> = (0..usize::from(pmcp.num_palettes()))
                    .map(|slot| pmcp.palette_of(slot).expect("slot in range").to_string())
                    .collect();
                println!(
                    "  PMCP          {} slots -> [{}]",
                    pmcp.num_palettes(),
                    indices.join(",")
                );
            }
        } else if let Ok(nscr) = Nscr::parse(member) {
            println!("{member_path} member {id}: NSCR");
            println!("  version       0x{:04X}", nscr.version());
            println!(
                "  screen        {}x{} px ({}x{} tiles)",
                nscr.width(),
                nscr.height(),
                nscr.width_tiles(),
                nscr.height_tiles()
            );
            println!("  colorMode     {}", nscr.color_mode());
            println!("  screenFormat  {}", nscr.screen_format());
            println!("  entries       {} bytes", nscr.entries().len());
        } else if let Ok(ncer) = Ncer::parse(member) {
            println!("{member_path} member {id}: NCER");
            println!("  version       0x{:04X}", ncer.version());
            println!("  mapping       {:?}", ncer.mapping());
            println!(
                "  cells         {} (extended: {})",
                ncer.cell_count(),
                if ncer.is_extended() { "yes" } else { "no" }
            );
            println!(
                "  OAM           {} entries",
                ncer.cells().iter().map(|c| c.oam_count).sum::<usize>()
            );
            println!(
                "  labels        {} ({})",
                ncer.labels().len(),
                ncer.labels().join(", ")
            );
            if let Some(vram) = ncer.vram_transfer() {
                println!(
                    "  VRAM transfer max {} bytes, {} blocks",
                    vram.sz_byte_max,
                    vram.blocks.len()
                );
            }
            if let Some(ucat) = ncer.ucat() {
                let attrs: Vec<String> = ucat.attrs().iter().map(|a| format!("0x{a:X}")).collect();
                println!("  UCAT          [{}]", attrs.join(", "));
            }
            for (i, cell) in ncer.cells().iter().enumerate() {
                let mut flags = String::new();
                if cell.h_flip() {
                    flags.push_str(" hflip");
                }
                if cell.v_flip() {
                    flags.push_str(" vflip");
                }
                if cell.hv_flip() {
                    flags.push_str(" hvflip");
                }
                if cell.has_bounding_rect() {
                    flags.push_str(" rect");
                }
                println!(
                    "  cell {i:3}     {} OAM, radius {}{flags}",
                    cell.oam_count,
                    cell.radius()
                );
                if let Some(b) = cell.bounding_box() {
                    println!(
                        "               bounds x {}..{} y {}..{}",
                        b.min_x, b.max_x, b.min_y, b.max_y
                    );
                }
                for j in 0..cell.oam_count {
                    let (a0, a1, a2) = cell.oam_attr(j).expect("j in range");
                    println!(
                        "               oam {j}: attr0 0x{a0:04X} attr1 0x{a1:04X} attr2 0x{a2:04X}"
                    );
                }
            }
        } else if let Ok(nanr) = Nanr::parse(member) {
            println!("{member_path} member {id}: NANR");
            println!("  version       0x{:04X}", nanr.version());
            println!(
                "  sequences     {} ({} frames total){}",
                nanr.sequence_count(),
                nanr.total_frames(),
                if nanr.uaat().is_some() { " UAAT" } else { "" }
            );
            for (i, seq) in nanr.sequences().iter().enumerate() {
                println!(
                    "  seq {i:3} \"{}\"  {} frames, loop {}, {:?}, {:?}",
                    seq.label(),
                    seq.frame_count(),
                    seq.loop_start(),
                    seq.element(),
                    seq.play_mode()
                );
                for j in 0..seq.frame_count() {
                    println!(
                        "    frame {j:3} delay {:3}  {:?}",
                        seq.frames()[j].delay,
                        seq.result(j).expect("validated at parse")
                    );
                }
            }
        } else if let Ok(btx) = Btx::parse(member) {
            dump_btx(&format!("{member_path} member {id}"), &btx);
        } else {
            eprintln!("gfx: {member_path} member {id} is not a NCGR/NCLR/NSCR");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    println!("{}: {} members", member_path, narc.file_count());
    for id in 0..narc.file_count() {
        let member = narc.file(id).expect("id in range");
        let len = member.len();
        if let Ok(ncgr) = Ncgr::parse(member) {
            let dims = match (ncgr.height(), ncgr.width()) {
                (Some(h), Some(w)) => format!("{w}x{h} tiles"),
                _ => "linear".to_owned(),
            };
            println!(
                "{id:4}  {len:8}  NCGR  {dims:14}  {}bpp  {:?}  {tiles} tiles",
                ncgr.bpp(),
                ncgr.mapping(),
                tiles = ncgr.tile_count()
            );
        } else if let Ok(nclr) = Nclr::parse(member) {
            println!(
                "{id:4}  {len:8}  NCLR  {}bpp  {colors:4} colors  logical {logical}B{compressed}{pmcp}",
                nclr.bpp(),
                colors = nclr.color_count(),
                logical = nclr.logical_size(),
                compressed = if nclr.is_compressed() {
                    " compressed"
                } else {
                    ""
                },
                pmcp = if nclr.pmcp().is_some() { " PMCP" } else { "" }
            );
        } else if let Ok(nscr) = Nscr::parse(member) {
            println!(
                "{id:4}  {len:8}  NSCR  {}x{}px  colorMode {}  screenFormat {}",
                nscr.width(),
                nscr.height(),
                nscr.color_mode(),
                nscr.screen_format()
            );
        } else if let Ok(ncer) = Ncer::parse(member) {
            println!(
                "{id:4}  {len:8}  NCER  {} cells  {} OAM  {:?}{}{}  {} labels",
                ncer.cell_count(),
                ncer.cells().iter().map(|c| c.oam_count).sum::<usize>(),
                ncer.mapping(),
                if ncer.is_extended() { "  ext" } else { "" },
                if ncer.ucat().is_some() { "  UCAT" } else { "" },
                ncer.labels().len()
            );
        } else if let Ok(nanr) = Nanr::parse(member) {
            println!(
                "{id:4}  {len:8}  NANR  {} seqs  {} frames{}",
                nanr.sequence_count(),
                nanr.total_frames(),
                if nanr.uaat().is_some() { "  UAAT" } else { "" }
            );
        } else if let Ok(btx) = Btx::parse(member) {
            println!(
                "{id:4}  {len:8}  BTX   {} tex  {} pal{}",
                btx.texture_count(),
                btx.palette_count(),
                if btx.uses_pltt4() { "  PLTT4" } else { "" }
            );
        } else {
            println!("{id:4}  {len:8}  (not a BG graphics member)");
        }
    }
    ExitCode::SUCCESS
}
