//! Trace comparison — where the harness says EQUIVALENT.
//!
//! [`compare`] walks two traces in lockstep (the pair is already gated
//! by the header parse: both sides ran the same engine state-format
//! version) and returns a [`Report`]:
//!
//! * a `hard`-bucket region mismatch (or any structural mismatch) is a
//!   **divergence** — reported immediately as [`Verdict::Diverged`] with
//!   the frame and region;
//! * a `drift`-bucket region mismatch is **tolerated** — recorded in the
//!   report's drift list, never fatal;
//! * a record present on only one side is a divergence — a deterministic
//!   failure, never silence.
//!
//! Call (`C`) records are always exact: the arguments must match before
//! the outputs are comparable, so an argument or post-call-state
//! mismatch is a divergence regardless of buckets.
//!
//! Gates: pairs differing in `rom-sha1` or `regions-sha1` refuse to
//! compare ([`HarnessError::Gate`]) — a bogus divergence list from the
//! wrong ROM or config is worse than no answer. Without a
//! [`RegionSet`] every mismatch is treated as `hard`.

use crate::regions::RegionSet;
use crate::trace::{Hash, Trace, TraceRecord};
use crate::{HarnessError, Verdict};

/// One tolerated (`drift`-bucket) mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    /// Frame the mismatch was observed at.
    pub frame: u32,
    /// Region name.
    pub region: String,
    /// The expected side's hash.
    pub expected: Hash,
    /// The actual side's hash.
    pub actual: Hash,
}

/// The first fatal divergence, as the report presents it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstDivergence {
    /// Frame the divergence was observed at (the record's frame; for a
    /// record missing on one side, that record's frame).
    pub frame: u32,
    /// The region or function the divergence is attributed to.
    pub what: String,
    /// Human-readable detail: both sides' values where they exist.
    pub detail: String,
}

/// The comparator's full result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The verdict: `Equivalent`, or `Diverged` from the first
    /// hard-bucket mismatch.
    pub verdict: Verdict,
    /// The first divergence, when the verdict is `Diverged`.
    pub first: Option<FirstDivergence>,
    /// Drift-bucket mismatches tolerated along the way (in walk order).
    pub drift: Vec<Drift>,
}

impl Report {
    /// Records a divergence and seals the verdict.
    fn diverged(frame: u32, what: String, detail: String) -> Self {
        Self {
            verdict: Verdict::Diverged { frame },
            first: Some(FirstDivergence {
                frame,
                what,
                detail,
            }),
            drift: Vec::new(),
        }
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for drift in &self.drift {
            writeln!(
                f,
                "drift (non-fatal): frame {} region {} (expected {}, actual {})",
                drift.frame,
                drift.region,
                hex(&drift.expected),
                hex(&drift.actual)
            )?;
        }
        match &self.first {
            Some(first) => {
                writeln!(
                    f,
                    "first divergence: frame {} {} ({})",
                    first.frame, first.what, first.detail
                )?;
                writeln!(f, "verdict: DIVERGED frame {} {}", first.frame, first.what)
            }
            None => match self.drift.len() {
                0 => write!(f, "verdict: EQUIVALENT"),
                n => write!(f, "verdict: EQUIVALENT ({n} drift mismatches tolerated)"),
            },
        }
    }
}

/// Compares two traces in lockstep.
///
/// `regions` supplies the buckets: with it, a region missing from the
/// config is a gate error (the trace and config disagree); without it,
/// every mismatch is treated as `hard`.
///
/// # Errors
/// Returns a [`HarnessError::Gate`] when the pair ran different ROMs
/// or different region configs, or when the trace names a region the
/// config does not know.
pub fn compare(
    expected: &Trace,
    actual: &Trace,
    regions: Option<&RegionSet>,
) -> Result<Report, HarnessError> {
    if expected.header.rom_sha1 != actual.header.rom_sha1 {
        return Err(HarnessError::Gate {
            what: format!(
                "rom-sha1 mismatch: expected {}, actual {}",
                hex(&expected.header.rom_sha1),
                hex(&actual.header.rom_sha1)
            ),
        });
    }
    if expected.header.regions_sha1 != actual.header.regions_sha1 {
        return Err(HarnessError::Gate {
            what: format!(
                "regions-sha1 mismatch: expected {}, actual {}",
                hex(&expected.header.regions_sha1),
                hex(&actual.header.regions_sha1)
            ),
        });
    }

    // A config that does not know a trace's region is a gate error up
    // front — never lazily on the first mismatch, which would let two
    // producers pass silently on an unwatched region.
    if let Some(set) = regions {
        for record in expected.records.iter().chain(&actual.records) {
            if let TraceRecord::Sample { region, .. } = record
                && set.by_name(region).is_none()
            {
                return Err(HarnessError::Gate {
                    what: format!("trace references region '{region}' not in the config"),
                });
            }
        }
    }
    // The bucket of a sampled region: hard without a config, drift
    // where the config says so (the config knows every name by now).
    let bucket_of = |region: &str| -> bool {
        match regions {
            None => true,
            Some(set) => set
                .by_name(region)
                .is_some_and(|r| r.bucket == crate::regions::Bucket::Hard),
        }
    };

    let mut drift = Vec::new();
    for pair in expected.records.iter().zip(&actual.records) {
        let (a, b) = pair;
        let report = match (a, b) {
            (
                TraceRecord::Sample {
                    frame,
                    region,
                    hash,
                },
                TraceRecord::Sample {
                    frame: frame_b,
                    region: region_b,
                    hash: hash_b,
                },
            ) => {
                if frame != frame_b {
                    Report::diverged(
                        (*frame).min(*frame_b),
                        region.clone(),
                        format!("frame mismatch: expected frame {frame}, actual frame {frame_b}"),
                    )
                } else if region != region_b {
                    Report::diverged(
                        *frame,
                        region.clone(),
                        format!("region mismatch: expected {region}, actual {region_b}"),
                    )
                } else if hash == hash_b {
                    continue;
                } else if bucket_of(region) {
                    Report::diverged(
                        *frame,
                        region.clone(),
                        format!(
                            "hard bucket: expected {}, actual {}",
                            hex(hash),
                            hex(hash_b)
                        ),
                    )
                } else {
                    drift.push(Drift {
                        frame: *frame,
                        region: region.clone(),
                        expected: *hash,
                        actual: *hash_b,
                    });
                    continue;
                }
            }
            (
                TraceRecord::Call {
                    seq,
                    func,
                    args,
                    state,
                },
                TraceRecord::Call {
                    seq: seq_b,
                    func: func_b,
                    args: args_b,
                    state: state_b,
                },
            ) => {
                if seq != seq_b {
                    Report::diverged(
                        0,
                        func.clone(),
                        format!("probe order mismatch: expected seq {seq}, actual seq {seq_b}"),
                    )
                } else if func != func_b {
                    Report::diverged(
                        0,
                        func.clone(),
                        format!("probe mismatch: expected {func}, actual {func_b}"),
                    )
                } else if args != args_b {
                    Report::diverged(
                        0,
                        func.clone(),
                        format!(
                            "argument mismatch: expected [{:#010x} {:#010x} {:#010x} {:#010x}], actual [{:#010x} {:#010x} {:#010x} {:#010x}]",
                            args[0],
                            args[1],
                            args[2],
                            args[3],
                            args_b[0],
                            args_b[1],
                            args_b[2],
                            args_b[3]
                        ),
                    )
                } else if state != state_b {
                    Report::diverged(
                        0,
                        func.clone(),
                        format!(
                            "post-call state mismatch: expected {}, actual {}",
                            hex(state),
                            hex(state_b)
                        ),
                    )
                } else {
                    continue;
                }
            }
            (a, b) => Report::diverged(
                frame_of(a),
                what_of(a, b),
                format!(
                    "record kind mismatch: expected {}, actual {}",
                    kind_of(a),
                    kind_of(b)
                ),
            ),
        };
        return Ok(with_drift(report, drift));
    }

    // One side ran out of records first: the first unpaired record.
    match expected.records.len().cmp(&actual.records.len()) {
        std::cmp::Ordering::Greater => {
            let record = &expected.records[actual.records.len()];
            return Ok(with_drift(
                Report::diverged(
                    frame_of(record),
                    what_alone(record),
                    "record missing on the actual side".to_string(),
                ),
                drift,
            ));
        }
        std::cmp::Ordering::Less => {
            let record = &actual.records[expected.records.len()];
            return Ok(with_drift(
                Report::diverged(
                    frame_of(record),
                    what_alone(record),
                    "record missing on the expected side".to_string(),
                ),
                drift,
            ));
        }
        std::cmp::Ordering::Equal => {}
    }

    Ok(Report {
        verdict: Verdict::Equivalent,
        first: None,
        drift,
    })
}

/// Attaches walk-order drift to an already-diverged report.
fn with_drift(mut report: Report, drift: Vec<Drift>) -> Report {
    report.drift = drift;
    report
}

/// The frame a record was emitted at (calls report 0: probes are
/// sequenced, not frame-timed).
fn frame_of(record: &TraceRecord) -> u32 {
    match record {
        TraceRecord::Sample { frame, .. } => *frame,
        TraceRecord::Call { .. } => 0,
    }
}

/// The name a kind-mismatched record pair is attributed to.
fn what_of(expected: &TraceRecord, actual: &TraceRecord) -> String {
    match (expected, actual) {
        (TraceRecord::Sample { region, .. }, _) => region.clone(),
        (TraceRecord::Call { func, .. }, _) => func.clone(),
    }
    .to_string()
}

/// The name an unpaired record is attributed to.
fn what_alone(record: &TraceRecord) -> String {
    match record {
        TraceRecord::Sample { region, .. } => region.clone(),
        TraceRecord::Call { func, .. } => func.clone(),
    }
}

/// The record kind, for mismatch detail.
fn kind_of(record: &TraceRecord) -> &'static str {
    match record {
        TraceRecord::Sample { .. } => "F (sample)",
        TraceRecord::Call { .. } => "C (call)",
    }
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
    use crate::trace::{Trace, TraceHeader, TraceRecord};

    /// A trace header both fixtures share (same ROM, same regions).
    fn header() -> TraceHeader {
        TraceHeader {
            producer: "test".to_string(),
            rom_sha1: [0x4f; 20],
            input_sha1: [0x0f; 20],
            regions_sha1: [0x8e; 20],
            frames: 600,
            frame_rate: "59.8268".to_string(),
            rtc: Some("2010-03-01T09:00:00".to_string()),
        }
    }

    /// A trace with the given records.
    fn trace(records: Vec<TraceRecord>) -> Trace {
        Trace {
            header: header(),
            records,
        }
    }

    fn sample(frame: u32, region: &str, hash: u8) -> TraceRecord {
        TraceRecord::Sample {
            frame,
            region: region.to_string(),
            hash: [hash; 20],
        }
    }

    fn call(seq: u32, func: &str, arg0: u32, state: u8) -> TraceRecord {
        TraceRecord::Call {
            seq,
            func: func.to_string(),
            args: [arg0, 0, 0, 0],
            state: [state; 20],
        }
    }

    /// The config: rng hard, anim-counter drift.
    fn regions() -> RegionSet {
        RegionSet::parse("rng hard 0x02001234 4 1\nanim-counter drift 0x020089ab 4 30\n")
            .expect("fixture config must parse")
    }

    #[test]
    fn identical_traces_are_equivalent() {
        let records = vec![
            sample(0, "rng", 1),
            sample(1, "rng", 2),
            call(0, "LCRandom", 0x1234, 3),
        ];
        let expected = trace(records.clone());
        let actual = trace(records);
        let report = compare(&expected, &actual, Some(&regions())).expect("must compare");
        assert_eq!(report.verdict, Verdict::Equivalent);
        assert_eq!(report.first, None);
        assert!(report.drift.is_empty());
        assert_eq!(report.to_string(), "verdict: EQUIVALENT");
    }

    #[test]
    fn hard_mismatch_diverges_with_frame_and_region() {
        let expected = trace(vec![sample(311, "rng", 1), sample(312, "rng", 2)]);
        let actual = trace(vec![sample(311, "rng", 1), sample(312, "rng", 3)]);
        let report = compare(&expected, &actual, Some(&regions())).expect("must compare");
        assert_eq!(report.verdict, Verdict::Diverged { frame: 312 });
        let first = report.first.clone().expect("divergence present");
        assert_eq!(first.frame, 312);
        assert_eq!(first.what, "rng");
        assert!(first.detail.contains("hard bucket"));
        assert_eq!(
            report.to_string(),
            format!(
                "first divergence: frame 312 rng (hard bucket: expected {}, actual {})\nverdict: DIVERGED frame 312 rng\n",
                hex(&[2u8; 20]),
                hex(&[3u8; 20])
            )
        );
    }

    #[test]
    fn drift_mismatch_is_tolerated() {
        let expected = trace(vec![sample(289, "anim-counter", 1), sample(290, "rng", 1)]);
        let actual = trace(vec![sample(289, "anim-counter", 2), sample(290, "rng", 1)]);
        let report = compare(&expected, &actual, Some(&regions())).expect("must compare");
        assert_eq!(report.verdict, Verdict::Equivalent);
        assert_eq!(report.drift.len(), 1);
        assert_eq!(report.drift[0].frame, 289);
        assert_eq!(report.drift[0].region, "anim-counter");
        assert_eq!(
            report.to_string(),
            format!(
                "drift (non-fatal): frame 289 region anim-counter (expected {}, actual {})\nverdict: EQUIVALENT (1 drift mismatches tolerated)",
                hex(&[1u8; 20]),
                hex(&[2u8; 20])
            )
        );
    }

    #[test]
    fn without_a_config_every_mismatch_is_hard() {
        let expected = trace(vec![sample(289, "anim-counter", 1)]);
        let actual = trace(vec![sample(289, "anim-counter", 2)]);
        let report = compare(&expected, &actual, None).expect("must compare");
        assert_eq!(report.verdict, Verdict::Diverged { frame: 289 });
    }

    #[test]
    fn missing_and_extra_records_diverge() {
        // Shorter actual side: the missing record's frame is the verdict.
        let expected = trace(vec![sample(0, "rng", 1), sample(120, "rng", 1)]);
        let actual = trace(vec![sample(0, "rng", 1)]);
        let report = compare(&expected, &actual, Some(&regions())).expect("must compare");
        assert_eq!(report.verdict, Verdict::Diverged { frame: 120 });
        assert!(
            report
                .first
                .unwrap()
                .detail
                .contains("missing on the actual side")
        );

        // Longer actual side: the extra record is the divergence.
        let expected = trace(vec![sample(0, "rng", 1)]);
        let actual = trace(vec![sample(0, "rng", 1), sample(30, "rng", 1)]);
        let report = compare(&expected, &actual, Some(&regions())).expect("must compare");
        assert_eq!(report.verdict, Verdict::Diverged { frame: 30 });
        assert!(
            report
                .first
                .unwrap()
                .detail
                .contains("missing on the expected side")
        );
    }

    #[test]
    fn structural_mismatches_diverge() {
        // Frame mismatch on paired records.
        let expected = trace(vec![sample(120, "rng", 1)]);
        let actual = trace(vec![sample(121, "rng", 1)]);
        let report = compare(&expected, &actual, Some(&regions())).expect("must compare");
        assert_eq!(report.verdict, Verdict::Diverged { frame: 120 });

        // Region mismatch (compared without a config, so the unknown
        // name on the actual side is a divergence, not a gate error).
        let actual = trace(vec![sample(120, "mt", 1)]);
        let report = compare(&expected, &actual, None).expect("must compare");
        assert_eq!(report.first.unwrap().what, "rng");

        // Record kind mismatch.
        let actual = trace(vec![call(0, "rng", 0, 0)]);
        let report = compare(&expected, &actual, Some(&regions())).expect("must compare");
        assert!(
            report
                .first
                .unwrap()
                .detail
                .contains("record kind mismatch")
        );
    }

    #[test]
    fn call_records_are_always_exact() {
        // Argument mismatch: divergence even before the outputs.
        let expected = trace(vec![call(0, "LCRandom", 0x1234, 1)]);
        let actual = trace(vec![call(0, "LCRandom", 0x5678, 1)]);
        let report = compare(&expected, &actual, None).expect("must compare");
        assert_eq!(report.verdict, Verdict::Diverged { frame: 0 });
        assert!(report.first.unwrap().detail.contains("argument mismatch"));

        // Post-call state mismatch.
        let actual = trace(vec![call(0, "LCRandom", 0x1234, 2)]);
        let report = compare(&expected, &actual, None).expect("must compare");
        assert!(report.first.unwrap().detail.contains("post-call state"));

        // Probe order / function mismatches.
        let actual = trace(vec![call(1, "LCRandom", 0x1234, 1)]);
        assert!(
            compare(&expected, &actual, None)
                .unwrap()
                .first
                .unwrap()
                .detail
                .contains("probe order")
        );
        let actual = trace(vec![call(0, "MTRandom", 0x1234, 1)]);
        assert!(
            compare(&expected, &actual, None)
                .unwrap()
                .first
                .unwrap()
                .detail
                .contains("probe mismatch")
        );
    }

    #[test]
    fn gates_refuse_wrong_pairings() {
        let expected = trace(vec![]);
        // Different ROM.
        let mut actual = trace(vec![]);
        actual.header.rom_sha1 = [0x99; 20];
        let err = compare(&expected, &actual, None).expect_err("must refuse");
        assert!(matches!(err, HarnessError::Gate { .. }));
        assert!(err.to_string().contains("rom-sha1 mismatch"));

        // Different regions config.
        let mut actual = trace(vec![]);
        actual.header.regions_sha1 = [0x77; 20];
        let err = compare(&expected, &actual, None).expect_err("must refuse");
        assert!(err.to_string().contains("regions-sha1 mismatch"));

        // Trace names a region the config does not know.
        let expected = trace(vec![sample(0, "unknown-region", 1)]);
        let actual = trace(vec![sample(0, "unknown-region", 1)]);
        let err = compare(&expected, &actual, Some(&regions())).expect_err("must refuse");
        assert!(err.to_string().contains("not in the config"));
    }
}
