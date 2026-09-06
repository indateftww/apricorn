//! The first true differential: one pinned function sequence, run on
//! two different machines, must produce the same trace records.
//!
//! The oracle runs `SetLCRNGSeed(0x1234)` then `LCRandom` as probes at
//! frame 20's boundary of the *whole emulated console* (booted retail
//! HeartGold, interrupts live, both CPUs running); arm-runner runs
//! the same functions on its bare ARMv5TE interpreter with the same
//! entry state (r0–r3 = args, r12 = 0, r13 = scratch stack, lr =
//! sentinel). Both hash the same watched region after each call. If
//! the register results *or* the state hashes differ, the comparator
//! names the record — that is the entire Phase 2 methodology in one
//! test.
//!
//! Frame 20, not 0: DirectBoot enters the game at its compressed ARM9
//! binary and crt0 decompresses the static main in place over the
//! first few frames — a frame-0 probe would jump into the still-
//! compressed image. Twenty frames in, the pinned functions are real
//! code in both machines.
//!
//! The records before the probes are frame samples of the watched
//! region, which the game has not touched yet (it seeds this LCG from
//! the clock around frame 185); arm-runner mirrors them with its own
//! pre-probe memory, which is identically zeroed bss (the pins table
//! records exactly that hash for this address). If the game ever
//! reaches the region earlier, the comparator catches the mismatch —
//! the mirror is honest, not assumed.
//!
//! Runs only when the ROM (`hg_usa.nds`) and the oracle binary
//! (`out/oracle/apricorn-oracle[.exe]`) are present; CI skips silently.

use std::path::Path;

use apricorn_harness::arm::retail::RetailArm9;
use apricorn_harness::oracle::{OracleRun, ProbeSpec};
use apricorn_harness::pins::{PinMode, PinTable};
use apricorn_harness::regions::RegionSet;
use apricorn_harness::trace::{Trace, TraceHeader, TraceRecord};
use apricorn_harness::{HarnessError, diff};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// The watched region: the game's boot LCG state slot. The probes
/// establish its contents themselves, so both machines see identical
/// bytes after the calls regardless of how far apart their boots are.
const REGIONS_CONF: &str = "# name bucket address size sample\nrng hard 0x021D15A8 4 1\n";

const RTC: &str = "2010-03-01T09:00:00";

/// The probes run after this many frames (see the module docs), so the
/// oracle trace carries `PROBE_FRAME` frame samples before its `C`
/// records.
const PROBE_FRAME: u32 = 20;

fn setup() -> Option<()> {
    if !Path::new(ROM_PATH).is_file() {
        eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
        return None;
    }
    if apricorn_harness::oracle::find_binary().is_none() {
        eprintln!("skipping: out/oracle/apricorn-oracle not built (run oracle/setup.ps1)");
        return None;
    }
    Some(())
}

/// The probe schedule: seed the LCG, then draw from it, both at the
/// same frame boundary (no game execution between them). Entries come
/// from the pin table, Thumb bit included — the same entries the ARM
/// side uses.
fn probes() -> Vec<ProbeSpec> {
    let table = PinTable::arm9();
    let entry = |name: &str| {
        let pin = table.get(name).unwrap_or_else(|| panic!("pin {name}"));
        assert_ne!(pin.mode, PinMode::Data, "{name} is a data pin");
        pin.address | u32::from(pin.mode == PinMode::Thumb)
    };
    vec![
        ProbeSpec {
            frame: PROBE_FRAME,
            entry: entry("SetLCRNGSeed"),
            args: vec![0x1234],
            name: "SetLCRNGSeed".to_string(),
        },
        ProbeSpec {
            frame: PROBE_FRAME,
            entry: entry("LCRandom"),
            args: vec![],
            name: "LCRandom".to_string(),
        },
    ]
}

/// arm-runner's half of the differential: the same probe sequence on the
/// bare interpreter, emitted as a trace so `diff::compare` can walk both
/// sides in lockstep.
fn arm_runner_trace(probes: &[ProbeSpec], regions: &RegionSet, rom: &[u8]) -> Trace {
    let mut arm9 = RetailArm9::load(rom).expect("retail ROM failed to load");
    let region = regions.regions()[0].clone();

    // The region hash of the pre-probe machine: both machines hold
    // zeroed bss here (the oracle's own frames 0..19 will carry this
    // same hash; the comparator proves it, not this comment).
    let region_bytes = |arm9: &mut RetailArm9| -> Vec<u8> {
        arm9.cpu()
            .mem()
            .read_block(region.address, region.size)
            .unwrap_or_else(|e| panic!("region {}: {e}", region.name))
            .to_vec()
    };
    let pre_probe_hash = Trace::hash_region(&region_bytes(&mut arm9));

    let mut records = Vec::new();
    for frame in 0..PROBE_FRAME {
        records.push(TraceRecord::Sample {
            frame,
            region: region.name.clone(),
            hash: pre_probe_hash,
        });
    }

    for (seq, probe) in probes.iter().enumerate() {
        arm9.cpu().prepare_call(probe.entry, &probe.args);
        let result = arm9
            .cpu()
            .run_default()
            .unwrap_or_else(|e| panic!("{} faulted: {e}", probe.name));

        // The C record's state: SHA-1 over the concatenated regions, in
        // regions.conf order — identical to the oracle's StateHash.
        records.push(TraceRecord::Call {
            seq: seq as u32,
            func: probe.name.clone(),
            args: [result.r0, result.r1, result.r2, result.r3],
            state: Trace::hash_region(&region_bytes(&mut arm9)),
        });
    }

    Trace {
        header: TraceHeader {
            producer: "arm-runner".to_string(),
            rom_sha1: Trace::hash_bytes(rom),
            input_sha1: Trace::hash_bytes(&[]),
            regions_sha1: Trace::hash_bytes(regions.canonical().as_bytes()),
            frames: PROBE_FRAME,
            frame_rate: "59.8268".to_string(),
            rtc: None,
        },
        records,
    }
}

#[test]
fn fn_lcg_seeded_oracle_matches_arm_runner() {
    let Some(()) = setup() else { return };

    let regions = RegionSet::parse(REGIONS_CONF).expect("regions.conf parses");
    let probes = probes();

    // The oracle side: booted retail ROM, 20 frames, then the probes at
    // frame 20's boundary.
    let oracle_run = OracleRun {
        rom: Path::new(ROM_PATH),
        regions: &regions,
        input: None,
        probes: &probes,
        frames: PROBE_FRAME,
        rtc: Some(RTC),
        producer: None,
    };
    let oracle_trace = oracle_run.run().expect("oracle run failed");

    // Both sides must have run the same record sequence shape: 20 frame
    // samples, then the two probes.
    assert_eq!(oracle_trace.records.len(), PROBE_FRAME as usize + 2);

    // The arm-runner side must pin the same passthrough hashes the
    // oracle pinned, or the gate refuses the pair.
    let rom = std::fs::read(ROM_PATH).expect("read ROM");
    let mut arm_trace = arm_runner_trace(&probes, &regions, &rom);
    arm_trace.header.input_sha1 = oracle_trace.header.input_sha1;
    arm_trace.header.regions_sha1 = oracle_trace.header.regions_sha1;

    let report = match diff::compare(&oracle_trace, &arm_trace, None) {
        Ok(report) => report,
        Err(HarnessError::Gate { what }) => {
            panic!("diff gate refused the pair: {what}")
        }
        Err(e) => panic!("diff failed: {e}"),
    };
    assert_eq!(
        report.verdict,
        apricorn_harness::Verdict::Equivalent,
        "oracle and arm-runner diverged: {report:?}"
    );
}
