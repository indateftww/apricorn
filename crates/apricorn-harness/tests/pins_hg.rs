//! Integration tests for the committed ARM9 pin table against a real
//! retail HeartGold (US) dump.
//!
//! These run only when `hg_usa.nds` sits at the repo root (each
//! developer supplies their own ROM; CI has none, so the tests skip
//! silently). The pinned values were cut from exactly that dump
//! (SHA-1 4fcded0e…), so on any other ROM they are expected to fail
//! loudly — that is the design.

use apricorn_core::nds::NdsRom;
use apricorn_harness::pins::{self, PinMode, PinTable};
use sha1::Digest;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

fn load_rom() -> Option<Vec<u8>> {
    match std::fs::read(ROM_PATH) {
        Ok(data) => Some(data),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

#[test]
fn pins_verify_against_retail_arm9_image() {
    let Some(data) = load_rom() else { return };
    let rom = NdsRom::parse(&data).expect("retail ROM must parse");

    // The ARM9 binary is BLZ "compressed static": the header's size is
    // the stored size, and the loaded image is the decompressed form.
    let image = rom.arm9_image().expect("retail ARM9 must decompress");
    assert_eq!(image.len(), 0x0011_1EF8, "decompressed arm9 size");
    let image_hash = format!("{:x}", sha1::Sha1::digest(image.as_ref()));
    assert_eq!(
        image_hash,
        pins::PINNED_IMAGE_SHA1,
        "decompressed arm9 image hash"
    );

    // Every pin must match the decompressed image (or be a marked
    // .bss pin past its end).
    let table = PinTable::arm9();
    pins::verify_image(&table, &image, rom.header.arm9.ram_address)
        .expect("committed pins must match the retail image");

    // The pins are exactly the committed differential surface: the 17
    // math pins of Phase 2/3 (12 code + 5 data), the 47 save pins of
    // Phase 4 step 4 (45 size stubs + 2 chunk-table globals), the
    // map-header table pin, the 26 field-movement pins (25 code +
    // gMovementCmdTable) and the 12 day/night pins (9 code + 3 data)
    // of Phase 5.
    assert_eq!(table.pins().len(), 103);
    assert_eq!(
        table
            .pins()
            .iter()
            .filter(|p| p.mode == PinMode::Arm || p.mode == PinMode::Thumb)
            .count(),
        91,
        "91 code pins"
    );

    // The discovery constants really are where the table's provenance
    // column says they are.
    let base = rom.header.arm9.ram_address;
    for (constant, address) in [
        (0x41C6_4E6D, 0x0201_FD60), // LCG multiplier
        (0x0000_6073, 0x0201_FD64), // LCG increment
        (0x6C07_8965, 0x0201_FDB4), // MT init multiplier
        (0x9D2C_5680, 0x0201_FEC4), // MT tempering mask 1
        (0xEFC6_0000, 0x0201_FEC8), // MT tempering mask 2
        (0x41C6_4E6D, 0x0201_FF90), // MonEncryptionLCRNG's pool
        (0x0000_1021, 0x0201_FFDC), // CRC16-CCITT polynomial
        (0x9908_B0DF, 0x0210_F6D4), // MT XOR mask (sMTRNG_XOR[1])
        (0x0000_FFFF, 0x020E_3A54), // MATH_CalcCRC16CCITT's init-value pool
    ] {
        let hits = pins::scan_constant(&image, base, constant);
        assert!(
            hits.contains(&address),
            "constant {constant:#010x} not found at {address:#010x} (hits: {hits:#010x?})"
        );
    }
}
