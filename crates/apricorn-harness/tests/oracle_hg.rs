//! Oracle integration tests against the retail HeartGold (US) ROM: the
//! patched melonDS 1.1 build under `out/oracle` boots the real cart and
//! must be deterministic — the whole harness rests on that.
//!
//! These run only when both the ROM (`hg_usa.nds` at the repo root, same
//! policy as `arm_hg.rs`) and the oracle binary (built by
//! `oracle/setup.ps1` into `out/oracle/`, gitignored) are present; CI has
//! neither and skips silently. No committed artifact can make them pass
//! falsely: the oracle self-computes `rom-sha1` and `Trace::parse`
//! validates every line it emits.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use apricorn_harness::diff;
use apricorn_harness::oracle::{OracleRun, ShotRequest};
use apricorn_harness::regions::RegionSet;
use apricorn_harness::trace::{Trace, TraceRecord};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// sLCRNG_State, the game's boot LCG seed slot (`pins/arm9.tsv`): a tiny
/// 4-byte region that turns out to also be where the RTC first shows —
/// HeartGold seeds this LCG from the clock around frame 185.
const RNG_REGION: (&str, u32, u32, u32) = ("sLCRNG_State", 0x021D_15A8, 4, 1);

/// Enough frames for the game to seed its RNG from the pinned clock
/// (observed at frame ~185 in a DirectBoot retail run).
const FRAMES: u32 = 300;

/// The pinned RTC every test here boots with.
const RTC: &str = "2010-03-01T09:00:00";

/// The frame the boot LCG is seeded at (`InitializeMainRNG` from the
/// pinned clock) — the only frame in the first 600 whose
/// `sLCRNG_State` hash differs from the zeroed bss.
const SEED_FRAME: u32 = 185;

/// A frame with the copyright notice on the top LCD (white text on
/// black): the first visible content after DirectBoot.
const CONTENT_FRAME: u32 = 300;

/// A frame inside the title intro (sky on top, sunset sea below) —
/// far past where the first oracle's boot loop rebooted (~220).
const INTRO_FRAME: u32 = 899;

fn oracle_bin() -> Option<PathBuf> {
    apricorn_harness::oracle::find_binary()
}

/// Decodes a PNG the way the oracle writes it: checks the signature,
/// every chunk's CRC-32, the IHDR (8-bit RGB, no interlace), inflates
/// the IDAT zlib stream — stored blocks only, the oracle's encoder
/// writes nothing else — against its Adler-32, and strips the per-row
/// filter bytes (type 0). Returns `(width, height, rgb)`.
fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn be32(b: &[u8]) -> u32 {
        u32::from_be_bytes(b[..4].try_into().unwrap())
    }

    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    let (mut width, mut height) = (0, 0);
    let mut idat = Vec::new();
    let mut ended = false;
    let mut pos = 8;
    while pos < bytes.len() {
        let len = be32(&bytes[pos..]) as usize;
        let ty = &bytes[pos + 4..pos + 8];
        let data = &bytes[pos + 8..pos + 8 + len];
        let crc = be32(&bytes[pos + 8 + len..]);
        assert_eq!(
            crc,
            crc32(&bytes[pos + 4..pos + 8 + len]),
            "bad CRC on chunk {}",
            String::from_utf8_lossy(ty)
        );
        match ty {
            b"IHDR" => {
                width = be32(&data[0..]);
                height = be32(&data[4..]);
                assert_eq!(&data[8..13], &[8, 2, 0, 0, 0], "IHDR: 8-bit RGB, no interlace");
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => ended = true,
            _ => {}
        }
        pos += 12 + len;
    }
    assert!(ended, "no IEND");

    // zlib: CMF/FLG (deflate, check bits), stored blocks, Adler-32.
    assert_eq!(idat[0] & 0x0F, 8, "zlib CM");
    assert_eq!((u32::from(idat[0]) << 8 | u32::from(idat[1])) % 31, 0, "zlib FCHECK");
    let mut raw = Vec::new();
    let mut p = 2;
    loop {
        let header = idat[p];
        assert_eq!(header & 0x06, 0, "not a stored block");
        let len = usize::from(u16::from_le_bytes([idat[p + 1], idat[p + 2]]));
        let nlen = u16::from_le_bytes([idat[p + 3], idat[p + 4]]);
        assert_eq!(nlen, !(len as u16), "NLEN");
        raw.extend_from_slice(&idat[p + 5..p + 5 + len]);
        p += 5 + len;
        if header & 1 != 0 {
            break;
        }
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    assert_eq!(be32(&idat[p..]), (b << 16) | a, "Adler-32");

    let stride = 1 + width as usize * 3;
    assert_eq!(raw.len(), stride * height as usize, "raw image size");
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for row in raw.chunks(stride) {
        assert_eq!(row[0], 0, "filter type None");
        rgb.extend_from_slice(&row[1..]);
    }
    (width, height, rgb)
}

/// How many distinct colours an RGB8 buffer holds.
fn distinct_colours(rgb: &[u8]) -> usize {
    rgb.chunks(3).collect::<HashSet<_>>().len()
}

/// The run's prerequisites, or a skip.
fn setup() -> Option<(PathBuf, PathBuf)> {
    if !PathBuf::from(ROM_PATH).is_file() {
        eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
        return None;
    }
    let Some(oracle) = oracle_bin() else {
        eprintln!("skipping: out/oracle/apricorn-oracle not built (run oracle/setup.ps1)");
        return None;
    };
    let work = std::env::temp_dir().join("apricorn-oracle-test");
    std::fs::create_dir_all(&work).unwrap();
    Some((oracle, work))
}

/// Compiles the regions blob the oracle consumes (little-endian:
/// u32 count, then addr/size/sample/name_len + name per region —
/// docs/oracle.md is the format contract).
fn write_regions_blob(path: &std::path::Path) {
    let (name, addr, size, sample) = RNG_REGION;
    let mut blob = Vec::new();
    blob.extend_from_slice(&1u32.to_le_bytes());
    blob.extend_from_slice(&addr.to_le_bytes());
    blob.extend_from_slice(&size.to_le_bytes());
    blob.extend_from_slice(&sample.to_le_bytes());
    blob.extend_from_slice(&(name.len() as u32).to_le_bytes());
    blob.extend_from_slice(name.as_bytes());
    std::fs::write(path, blob).unwrap();
}

/// One oracle run: the pinned-RTC boot with no input, [`FRAMES`] frames.
/// Returns the trace text, failing the test with the oracle's stderr if
/// it doesn't exit 0.
fn run(oracle: &Path, work: &Path, rtc: &str, tag: &str) -> String {
    run_with(oracle, work, rtc, tag, FRAMES, &[])
}

/// [`run`] with a frame count and extra command-line flags.
fn run_with(oracle: &Path, work: &Path, rtc: &str, tag: &str, frames: u32, extra: &[&str]) -> String {
    let regions = work.join(format!("{tag}.regions.bin"));
    let out = work.join(format!("{tag}.trace"));
    write_regions_blob(&regions);

    // Passthrough header values: both traces share them, and the diff
    // gate requires the pair to agree. Well-formed hex, content-free.
    let input_sha = "00".repeat(20);
    let regions_sha = "ff".repeat(20);

    let status = Command::new(oracle)
        .arg("run")
        .arg("--rom")
        .arg(ROM_PATH)
        .arg("--out")
        .arg(&out)
        .arg("--regions")
        .arg(&regions)
        .arg("--frames")
        .arg(frames.to_string())
        .arg("--rtc")
        .arg(rtc)
        .arg("--input-sha1")
        .arg(&input_sha)
        .arg("--regions-sha1")
        .arg(&regions_sha)
        .args(extra)
        .output()
        .expect("failed to spawn the oracle");
    assert!(
        status.status.success(),
        "oracle exited {:?}\nstderr:\n{}",
        status.status.code(),
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(&out)
        .unwrap_or_else(|e| panic!("oracle wrote no trace at {}: {e}", out.display()))
}

#[test]
fn oracle_boot_is_deterministic() {
    let Some((oracle, work)) = setup() else {
        return;
    };

    let a = run(&oracle, &work, "2010-03-01T09:00:00", "det-a");
    let b = run(&oracle, &work, "2010-03-01T09:00:00", "det-b");

    // Two same-configuration runs must be byte-identical, not just
    // equivalent: any divergence here is the oracle, never the game.
    assert_eq!(a, b, "two identical oracle runs produced different traces");

    // And the output must be well-formed for the Rust side end to end:
    // parse both, compare, expect EQUIVALENT.
    let a = Trace::parse(&a).expect("oracle trace failed to parse");
    let b = Trace::parse(&b).expect("oracle trace failed to parse");
    let report = diff::compare(&a, &b, None).expect("diff gate refused a same-config pair");
    assert_eq!(report.verdict, apricorn_harness::Verdict::Equivalent);
}

#[test]
fn rtc_pin_changes_the_boot() {
    let Some((oracle, work)) = setup() else {
        return;
    };

    let a = run(&oracle, &work, "2010-03-01T09:00:00", "rtc-a");
    let b = run(&oracle, &work, "2011-07-04T12:34:56", "rtc-b");

    // The pinned RTC is an input like any other: a different clock must
    // reach the game (HeartGold seeds its boot LCG from it) and change
    // the trace — otherwise the oracle would be ignoring its own pin.
    let ta = Trace::parse(&a).expect("oracle trace failed to parse");
    let tb = Trace::parse(&b).expect("oracle trace failed to parse");
    assert_ne!(ta.header.rtc, tb.header.rtc, "rtc header not passthrough");
    assert_ne!(a, b, "different pinned RTCs produced identical traces");

    // The divergence must be a real state change, not just the header.
    let report = diff::compare(&ta, &tb, None).expect("diff gate refused a same-config pair");
    assert!(
        matches!(report.verdict, apricorn_harness::Verdict::Diverged { .. }),
        "different RTCs but no state divergence: {report:?}"
    );
}

/// `--shots` must write well-formed 256×192 RGB8 PNGs of both LCDs,
/// through the Rust plumbing (`OracleRun::run_with_shots`), and the
/// pictures must be the game's: the copyright notice is on the top
/// LCD at [`CONTENT_FRAME`], so it is not a flat colour.
#[test]
fn shots_write_valid_pngs_of_both_screens() {
    let Some((_, work)) = setup() else {
        return;
    };
    let dir = work.join("shots-valid");
    let _ = std::fs::remove_dir_all(&dir);

    let regions = RegionSet::parse(&format!(
        "# name bucket address size sample\n{} hard {:#010X} {} {}\n",
        RNG_REGION.0, RNG_REGION.1, RNG_REGION.2, RNG_REGION.3
    ))
    .expect("region set");
    let shots = ShotRequest {
        frames: vec![CONTENT_FRAME],
        dir: dir.clone(),
    };
    let trace = OracleRun {
        rom: Path::new(ROM_PATH),
        regions: &regions,
        input: None,
        probes: &[],
        frames: CONTENT_FRAME + 1,
        rtc: Some(RTC),
        producer: None,
    }
    .run_with_shots(&shots)
    .expect("oracle run with shots");
    assert_eq!(trace.header.frames, CONTENT_FRAME + 1);

    let top = decode_png(&std::fs::read(shots.top_path(CONTENT_FRAME)).expect("top PNG"));
    let bottom =
        decode_png(&std::fs::read(shots.bottom_path(CONTENT_FRAME)).expect("bottom PNG"));
    for (name, (w, h, rgb)) in [("top", &top), ("bottom", &bottom)] {
        assert_eq!((*w, *h), (256, 192), "{name} LCD size");
        assert_eq!(rgb.len(), 256 * 192 * 3, "{name} LCD pixel count");
    }
    assert!(
        distinct_colours(&top.2) > 1,
        "the top LCD is a flat colour at frame {CONTENT_FRAME} — no copyright notice"
    );
}

/// The trace is byte-identical with and without `--shots`: screenshots
/// are observation only, never a change to the machine or its output.
#[test]
fn shots_leave_the_trace_byte_identical() {
    let Some((oracle, work)) = setup() else {
        return;
    };
    let dir = work.join("shots-det");
    let _ = std::fs::remove_dir_all(&dir);

    let plain = run_with(&oracle, &work, RTC, "shots-off", 60, &[]);
    let shot = run_with(
        &oracle,
        &work,
        RTC,
        "shots-on",
        60,
        &["--shots", "10,59", "--shots-dir", dir.to_str().unwrap()],
    );
    assert_eq!(plain, shot, "--shots changed the trace");
    for frame in [10, 59] {
        for screen in ["top", "bottom"] {
            let path = dir.join(format!("frame_{frame:06}_{screen}.png"));
            assert!(path.is_file(), "missing {}", path.display());
        }
    }
    let past_end = dir.join("frame_000060_top.png");
    assert!(!past_end.is_file(), "a frame past --frames was written");
}

/// The boot must run straight through: the boot LCG is seeded exactly
/// once ([`SEED_FRAME`]) and the title intro is on screen by
/// [`INTRO_FRAME`]. The first oracle passed the harness's "bit set =
/// held" keymask straight into the active-low KEYINPUT register, so
/// every key was held from power-on and HeartGold's L+R+START+SELECT
/// soft reset rebooted it every ~220 frames — re-seeding at 185, 407,
/// 629… with both LCDs blank forever. This pins the fix.
#[test]
fn boot_runs_through_to_the_title_intro() {
    let Some((oracle, work)) = setup() else {
        return;
    };
    let dir = work.join("shots-intro");
    let _ = std::fs::remove_dir_all(&dir);

    let text = run_with(
        &oracle,
        &work,
        RTC,
        "intro",
        INTRO_FRAME + 1,
        &["--shots", &INTRO_FRAME.to_string(), "--shots-dir", dir.to_str().unwrap()],
    );
    let trace = Trace::parse(&text).expect("oracle trace failed to parse");
    let samples: Vec<(u32, [u8; 20])> = trace
        .records
        .iter()
        .filter_map(|r| match r {
            TraceRecord::Sample { frame, hash, .. } => Some((*frame, *hash)),
            TraceRecord::Call { .. } => None,
        })
        .collect();
    let zeroed = samples[0].1;
    let seeded: Vec<u32> = samples
        .iter()
        .filter(|(_, h)| *h != zeroed)
        .map(|(f, _)| *f)
        .collect();
    assert_eq!(
        seeded,
        vec![SEED_FRAME],
        "sLCRNG_State must leave its zeroed value at frame {SEED_FRAME} only (a reboot re-seeds it)"
    );

    let top = decode_png(&std::fs::read(dir.join(format!("frame_{INTRO_FRAME:06}_top.png"))).unwrap());
    let bottom =
        decode_png(&std::fs::read(dir.join(format!("frame_{INTRO_FRAME:06}_bottom.png"))).unwrap());
    assert!(distinct_colours(&top.2) > 1, "top LCD blank at frame {INTRO_FRAME}");
    assert!(
        distinct_colours(&bottom.2) > 16,
        "bottom LCD has no scene at frame {INTRO_FRAME}"
    );
}
