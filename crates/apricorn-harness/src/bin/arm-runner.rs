//! `arm-runner` — load a retail ARM9 image and call a pinned function.
//!
//! ```text
//! usage: arm-runner call --rom ROM.nds --fn PIN [--arg 0x1234]...
//!        arm-runner scan --rom ROM.nds --constant 0x41C64E6D
//!        arm-runner pins
//! ```
//!
//! `call` verifies every committed pin against the loaded image (a
//! wrong dump or drifted address fails loudly), then runs the named
//! leaf to the sentinel and prints the return registers.
//! `scan` reports where a 4-aligned word constant appears in the
//! decompressed image — the discovery tool behind the pin table.
//! `pins` lists the committed table.
//!
//! Exit code 0 = success, 64 = usage or load error.

use std::process::ExitCode;

use apricorn_core::nds::NdsRom;
use apricorn_harness::arm::retail::RetailArm9;
use apricorn_harness::pins::{self, PinMode, PinTable};

const USAGE: &str = "usage: arm-runner call --rom ROM.nds --fn PIN [--arg 0xVALUE]...\n       arm-runner scan --rom ROM.nds --constant 0xVALUE\n       arm-runner pins";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(64);
    };
    let result = match command.as_str() {
        "call" => cmd_call(args.collect()),
        "scan" => cmd_scan(args.collect()),
        "pins" => {
            for pin in PinTable::arm9().pins() {
                println!("{}\t{:?}", pin.name, pin.mode);
            }
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}\n  unknown command {command:?}");
            return ExitCode::from(64);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("arm-runner: {message}");
            ExitCode::from(64)
        }
    }
}

/// Splits `--flag value` pairs out of `args`.
fn options(args: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut opts = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let key = args[i]
            .strip_prefix("--")
            .ok_or_else(|| format!("expected --flag, got {:?}", args[i]))?;
        i += 1;
        let Some(value) = args.get(i) else {
            return Err(format!("--{key} needs a value"));
        };
        i += 1;
        opts.push((key.to_string(), value.clone()));
    }
    Ok(opts)
}

fn opt<'a>(opts: &'a [(String, String)], key: &str) -> Option<&'a str> {
    opts.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn load_rom(path: &str) -> Result<RetailArm9, String> {
    let data = std::fs::read(path).map_err(|e| format!("--rom {path:?}: {e}"))?;
    RetailArm9::load(&data).map_err(|e| e.to_string())
}

fn cmd_call(args: Vec<String>) -> Result<(), String> {
    let opts = options(&args)?;
    let rom_path = opt(&opts, "rom").ok_or("call needs --rom")?;
    let fn_name = opt(&opts, "fn").ok_or("call needs --fn")?;

    let mut arm9 = load_rom(rom_path)?;
    let table = PinTable::arm9();
    let pin = table
        .get(fn_name)
        .ok_or_else(|| format!("no pin named {fn_name:?}"))?;
    if pin.mode == PinMode::Data {
        return Err(format!("{fn_name:?} is a data pin, not callable"));
    }
    // Thumb pins are entered with bit 0 set, BX-style.
    let entry = pin.address | u32::from(pin.mode == PinMode::Thumb);

    let mut args = [0u32; 4];
    for (i, (_, raw)) in opts.iter().filter(|(k, _)| k == "arg").take(4).enumerate() {
        args[i] = parse_u32(raw).map_err(|e| format!("--arg {raw:?}: {e}"))?;
    }

    let cpu = arm9.cpu();
    cpu.prepare_call(entry, &args);
    let result = cpu
        .run_default()
        .map_err(|e| format!("{fn_name} faulted: {e}"))?;
    println!(
        "{fn_name} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} steps={}",
        result.r0, result.r1, result.r2, result.r3, result.steps
    );
    Ok(())
}

fn cmd_scan(args: Vec<String>) -> Result<(), String> {
    let opts = options(&args)?;
    let rom_path = opt(&opts, "rom").ok_or("scan needs --rom")?;
    let constant = parse_u32(opt(&opts, "constant").ok_or("scan needs --constant")?)
        .map_err(|e| format!("--constant: {e}"))?;

    let data = std::fs::read(rom_path).map_err(|e| format!("--rom {rom_path:?}: {e}"))?;
    let rom = NdsRom::parse(&data).map_err(|e| e.to_string())?;
    let image = rom.arm9_image().map_err(|e| e.to_string())?;
    for addr in pins::scan_constant(&image, rom.header.arm9.ram_address, constant) {
        println!("{addr:#010x}");
    }
    Ok(())
}

fn parse_u32(text: &str) -> Result<u32, String> {
    let text = text.trim();
    u32::from_str_radix(text.trim_start_matches("0x"), 16)
        .map_err(|e| format!("{e} (hex with optional 0x prefix)"))
}
