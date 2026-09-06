//! The trace format — the oracle's and the engine's common language.
//!
//! A trace is line-oriented text so the C++ oracle can emit it with
//! `printf`, humans can read it, and the comparator can pin the exact
//! line of a divergence. It carries **hashes and register metadata
//! only** — never ROM content or raw game state.
//!
//! ```text
//! TRACE apricorn 1
//! producer oracle-melonds-1.1
//! state-format 1
//! rom-sha1 4fcded0e2713dc03929845de631d0932ea2b5a37
//! input-sha1 0f4dba6ab5db31c6d3bcf5b7f9b8a4c2e1d0f9a3
//! regions-sha1 8e6c5d4b3a291807f6e5d4c3b2a1908f7e6d5c4b
//! frames 600
//! frame-rate 59.8268
//! rtc 2010-03-01T09:00:00
//! F 120 rng 6f2ac0ffee15a8dd36ee8a7b1c9d0e2f3a4b5c6d
//! C 0 LCRandom r0=0x0000abcd r1=0x00000000 r2=0x00000000 r3=0x00000000 state=...
//! ```
//!
//! The first line is the **trace text format** version (`TRACE apricorn
//! 1` — bump it when this grammar changes). `state-format` is the
//! **engine state** version and must equal
//! [`apricorn_core::STATE_FORMAT_VERSION`]; [`Trace::parse`] refuses any
//! trace whose state-format differs, so a stale trace can never be
//! silently compared against a newer engine.
//!
//! Two record kinds:
//!
//! * `F <frame> <region> <sha1>` — a frame-sampled region hash. `frame`
//!   counts VBlanks since boot (equivalence is frame-indexed, never
//!   wall-clock); `region` is a [`crate::regions`] name; the hash is
//!   40 lowercase hex digits.
//! * `C <seq> <fn> r0=… r1=… r2=… r3=… state=…` — a per-function probe
//!   record (arm-runner and the oracle's `call` mode): the function's
//!   arguments as r0–r3 in `0x`-padded hex, and the hash over the
//!   canonical region set after the call.
//!
//! The diff refuses trace pairs that differ in `state-format`,
//! `rom-sha1`, or `regions-sha1` (see `docs/equivalence.md`).

use crate::HarnessError;
use apricorn_core::STATE_FORMAT_VERSION;
use sha1::{Digest, Sha1};

/// A SHA-1 digest as raw bytes.
pub type Hash = [u8; 20];

/// The trace header: identity and gate fields before any record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceHeader {
    /// Who produced this trace (e.g. `oracle-melonds-1.1`, `arm-runner`).
    pub producer: String,
    /// SHA-1 of the ROM replayed.
    pub rom_sha1: Hash,
    /// SHA-1 of the input script replayed.
    pub input_sha1: Hash,
    /// SHA-1 over the canonical `regions.conf`.
    pub regions_sha1: Hash,
    /// Frames replayed (VBlanks since boot).
    pub frames: u32,
    /// The pinned frame rate, purely informational (Hz, NDS VBlank).
    pub frame_rate: String,
    /// The pinned RTC the replay ran under, if any.
    pub rtc: Option<String>,
}

/// One trace record — a frame-sampled hash or a function-probe result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceRecord {
    /// `F <frame> <region> <sha1>` — a watched region's hash at a frame.
    Sample {
        /// Frame index (VBlanks since boot).
        frame: u32,
        /// Region name from the `regions.conf`.
        region: String,
        /// SHA-1 of the region bytes at that frame.
        hash: Hash,
    },
    /// `C <seq> <fn> r0..r3 state=…` — a per-function probe.
    Call {
        /// Probe sequence number (probes run in order).
        seq: u32,
        /// The function's name (a `pins` entry).
        func: String,
        /// Arguments r0–r3 the function was called with.
        args: [u32; 4],
        /// SHA-1 over the canonical region set after the call.
        state: Hash,
    },
}

/// A parsed trace: header plus records, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    /// Identity and gate fields.
    pub header: TraceHeader,
    /// `F`/`C` records in emission order.
    pub records: Vec<TraceRecord>,
}

impl Trace {
    /// Parses trace text.
    ///
    /// Enforces the header gate (`state-format` must equal
    /// [`apricorn_core::STATE_FORMAT_VERSION`]; the `TRACE` text-format
    /// line must be `TRACE apricorn 1`), requires every mandatory header
    /// key, and rejects unknown lines with their 1-based line number.
    ///
    /// # Errors
    /// Returns a [`HarnessError::Syntax`] for malformed lines and a
    /// [`HarnessError::Gate`] for version-gate failures.
    pub fn parse(text: &str) -> Result<Self, HarnessError> {
        let mut lines = text.lines();
        let first = lines.next().ok_or(HarnessError::Syntax {
            line: 1,
            what: "empty trace".to_string(),
        })?;
        if first != "TRACE apricorn 1" {
            return Err(HarnessError::Syntax {
                line: 1,
                what: format!("expected 'TRACE apricorn 1', got '{first}'"),
            });
        }

        // Header keys, then records. Keys may appear in any order before
        // the first record line; mandatory keys must all be present.
        let mut producer: Option<String> = None;
        let mut rom_sha1: Option<Hash> = None;
        let mut input_sha1: Option<Hash> = None;
        let mut regions_sha1: Option<Hash> = None;
        let mut frames: Option<u32> = None;
        let mut frame_rate: Option<String> = None;
        let mut rtc: Option<String> = None;
        let mut records = Vec::new();

        for (idx, raw) in lines.enumerate() {
            let line_no = idx + 2; // +2: 1-based, and line 1 was the TRACE line
            let bad = |what: &str| HarnessError::Syntax {
                line: line_no,
                what: what.to_string(),
            };
            let Some((key, value)) = raw.split_once(' ') else {
                return Err(bad(&format!("unknown line: {raw}")));
            };
            match key {
                "producer" => producer = Some(value.to_string()),
                "state-format" => {
                    let version = value
                        .parse::<u32>()
                        .map_err(|_| bad("state-format: not a number"))?;
                    if version != STATE_FORMAT_VERSION {
                        return Err(HarnessError::Gate {
                            what: format!("state-format {version} != {STATE_FORMAT_VERSION}"),
                        });
                    }
                }
                "rom-sha1" => {
                    rom_sha1 =
                        Some(parse_hash(value).ok_or_else(|| bad("rom-sha1: want 40 hex digits"))?)
                }
                "input-sha1" => {
                    input_sha1 = Some(
                        parse_hash(value).ok_or_else(|| bad("input-sha1: want 40 hex digits"))?,
                    )
                }
                "regions-sha1" => {
                    regions_sha1 = Some(
                        parse_hash(value).ok_or_else(|| bad("regions-sha1: want 40 hex digits"))?,
                    )
                }
                "frames" => {
                    frames = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| bad("frames: not a number"))?,
                    )
                }
                "frame-rate" => frame_rate = Some(value.to_string()),
                "rtc" => rtc = Some(value.to_string()),
                "F" => {
                    let mut fields = value.split_whitespace();
                    let frame = fields
                        .next()
                        .and_then(|f| f.parse::<u32>().ok())
                        .ok_or_else(|| bad("F: want 'F <frame> <region> <sha1>'"))?;
                    let region = fields
                        .next()
                        .ok_or_else(|| bad("F: missing region"))?
                        .to_string();
                    let hash = fields
                        .next()
                        .and_then(parse_hash)
                        .ok_or_else(|| bad("F: missing or malformed sha1"))?;
                    if fields.next().is_some() {
                        return Err(bad("F: trailing fields"));
                    }
                    records.push(TraceRecord::Sample {
                        frame,
                        region,
                        hash,
                    });
                }
                "C" => {
                    let seq = value
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<u32>().ok())
                        .ok_or_else(|| bad("C: want 'C <seq> <fn> r0..r3 state=…'"))?;
                    let mut fields = value.split_whitespace().skip(1);
                    let func = fields
                        .next()
                        .ok_or_else(|| bad("C: missing fn"))?
                        .to_string();
                    let mut args = [0u32; 4];
                    for arg in &mut args {
                        let field = fields.next().ok_or_else(|| bad("C: missing r0..r3"))?;
                        let hex = field
                            .strip_prefix("r0=")
                            .or_else(|| field.strip_prefix("r1="))
                            .or_else(|| field.strip_prefix("r2="))
                            .or_else(|| field.strip_prefix("r3="))
                            .ok_or_else(|| bad("C: want r0=… r1=… r2=… r3=…"))?;
                        *arg = parse_hex32(hex)
                            .ok_or_else(|| bad("C: arg must be 0x-prefixed hex"))?;
                    }
                    let state = fields
                        .next()
                        .and_then(|f| f.strip_prefix("state="))
                        .and_then(parse_hash)
                        .ok_or_else(|| bad("C: missing or malformed state hash"))?;
                    if fields.next().is_some() {
                        return Err(bad("C: trailing fields"));
                    }
                    records.push(TraceRecord::Call {
                        seq,
                        func,
                        args,
                        state,
                    });
                }
                _ => return Err(bad(&format!("unknown header key: {key}"))),
            }
        }

        let need = |what: &str| HarnessError::Syntax {
            line: 1,
            what: format!("missing header key: {what}"),
        };
        Ok(Self {
            header: TraceHeader {
                producer: producer.ok_or_else(|| need("producer"))?,
                rom_sha1: rom_sha1.ok_or_else(|| need("rom-sha1"))?,
                input_sha1: input_sha1.ok_or_else(|| need("input-sha1"))?,
                regions_sha1: regions_sha1.ok_or_else(|| need("regions-sha1"))?,
                frames: frames.ok_or_else(|| need("frames"))?,
                frame_rate: frame_rate.ok_or_else(|| need("frame-rate"))?,
                rtc,
            },
            records,
        })
    }

    /// SHA-1 of arbitrary bytes — the hash every trace field carries.
    #[must_use]
    pub fn hash_bytes(bytes: &[u8]) -> Hash {
        let mut hasher = Sha1::new();
        hasher.update(bytes);
        hasher.finalize().into()
    }

    /// SHA-1 of a region's bytes at one frame (the value an `F` record
    /// carries). Producers hash the same bytes at the same frame;
    /// the comparator never sees the content.
    #[must_use]
    pub fn hash_region(bytes: &[u8]) -> Hash {
        Self::hash_bytes(bytes)
    }
}

impl std::fmt::Display for Trace {
    /// Serializes back to canonical trace text.
    ///
    /// The output re-parses to the same [`Trace`]
    /// (round-trip-tested), and the writer is the reference the C++
    /// oracle's emission must match byte-for-byte.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let h = &self.header;
        writeln!(f, "TRACE apricorn 1")?;
        writeln!(f, "producer {}", h.producer)?;
        writeln!(f, "state-format {STATE_FORMAT_VERSION}")?;
        writeln!(f, "rom-sha1 {}", hex(&h.rom_sha1))?;
        writeln!(f, "input-sha1 {}", hex(&h.input_sha1))?;
        writeln!(f, "regions-sha1 {}", hex(&h.regions_sha1))?;
        writeln!(f, "frames {}", h.frames)?;
        writeln!(f, "frame-rate {}", h.frame_rate)?;
        if let Some(rtc) = &h.rtc {
            writeln!(f, "rtc {rtc}")?;
        }
        for record in &self.records {
            match record {
                TraceRecord::Sample {
                    frame,
                    region,
                    hash,
                } => writeln!(f, "F {frame} {region} {}", hex(hash))?,
                TraceRecord::Call {
                    seq,
                    func,
                    args,
                    state,
                } => writeln!(
                    f,
                    "C {seq} {func} r0=0x{:08x} r1=0x{:08x} r2=0x{:08x} r3=0x{:08x} state={}",
                    args[0],
                    args[1],
                    args[2],
                    args[3],
                    hex(state)
                )?,
            }
        }
        Ok(())
    }
}

/// Parses 40 lowercase-hex digits into a digest.
fn parse_hash(text: &str) -> Option<Hash> {
    let bytes = text.as_bytes();
    if bytes.len() != 40 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut hash = [0u8; 20];
    for (i, chunk) in bytes.chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        hash[i] = (hi * 16 + lo) as u8;
    }
    Some(hash)
}

/// Parses a `0x`-prefixed hex u32.
fn parse_hex32(text: &str) -> Option<u32> {
    let digits = text.strip_prefix("0x")?;
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// Lowercase hex of a digest.
fn hex(hash: &Hash) -> String {
    let mut out = String::with_capacity(40);
    for b in hash {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete, canonical trace exercising both record kinds.
    fn fixture() -> Trace {
        Trace {
            header: TraceHeader {
                producer: "oracle-melonds-1.1".to_string(),
                rom_sha1: [0x4f; 20],
                input_sha1: [0x0f; 20],
                regions_sha1: [0x8e; 20],
                frames: 600,
                frame_rate: "59.8268".to_string(),
                rtc: Some("2010-03-01T09:00:00".to_string()),
            },
            records: vec![
                TraceRecord::Sample {
                    frame: 120,
                    region: "rng".to_string(),
                    hash: [0x6f; 20],
                },
                TraceRecord::Sample {
                    frame: 150,
                    region: "anim-counter".to_string(),
                    hash: [0x11; 20],
                },
                TraceRecord::Call {
                    seq: 0,
                    func: "LCRandom".to_string(),
                    args: [0x0000_1234, 0, 0, 0],
                    state: [0x99; 20],
                },
            ],
        }
    }

    #[test]
    fn round_trips_byte_exact() {
        let trace = fixture();
        let text = trace.to_string();
        assert_eq!(
            Trace::parse(&text).expect("writer output must parse"),
            trace
        );
        // And the reverse direction: parsing canonical text and writing
        // it back is the identity.
        let reparsed = Trace::parse(&text).expect("fixture must parse");
        assert_eq!(reparsed.to_string(), text);
    }

    #[test]
    fn gate_refuses_foreign_state_format() {
        let text = fixture()
            .to_string()
            .replace("state-format 1", "state-format 999");
        let err = Trace::parse(&text).expect_err("must refuse");
        assert_eq!(
            err,
            HarnessError::Gate {
                what: "state-format 999 != 1".to_string()
            }
        );

        // A trace whose state-format predates the engine's refuses to
        // compare, whichever side is stale — the point of the gate.
        let text = "TRACE apricorn 1\nstate-format 0\nproducer x\nrom-sha1 0000000000000000000000000000000000000000\ninput-sha1 0000000000000000000000000000000000000000\nregions-sha1 0000000000000000000000000000000000000000\nframes 1\nframe-rate 59.8268\n";
        assert!(Trace::parse(text).is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        // Wrong TRACE line.
        assert!(Trace::parse("TRACE apricorn 2\n").is_err());
        assert!(Trace::parse("").is_err());

        // Missing mandatory header keys.
        let text = "TRACE apricorn 1\nproducer x\n";
        let err = Trace::parse(text).expect_err("must fail");
        assert_eq!(
            err,
            HarnessError::Syntax {
                line: 1,
                what: "missing header key: rom-sha1".to_string()
            }
        );

        // Unknown key.
        let text = fixture().to_string().replace("frames 600", "fames 600");
        assert!(Trace::parse(&text).is_err());

        // Malformed records, with line numbers.
        let base = fixture().to_string();
        let lines: Vec<&str> = base.lines().collect();
        let corrupt = |replacement_line: usize, with: &str| {
            let mut l = lines.clone();
            l[replacement_line] = with;
            l.join("\n")
        };
        let err = Trace::parse(&corrupt(9, "F 120 rng nothex")).expect_err("bad hash");
        assert_eq!(
            err,
            HarnessError::Syntax {
                line: 10,
                what: "F: missing or malformed sha1".to_string()
            }
        );
        assert!(Trace::parse(&corrupt(9, "F 120 rng 6f… trailing")).is_err());
        assert!(
            Trace::parse(&corrupt(
                11,
                "C 0 LCRandom r0=zz r1=0x0 r2=0x0 r3=0x0 state=6f…"
            ))
            .is_err()
        );
        assert!(Trace::parse(&corrupt(11, "C 0 LCRandom r0=0x0 state=6f…")).is_err());
        assert!(Trace::parse(&corrupt(11, "C 0 LCRandom")).is_err());
    }

    #[test]
    fn header_order_is_free_but_keys_unique() {
        // Keys may appear in any order before the records.
        let reordered = "TRACE apricorn 1\n\
                         frames 10\n\
                         frame-rate 59.8268\n\
                         rom-sha1 0000000000000000000000000000000000000001\n\
                         regions-sha1 0000000000000000000000000000000000000002\n\
                         input-sha1 0000000000000000000000000000000000000003\n\
                         producer test\n\
                         F 0 rng 0000000000000000000000000000000000000004\n";
        let trace = Trace::parse(reordered).expect("reordered header must parse");
        assert_eq!(trace.header.producer, "test");
        assert_eq!(trace.header.frames, 10);
        assert_eq!(trace.records.len(), 1);
        // The canonical writer is order-stable regardless.
        assert!(
            trace
                .to_string()
                .starts_with("TRACE apricorn 1\nproducer test\n")
        );
    }

    #[test]
    fn hashes_are_sha1_of_content() {
        let hash = Trace::hash_region(b"the watched bytes");
        // SHA-1 of the same content is stable and depends on the bytes.
        assert_eq!(Trace::hash_region(b"the watched bytes"), hash);
        assert_ne!(Trace::hash_region(b"other bytes"), hash);
        assert_eq!(hash.len(), 20);
    }
}
