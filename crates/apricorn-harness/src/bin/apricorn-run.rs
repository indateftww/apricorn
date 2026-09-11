//! `apricorn-run` — the headless engine runner.
//!
//! ```text
//! usage: apricorn-run --rom <rom.nds> --input <script.apin>
//!                     [--save <card.sav>] [--regions <regions.conf>]
//!                     [--trace <out.trace>] [--png <frame,frame,...>]
//!                     [--out <dir>] [--frames <N>]
//! ```
//!
//! Boots the real `apricorn-core` game on the ROM with the script's
//! pinned RTC (and the card backup, if given), ticks it through the
//! script's frames with the script's input, and:
//!
//! * writes the requested frames' two LCDs as PNGs
//!   (`frame_%06d_top.png` / `frame_%06d_bottom.png`, honoring the
//!   frame's display select, as `apricorn-gfx-dump` does) and prints
//!   each screen's SHA-1 over the raw RGBA — the digests the golden
//!   tests pin. `--out` names the directory; without it,
//!   `APRICORN_RENDER_OUT`, else `out/apricorn-run` (gitignored).
//! * writes the engine trace (`--trace`) over the `--regions` config —
//!   the file `apricorn-diff` compares against an oracle trace of the
//!   same case (`docs/engine-runner.md`). Without `--regions` the
//!   trace carries the header gates and no records.
//! * prints the end state: the frames run, the game state, the RNG
//!   state, and the player's identity once Oak has confirmed one.
//!
//! `--frames` overrides the script's `end` (frames past it get idle
//! input). Exit codes: 0 ok, 1 runtime error, 64 usage — the same
//! conventions as the other harness bins.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apricorn_core::app::game::Game;
use apricorn_core::assets::AssetStore;
use apricorn_core::frame::{DisplaySelect, LogicalFrame};
use apricorn_gfx::ScreenBuffer;
use apricorn_harness::engine::{EngineRun, EngineTick};
use apricorn_harness::input::InputScript;
use apricorn_harness::regions::RegionSet;
use sha1::{Digest, Sha1};

/// The usage summary, printed on a malformed invocation.
const USAGE: &str = "usage: apricorn-run --rom <rom.nds> --input <script.apin> [--save <card.sav>] \
                     [--regions <regions.conf>] [--trace <out.trace>] [--png <frame,frame,...>] \
                     [--out <dir>] [--frames <N>]";

/// One parsed invocation.
struct Args {
    rom: PathBuf,
    input: PathBuf,
    save: Option<PathBuf>,
    regions: Option<PathBuf>,
    trace: Option<PathBuf>,
    png: Vec<u32>,
    out: Option<PathBuf>,
    frames: Option<u32>,
}

/// Parses the flags; every flag takes one value.
fn parse_args() -> Result<Args, String> {
    let mut rom = None;
    let mut input = None;
    let mut save = None;
    let mut regions = None;
    let mut trace = None;
    let mut png = Vec::new();
    let mut out = None;
    let mut frames = None;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--rom" => rom = Some(PathBuf::from(value()?)),
            "--input" => input = Some(PathBuf::from(value()?)),
            "--save" => save = Some(PathBuf::from(value()?)),
            "--regions" => regions = Some(PathBuf::from(value()?)),
            "--trace" => trace = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--frames" => {
                frames = Some(
                    value()?
                        .parse()
                        .map_err(|_| "--frames: not a number".to_string())?,
                );
            }
            "--png" => {
                for n in value()?.split(',') {
                    png.push(
                        n.trim()
                            .parse()
                            .map_err(|_| format!("--png: bad frame index '{n}'"))?,
                    );
                }
            }
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    Ok(Args {
        rom: rom.ok_or("--rom is required")?,
        input: input.ok_or("--input is required")?,
        save,
        regions,
        trace,
        png,
        out,
        frames,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{USAGE}\n  {message}");
            return ExitCode::from(64);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("apricorn-run: {message}");
            ExitCode::from(1)
        }
    }
}

fn run(mut args: Args) -> Result<(), String> {
    let read = |path: &Path| {
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
    };
    let script = InputScript::parse(&read(&args.input)?)
        .map_err(|e| format!("{}: {e}", args.input.display()))?;
    let regions = match &args.regions {
        Some(path) => RegionSet::parse(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))?,
        None => RegionSet::parse("").expect("the empty config parses"),
    };
    let save = match &args.save {
        Some(path) => {
            Some(std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?)
        }
        None => None,
    };

    // Unique, tick-ordered dump points.
    args.png.sort_unstable();
    args.png.dedup();
    let out_dir = args.out.clone().unwrap_or_else(|| {
        std::env::var_os("APRICORN_RENDER_OUT")
            .map_or_else(|| PathBuf::from("out/apricorn-run"), PathBuf::from)
    });
    if !args.png.is_empty() {
        std::fs::create_dir_all(&out_dir)
            .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
    }

    let run = EngineRun {
        rom: &args.rom,
        regions: &regions,
        script: &script,
        save: save.as_deref(),
        frames: args.frames,
        producer: None,
    };
    println!(
        "apricorn-run: {} frames, rtc {}, {} region(s), {}",
        run.frames(),
        run.rtc_text(),
        regions.regions().len(),
        if save.is_some() { "card backup loaded" } else { "blank card" }
    );

    // The observer cannot return an error; the first failure is kept
    // and reported after the run.
    let mut failure: Option<String> = None;
    let output = run
        .run_with(|tick: &EngineTick<'_>| {
            if failure.is_some() || args.png.binary_search(&tick.index).is_err() {
                return;
            }
            if let Err(e) = dump_frame(tick.index, tick.frame, tick.store, &out_dir) {
                failure = Some(e);
            }
        })
        .map_err(|e| e.to_string())?;
    if let Some(message) = failure {
        return Err(message);
    }

    if let Some(path) = &args.trace {
        std::fs::write(path, output.trace.to_string())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!(
            "wrote {} ({} records)",
            path.display(),
            output.trace.records.len()
        );
    }
    print_end_state(&output.game, output.trace.header.frames);
    Ok(())
}

/// The end-of-run summary: frames, state, RNGs, identity.
fn print_end_state(game: &Game, frames: u32) {
    println!("frames {frames}");
    println!("state {:?}", game.state());
    println!("lcrng 0x{:08x}", game.lcrng().seed());
    println!("mtrng cycles {}", game.mtrng().cycles());
    match game.player() {
        Some(player) => println!(
            "player name=\"{}\" gender={}",
            player.name.to_text(),
            match player.gender {
                0 => "male".to_string(),
                1 => "female".to_string(),
                other => format!("{other}"),
            }
        ),
        None => println!("player (none: Oak has not confirmed one)"),
    }
    if let Some(data) = game.new_game_data() {
        println!("new-game trainer-id 0x{:08x}", data.trainer_id());
    }
}

/// Rasterizes, writes, and reports one frame's two LCDs — the
/// `apricorn-gfx-dump` idiom, so the SHA-1s printed here are the
/// golden tests' digests.
fn dump_frame(
    index: u32,
    frame: &LogicalFrame,
    store: &std::sync::Mutex<AssetStore>,
    out: &Path,
) -> Result<(), String> {
    let screens = {
        let store = store
            .lock()
            .map_err(|_| "the asset store lock is poisoned".to_string())?;
        apricorn_gfx::render(frame, &*store)
    };
    let (top, bottom) = match frame.display {
        DisplaySelect::MainOnTop => (&screens[0], &screens[1]),
        DisplaySelect::SubOnTop => (&screens[1], &screens[0]),
    };
    write_png(&out.join(format!("frame_{index:06}_top.png")), top)?;
    write_png(&out.join(format!("frame_{index:06}_bottom.png")), bottom)?;
    println!(
        "frame {index}: top sha1={} bottom sha1={}",
        sha1_hex(top),
        sha1_hex(bottom)
    );
    Ok(())
}

/// One screen as a PNG file (256×192 RGBA8, no conversion needed).
fn write_png(path: &Path, screen: &ScreenBuffer) -> Result<(), String> {
    let file =
        File::create(path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(
        BufWriter::new(file),
        ScreenBuffer::WIDTH as u32,
        ScreenBuffer::HEIGHT as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("png header for {}: {e}", path.display()))?;
    writer
        .write_image_data(screen.as_rgba().as_flattened())
        .map_err(|e| format!("png data for {}: {e}", path.display()))
}

/// The SHA-1 of one screen's raw RGBA bytes.
fn sha1_hex(screen: &ScreenBuffer) -> String {
    let digest = Sha1::digest(screen.as_rgba().as_flattened());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
