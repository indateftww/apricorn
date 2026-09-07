//! Phase 4, step 3: the engine's RNG (`apricorn_core::rng`) locked to
//! the *original* functions, draw by draw, through arm-runner.
//!
//! This is the first differential that pits Rust game logic — not the
//! interpreter, not a transcribed C reference — against the retail
//! ARM9 image. For each seeded stream, `SetLCRNGSeed` plants the same
//! state on both machines, then every `LCRandom` draw (and the
//! `GetLCRNGSeed` view behind it) must match `Lcrng::next_u16`
//! exactly; `PRandom` and the mon-encryption LCG (`MonEncryptionLCRNG`,
//! the same recurrence over a caller-owned seed) get the same
//! treatment. If a single bit of the recurrence ever drifts, the draw
//! it produced names the step.
//!
//! `LCRandRange` is `static inline` in pret (`include/math_util.h`),
//! so no pinned body exists to call; its modulo is plain arithmetic
//! over the differentially-pinned draw, covered by the unit tests
//! next to `Lcrng::rand_range`.
//!
//! Runs only when `hg_usa.nds` sits at the repo root (same policy as
//! `arm_hg.rs`; CI has no ROM and skips silently). Every load
//! re-verifies the pin table, so a wrong dump fails before any call.

use apricorn_core::rng::{Lcrng, prandom};
use apricorn_harness::arm::retail::RetailArm9;
use apricorn_harness::pins::{PinMode, PinTable};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// A scratch address above the loaded image (image ends 0x0211_1EF8)
/// and below the stack (0x0230_0000): the mon-encryption seed slot.
const SCRATCH: u32 = 0x0220_0000;

/// Draws per seeded stream — enough to make a recurrence bug (a
/// wrong multiplier, addend, or draw width) unreachable by luck.
const DRAWS: usize = 64;

fn load() -> Option<RetailArm9> {
    let data = match std::fs::read(ROM_PATH) {
        Ok(data) => data,
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            return None;
        }
    };
    let arm9 = match RetailArm9::load(&data) {
        Ok(arm9) => arm9,
        Err(e) => panic!("retail ROM failed to load: {e}"),
    };
    Some(arm9)
}

/// Calls the pinned function `name` with `args`, returning r0.
fn call(arm9: &mut RetailArm9, name: &str, args: &[u32]) -> u32 {
    let table = PinTable::arm9();
    let pin = table.get(name).unwrap_or_else(|| panic!("pin {name}"));
    assert_ne!(pin.mode, PinMode::Data, "{name} is a data pin");
    let entry = pin.address | u32::from(pin.mode == PinMode::Thumb);
    let cpu = arm9.cpu();
    cpu.prepare_call(entry, args);
    let result = cpu
        .run_default()
        .unwrap_or_else(|e| panic!("{name} faulted: {e}"));
    result.r0
}

// ===== LCRandom: the main stream =======================================

#[test]
fn lcrng_matches_the_original_stream() {
    let Some(mut arm9) = load() else { return };

    // The seeds the engine's unit tests pin known draws for (plus the
    // extremes), so a shared constant bug can't hide anywhere.
    for seed in [0x1234u32, 0, 0xFFFF_FFFF, 0xDEAD_BEEF] {
        call(&mut arm9, "SetLCRNGSeed", &[seed]);
        assert_eq!(call(&mut arm9, "GetLCRNGSeed", &[]), seed, "SetLCRNGSeed");

        let mut rng = Lcrng::new(seed);
        for i in 0..DRAWS {
            let got = call(&mut arm9, "LCRandom", &[]);
            assert_eq!(got, u32::from(rng.next_u16()), "seed {seed:#x}, draw {i}");
            // The draw comes from the post-advance state, and
            // GetLCRNGSeed sees exactly that state.
            assert_eq!(call(&mut arm9, "GetLCRNGSeed", &[]), rng.seed());
        }
    }
}

// ===== PRandom: the stateless step =====================================

#[test]
fn prandom_matches_the_original() {
    let Some(mut arm9) = load() else { return };

    for seed in [0x1234u32, 0, 1, 0xFFFF_FFFF, 0x5489] {
        assert_eq!(call(&mut arm9, "PRandom", &[seed]), prandom(seed));
    }
}

// ===== MonEncryptionLCRNG: the same recurrence, local seed ===========

#[test]
fn mon_encryption_lcg_matches_the_original() {
    let Some(mut arm9) = load() else { return };

    // MonEncryptionLCRNG(u32 *seed) advances the caller-owned seed in
    // place and returns its top 16 bits — the same shape as LCRandom
    // over a local Lcrng, which is exactly how the engine models it.
    let mut rng = Lcrng::new(0xDEAD_BEEF);
    arm9
        .cpu()
        .mem_mut()
        .write32(SCRATCH, 0xDEAD_BEEF)
        .expect("seed slot write");

    for i in 0..DRAWS {
        let got = call(&mut arm9, "MonEncryptionLCRNG", &[SCRATCH]);
        assert_eq!(got, u32::from(rng.next_u16()), "mon draw {i}");
        let state = arm9
            .cpu()
            .mem()
            .read32(SCRATCH)
            .expect("seed slot read");
        assert_eq!(state, rng.seed(), "mon state {i}");
    }
}