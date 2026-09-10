//! arm-runner known-value tests against the retail HeartGold (US)
//! ARM9 image: the interpreter calls the *original* pinned functions
//! and the results must match reference implementations transcribed
//! from pret's C (`refs/pokeheartgold/src/math_util.c`) and the
//! NitroSDK CRC header.
//!
//! These run only when `hg_usa.nds` sits at the repo root (same policy
//! as `pins_hg.rs`; CI has no ROM and skips silently). Every load
//! re-verifies the pin table, so a wrong dump fails before any call.

use apricorn_harness::arm::retail::RetailArm9;
use apricorn_harness::pins::PinMode;
use apricorn_harness::pins::PinTable;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// A scratch address above the loaded image (image ends 0x0211_1EF8)
/// and below the stack (0x0230_0000): the CRC table and input buffer.
const SCRATCH: u32 = 0x0220_0000;

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

// ===== LCG ============================================================

#[test]
fn lcg_matches_the_c_reference() {
    let Some(mut arm9) = load() else { return };

    // u32 LCRandom(void): state = state * 1103515245 + 24691;
    // return (u16)(state / 65536).
    let mut reference = 0x1234u32;
    call(&mut arm9, "SetLCRNGSeed", &[0x1234]);
    for i in 0..64 {
        let got = call(&mut arm9, "LCRandom", &[]);
        reference = reference.wrapping_mul(1103515245).wrapping_add(24691);
        let expected = reference >> 16;
        assert_eq!(got, expected, "LCRandom draw {i}");
        // GetLCRNGSeed sees the post-draw state.
        assert_eq!(call(&mut arm9, "GetLCRNGSeed", &[]), reference);
    }

    // u32 PRandom(u32 seed): seed * 1812433253 + 1 (stateless).
    assert_eq!(call(&mut arm9, "PRandom", &[0x1234]), {
        0x1234u32.wrapping_mul(1812433253).wrapping_add(1)
    });
}

// ===== Mersenne Twister ===============================================

/// The differential reference is the engine's own
/// `apicorn_core::rng::Mt19937` — the port of pret's
/// `SetMTRNGSeed`/`MTRandom` (`src/math_util.c`) — so the retail
/// image locks the engine implementation draw by draw (no local
/// reference copy to drift from it).
use apricorn_core::rng::Mt19937;

#[test]
fn mt_matches_the_c_reference() {
    let Some(mut arm9) = load() else { return };

    // Seeded stream: SetMTRNGSeed then 1300 draws (two twists).
    let mut reference = Mt19937::new(0x1234);
    call(&mut arm9, "SetMTRNGSeed", &[0x1234]);
    for i in 0..1300 {
        let got = call(&mut arm9, "MTRandom", &[]);
        assert_eq!(got, reference.next_u32(), "MTRandom draw {i}");
    }

    // Fresh-image stream: without seeding, cycles == 625 (the image's
    // own `sMTRNG_Cycles` initializer) forces the internal
    // SetMTRNGSeed(5489) reseed. The first loop left the global
    // mid-stream, so reload the image first.
    let Some(mut arm9) = load() else { return };
    let mut reference = Mt19937::uninitialized();
    for i in 0..1300 {
        let got = call(&mut arm9, "MTRandom", &[]);
        assert_eq!(got, reference.next_u32(), "unseeded MTRandom draw {i}");
    }
}

// ===== CRC-16/CCITT ====================================================

#[test]
fn crc16_ccitt_matches_known_values() {
    let Some(mut arm9) = load() else { return };

    // MATH_CRC16InitTable is a static inline in the SDK: the compiled
    // callers reach MATHi_CRC16InitTable directly, so we do too
    // (r0 = table, r1 = poly 0x1021).
    call(&mut arm9, "MATHi_CRC16InitTable", &[SCRATCH, 0x1021]);

    // The check value for CRC-16/CCITT-FALSE ("123456789" from init
    // 0xFFFF) is 0x29B1.
    let input = b"123456789";
    for (i, &byte) in input.iter().enumerate() {
        arm9.cpu()
            .mem_mut()
            .write8(SCRATCH + 0x200 + i as u32, byte)
            .expect("scratch write");
    }
    let hash = call(
        &mut arm9,
        "MATH_CalcCRC16CCITT",
        &[SCRATCH, SCRATCH + 0x200, input.len() as u32],
    );
    assert_eq!(hash, 0x29B1, "CRC-16/CCITT of \"123456789\"");

    // And an independent reference (MSB-first, poly 0x1021, init
    // 0xFFFF) over a longer buffer, using the same table.
    let mut reference = 0xFFFFu16;
    for &byte in input.iter() {
        reference = (reference << 8) ^ table_byte(reference, byte, SCRATCH, &mut arm9);
    }
    assert_eq!(u32::from(reference), 0x29B1, "reference check");
}

/// Reads the SDK-generated table entry the update step would use.
fn table_byte(crc: u16, byte: u8, table_addr: u32, arm9: &mut RetailArm9) -> u16 {
    let idx = (((crc >> 8) as u32) ^ byte as u32) & 0xFF;
    arm9.cpu()
        .mem()
        .read16(table_addr + idx * 2)
        .expect("table entry")
}
