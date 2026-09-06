//! Watched-region configuration (`regions.conf`).
//!
//! A corpus case lists the game-state ranges the harness watches: the
//! RNG state, the party, the player's position, flags, and so on. Every
//! producer (the melonDS oracle, `arm-runner`, and — from Phase 4 — the
//! headless engine) hashes the same regions at the same frames, so the
//! comparator can attribute a divergence to a specific subsystem.
//!
//! The format is line-oriented, one region per line, whitespace-separated:
//!
//! ```text
//! # name         bucket  address     size   sample  note
//! rng            hard    0x02001234  4      1       sLCRNG_State (arm9 bss)
//! mt             hard    0x02004567  2496   1       sMTRNG_State (624 u32)
//! anim-counter   drift   0x020089ab  4      30      OAM/anim scratch
//! ```
//!
//! * `name` — the symbolic id used in traces and diff output. Addresses
//!   are pinned data ([crate] `pins`), never baked into Rust code.
//! * `bucket` — `hard`: a mismatch is a divergence. `drift`: a mismatch
//!   is recorded but non-fatal (animation counters and other state that
//!   may legitimately run out of step).
//! * `address` — `0x`-prefixed hex address in ARM9 memory space.
//! * `size` — bytes hashed per sample.
//! * `sample` — hash every N frames (1 = every frame).
//! * `note` — optional free text (rest of the line; documentation only).
//!
//! **Canonical form** (what `regions-sha1` in a trace header hashes):
//! for each region in file order, `name bucket 0x%08X size sample`
//! newline-terminated — exactly the five identifying fields, in order,
//! single-spaced, lowercase hex, notes and comments excluded. This is
//! deliberately `printf`-able so the C++ oracle computes the identical
//! hash; see `docs/equivalence.md`.

use crate::HarnessError;
use sha1::{Digest, Sha1};

/// Whether a mismatch in this region is a divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    /// A mismatch is a divergence.
    Hard,
    /// A mismatch is recorded but non-fatal.
    Drift,
}

impl Bucket {
    /// The canonical `regions.conf` spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Hard => "hard",
            Bucket::Drift => "drift",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "hard" => Some(Bucket::Hard),
            "drift" => Some(Bucket::Drift),
            _ => None,
        }
    }
}

/// One watched memory region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// The symbolic id used in traces and diff output.
    pub name: String,
    /// Whether a mismatch here is fatal.
    pub bucket: Bucket,
    /// ARM9 address, `0x`-prefixed hex in `regions.conf`.
    pub address: u32,
    /// Bytes hashed per sample.
    pub size: u32,
    /// Hash every N frames (1 = every frame).
    pub sample: u32,
    /// Free-text documentation; excluded from the canonical form.
    pub note: Option<String>,
}

/// A parsed `regions.conf`: the regions in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSet {
    regions: Vec<Region>,
}

impl RegionSet {
    /// Parses `regions.conf` text.
    ///
    /// Blank lines and full-line `#` comments are skipped; anything else
    /// must be `name bucket address size sample [note…]`. Duplicate names
    /// are rejected (traces key records by name).
    ///
    /// # Errors
    /// Returns a [`HarnessError::Syntax`] naming the line of any
    /// malformed or duplicated entry.
    pub fn parse(text: &str) -> Result<Self, HarnessError> {
        let mut regions: Vec<Region> = Vec::new();
        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.trim_end();
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Fields 1-5 identify the region; everything after field 5 is
            // the note (free text, may contain spaces).
            let Some((fields, past_sample)) = scan_fields(line, 5) else {
                return Err(HarnessError::Syntax {
                    line: line_no,
                    what: "expected: name bucket address size sample [note]".to_string(),
                });
            };
            let name = fields[0].to_string();
            let what = |msg: &str| HarnessError::Syntax {
                line: line_no,
                what: format!("region {name}: {msg}"),
            };

            let bucket =
                Bucket::parse(fields[1]).ok_or_else(|| what("bucket must be hard or drift"))?;
            let address =
                parse_hex32(fields[2]).ok_or_else(|| what("address must be 0x-prefixed hex"))?;
            let size = fields[3]
                .parse::<u32>()
                .ok()
                .filter(|&s| s > 0)
                .ok_or_else(|| what("size must be a positive decimal"))?;
            let sample = fields[4]
                .parse::<u32>()
                .ok()
                .filter(|&s| s > 0)
                .ok_or_else(|| what("sample must be a positive decimal"))?;
            // The note is the free-text remainder after the sample field,
            // preserved verbatim (it may contain spaces).
            let note = line[past_sample..].trim();
            let note = (!note.is_empty()).then(|| note.to_string());

            if regions.iter().any(|r| r.name == name) {
                return Err(what("duplicate region name"));
            }
            regions.push(Region {
                name,
                bucket,
                address,
                size,
                sample,
                note,
            });
        }
        Ok(Self { regions })
    }

    /// The regions in file order.
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// Looks a region up by name.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&Region> {
        self.regions.iter().find(|r| r.name == name)
    }

    /// The canonical form: for each region in file order,
    /// `name bucket 0x%08X size sample`, newline-terminated. Notes and
    /// comments are excluded — this is the bytes `regions-sha1` hashes,
    /// and it must be byte-identical from every producer (the C++
    /// oracle prints it with one `printf`).
    #[must_use]
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        for r in &self.regions {
            out.push_str(&format!(
                "{} {} 0x{:08x} {} {}\n",
                r.name,
                r.bucket.as_str(),
                r.address,
                r.size,
                r.sample
            ));
        }
        out
    }

    /// SHA-1 over [`RegionSet::canonical`], as lowercase hex — the
    /// `regions-sha1` a trace header carries.
    #[must_use]
    pub fn sha1_hex(&self) -> String {
        let mut hasher = Sha1::new();
        hasher.update(self.canonical().as_bytes());
        hex(&hasher.finalize())
    }
}

impl std::fmt::Display for RegionSet {
    /// Serializes the set back to `regions.conf` form — the identifying
    /// fields column-aligned, with the note column restored. Re-parsing
    /// the output yields the same [`RegionSet`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "# name             bucket  address     size    sample  note"
        )?;
        for r in &self.regions {
            writeln!(
                f,
                "{:<17} {:<7} 0x{:08x} {:<7} {:<7} {}",
                r.name,
                r.bucket.as_str(),
                r.address,
                r.size,
                r.sample,
                r.note.as_deref().unwrap_or("")
            )?;
        }
        Ok(())
    }
}

/// Parses a `0x`-prefixed hex u32 (lower- or uppercase digits).
fn parse_hex32(text: &str) -> Option<u32> {
    let digits = text.strip_prefix("0x")?;
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// Collects the first `n` whitespace-separated fields of `line`, with
/// the byte offset just past the nth field (so the caller can recover
/// any free-text remainder). Returns `None` on fewer than `n` fields.
fn scan_fields(line: &str, n: usize) -> Option<(Vec<&str>, usize)> {
    let mut fields = Vec::with_capacity(n);
    let mut pos = 0;
    for _ in 0..n {
        let rest = &line[pos..];
        let start = pos + (rest.len() - rest.trim_start().len());
        if start >= line.len() {
            return None;
        }
        let end = line[start..]
            .find(char::is_whitespace)
            .map_or(line.len(), |w| start + w);
        fields.push(&line[start..end]);
        pos = end;
    }
    Some((fields, pos))
}

/// Lowercase hex of a digest.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example configuration from the module docs.
    const CONF: &str = "# name         bucket  address     size   sample  note\n\
                         \n\
                         rng            hard    0x02001234  4      1       sLCRNG_State (arm9 bss)\n\
                         mt             hard    0x02004567  2496   1       sMTRNG_State (624 u32)\n\
                         # a comment between regions\n\
                         anim-counter   drift   0x020089AB 4      30      OAM/anim scratch\n";

    #[test]
    fn parses_regions_conf() {
        let set = RegionSet::parse(CONF).expect("fixture must parse");
        assert_eq!(set.regions().len(), 3);

        let rng = set.by_name("rng").expect("rng present");
        assert_eq!(rng.bucket, Bucket::Hard);
        assert_eq!(rng.address, 0x02001234);
        assert_eq!(rng.size, 4);
        assert_eq!(rng.sample, 1);
        assert_eq!(rng.note.as_deref(), Some("sLCRNG_State (arm9 bss)"));

        // Uppercase hex is accepted; the canonical form lowercases it.
        let anim = set.by_name("anim-counter").expect("anim present");
        assert_eq!(anim.bucket, Bucket::Drift);
        assert_eq!(anim.address, 0x020089AB);

        // A region without trailing note parses to None.
        let bare = "solo hard 0x02000000 2 5\n";
        let set = RegionSet::parse(bare).expect("bare fixture must parse");
        assert_eq!(set.by_name("solo").unwrap().note, None);
    }

    #[test]
    fn canonical_form_excludes_notes_and_comments() {
        let set = RegionSet::parse(CONF).expect("fixture must parse");
        assert_eq!(
            set.canonical(),
            "rng hard 0x02001234 4 1\n\
             mt hard 0x02004567 2496 1\n\
             anim-counter drift 0x020089ab 4 30\n"
        );

        // The same regions with different notes/comments/spacing hash
        // identically — the canonical form is the semantic identity.
        let respaced = "rng hard 0x02001234 4 1\nmt hard 0x02004567 2496 1\nanim-counter drift 0x020089ab 4 30\n";
        let other = RegionSet::parse(respaced).expect("respaced must parse");
        assert_eq!(set.sha1_hex(), other.sha1_hex());
        assert_eq!(set.sha1_hex().len(), 40);
    }

    #[test]
    fn round_trips_through_to_string() {
        let set = RegionSet::parse(CONF).expect("fixture must parse");
        let written = set.to_string();
        let reparsed = RegionSet::parse(&written).expect("writer output must parse");
        assert_eq!(reparsed, set);
        // The canonical form is a formatting-independent invariant.
        assert_eq!(reparsed.canonical(), set.canonical());
    }

    #[test]
    fn rejects_broken_configs() {
        // Wrong bucket spelling.
        assert!(RegionSet::parse("r soft 0x02000000 4 1\n").is_err());
        // Address without 0x prefix.
        assert!(RegionSet::parse("r hard 20000000 4 1\n").is_err());
        // Bad sizes/samples.
        assert!(RegionSet::parse("r hard 0x02000000 0 1\n").is_err());
        assert!(RegionSet::parse("r hard 0x02000000 4 0\n").is_err());
        assert!(RegionSet::parse("r hard 0x02000000 4 x\n").is_err());
        // Missing fields.
        assert!(RegionSet::parse("r hard 0x02000000 4\n").is_err());
        // Duplicate names.
        assert!(RegionSet::parse("r hard 0x02000000 4 1\nr drift 0x02000004 4 1\n").is_err());

        // Errors name the offending line.
        let err = RegionSet::parse("# header\nbad line\n").expect_err("must fail");
        assert_eq!(
            err,
            HarnessError::Syntax {
                line: 2,
                what: "expected: name bucket address size sample [note]".to_string()
            }
        );
        let err = RegionSet::parse("r soft 0x02000000 4 1\n").expect_err("must fail");
        assert_eq!(
            err,
            HarnessError::Syntax {
                line: 1,
                what: "region r: bucket must be hard or drift".to_string()
            }
        );
    }

    #[test]
    fn empty_config_is_empty() {
        let set = RegionSet::parse("").expect("empty parses");
        assert!(set.regions().is_empty());
        assert_eq!(set.canonical(), "");
        assert_eq!(set.by_name("anything"), None);
    }
}
