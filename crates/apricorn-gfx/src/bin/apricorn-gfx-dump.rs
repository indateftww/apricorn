//! apricorn-gfx-dump — the headless frame dumper.
//!
//! Boots the Phase 3 boot chain, ticks it inputless, and at each
//! requested frame rasterizes the logical frame and writes the two
//! LCDs as PNGs, honoring the frame's [`DisplaySelect`] (engine B on
//! top for both Phase 3 apps). Prints each frame's top/bottom SHA-1s
//! over the raw RGBA — the same digests the golden test
//! (`tests/raster_hg.rs`) pins, so the CLI, the tests, and the
//! harness all speak pixels by hash.
//!
//! usage: `apricorn-gfx-dump --rom <rom.nds> --frame N[,N..] --out <dir>`
//!
//! Frames are global tick indices (frame 0 is the intro's APPEAR);
//! duplicates are collapsed and output is written in tick order.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use apricorn_core::Frame;
use apricorn_core::app::boot_chain;
use apricorn_core::assets::AssetStore;
use apricorn_core::frame::{DisplaySelect, LogicalFrame};
use apricorn_core::input::Input;
use apricorn_gfx::ScreenBuffer;
use sha1::{Digest, Sha1};

/// The usage summary, printed on a malformed invocation.
const USAGE: &str = "usage: apricorn-gfx-dump --rom <rom.nds> --frame N[,N..] --out <dir>";

/// One parsed invocation.
struct Args {
    rom: PathBuf,
    frames: Vec<u32>,
    out: PathBuf,
}

/// Parses `--rom`, `--frame` (comma-separated indices), `--out`.
fn parse_args() -> Result<Args, &'static str> {
    let mut rom = None;
    let mut frames = Vec::new();
    let mut out = None;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or("missing value");
        match flag.as_str() {
            "--rom" => rom = Some(PathBuf::from(value()?)),
            "--frame" => {
                let list = value()?;
                for n in list.split(',') {
                    frames.push(n.parse().map_err(|_| "bad frame index")?);
                }
            }
            "--out" => out = Some(PathBuf::from(value()?)),
            _ => return Err("unknown flag"),
        }
    }
    Ok(Args {
        rom: rom.ok_or("--rom is required")?,
        frames,
        out: out.ok_or("--out is required")?,
    })
}

fn main() -> ExitCode {
    let mut args = match parse_args() {
        Ok(args) if !args.frames.is_empty() => args,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(64);
        }
    };

    let store = match AssetStore::open(&args.rom) {
        Ok(store) => Arc::new(Mutex::new(store)),
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(1);
        }
    };
    if let Err(err) = std::fs::create_dir_all(&args.out) {
        eprintln!("error: cannot create {}: {err}", args.out.display());
        return ExitCode::from(1);
    }

    let mut chain = boot_chain(Arc::clone(&store));
    // Unique, tick-ordered dump points; the walk stops after the last.
    args.frames.sort_unstable();
    args.frames.dedup();
    let last = args.frames[args.frames.len() - 1];

    for index in 0..=last {
        let frame = chain.tick(Frame { index }, Input::default());
        if args.frames.binary_search(&index).is_ok() {
            dump_frame(index, frame, &store, &args.out);
        }
    }
    ExitCode::SUCCESS
}

/// Rasterizes, writes, and reports one frame's two LCDs.
fn dump_frame(index: u32, frame: &LogicalFrame, store: &Mutex<AssetStore>, out: &Path) {
    let screens = {
        let store = store
            .lock()
            .expect("the asset store is only locked per dump frame");
        apricorn_gfx::render(frame, &*store)
    };
    let (top, bottom) = match frame.display {
        DisplaySelect::MainOnTop => (&screens[0], &screens[1]),
        DisplaySelect::SubOnTop => (&screens[1], &screens[0]),
    };
    write_png(&out.join(format!("frame_{index:06}_top.png")), top);
    write_png(&out.join(format!("frame_{index:06}_bottom.png")), bottom);
    println!(
        "frame {index}: top sha1={} bottom sha1={}",
        sha1_hex(top),
        sha1_hex(bottom)
    );
}

/// One screen as a PNG file (256×192 RGBA8, no conversion needed).
fn write_png(path: &Path, screen: &ScreenBuffer) {
    let file = File::create(path).unwrap_or_else(|err| {
        eprintln!("error: cannot create {}: {err}", path.display());
        std::process::exit(1);
    });
    let mut encoder = png::Encoder::new(
        BufWriter::new(file),
        ScreenBuffer::WIDTH as u32,
        ScreenBuffer::HEIGHT as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap_or_else(|err| {
        eprintln!("error: png header for {}: {err}", path.display());
        std::process::exit(1);
    });
    writer
        .write_image_data(screen.as_rgba().as_flattened())
        .unwrap_or_else(|err| {
            eprintln!("error: png data for {}: {err}", path.display());
            std::process::exit(1);
        });
}

/// The SHA-1 of one screen's raw RGBA bytes — the digest the golden
/// tests pin.
fn sha1_hex(screen: &ScreenBuffer) -> String {
    let digest = Sha1::digest(screen.as_rgba().as_flattened());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
