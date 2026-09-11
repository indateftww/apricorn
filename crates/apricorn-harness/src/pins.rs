//! Pinned ARM9 addresses — the harness's symbol table.
//!
//! The pret decompilation has no symbol files, so the addresses the
//! harness probes (differential call targets, watched regions) are
//! *pinned*: recorded once with a SHA-1 over each pin's first bytes,
//! re-verified against the loaded ARM9 image on every run. A pin that
//! stops matching means the ROM in hand is not the ROM the pins were
//! cut from — a loud [`HarnessError::Gate`], never a silently wrong
//! probe.
//!
//! The committed table lives in [`PinTable::arm9`] (`pins/arm9.tsv`),
//! with a header recording the ROM and image hashes it was cut from.
//! Pins were discovered by scanning the image for known constants (the
//! LCG multiplier 0x41C64E6D, the MT multiplier 0x6C078965, the
//! tempering and XOR-mask words, the CRC polynomial 0x1021 — see
//! [`scan_constant`]) and decoding the surrounding Thumb code, plus
//! the SDK `MATH_*` functions whose real addresses are carried in
//! pret's labeled `lib/asm/nitro.s`.

use sha1::{Digest, Sha1};

use crate::HarnessError;

/// The HeartGold (US) retail ROM the committed pin table was cut from.
pub const PINNED_ROM_SHA1: &str = "4fcded0e2713dc03929845de631d0932ea2b5a37";

/// The decompressed ARM9 image that table was cut from.
pub const PINNED_IMAGE_SHA1: &str = "ad8d4da6bb5c010844dae8260212c86beef565fc";

/// How the code at a pinned address executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMode {
    /// ARM (A32) code — the SDK `MATH_*` functions.
    Arm,
    /// Thumb (T32) code — the game's own `math_util` functions.
    Thumb,
    /// Data (globals, literal pools) — never executed.
    Data,
}

/// A pinned address: a name, where it lives, and a hash proving it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    /// Symbolic name, unique in the table.
    pub name: String,
    /// How the code at the address executes.
    pub mode: PinMode,
    /// RAM address in the loaded ARM9 image.
    pub address: u32,
    /// Size in bytes (function bodies; full globals).
    pub size: u32,
    /// SHA-1 over the first `min(16, size)` bytes at the address
    /// (16 always, for code pins). Hex, 40 lowercase digits.
    pub prologue_sha1: String,
    /// How the pin was discovered (provenance, for humans).
    pub discovered_by: String,
}

impl Pin {
    /// SHA-1 over the first bytes of `bytes` per the pin's kind:
    /// 16 bytes for code, `min(16, size)` for data.
    fn hash(&self, bytes: &[u8]) -> String {
        let n = match self.mode {
            PinMode::Arm | PinMode::Thumb => 16,
            PinMode::Data => self.size.min(16) as usize,
        };
        hash_hex(&bytes[..n.min(bytes.len())])
    }

    /// The hash of an all-zero prologue for this pin's size — the
    /// expected `prologue_sha1` of a `.bss` pin (the loader zeroes it).
    fn zero_hash(&self) -> String {
        let n = match self.mode {
            PinMode::Arm | PinMode::Thumb => 16,
            PinMode::Data => self.size.min(16) as usize,
        };
        hash_hex(&vec![0u8; n])
    }
}

/// SHA-1 of `bytes`, lowercase hex.
fn hash_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

/// A parsed pin table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinTable {
    pins: Vec<Pin>,
}

impl PinTable {
    /// The committed HeartGold (US) ARM9 pin table (`pins/arm9.tsv`).
    ///
    /// Panics never happen in practice: the table is committed
    /// alongside this code and covered by tests that keep it parseable.
    #[must_use]
    pub fn arm9() -> Self {
        Self::parse(include_str!("../pins/arm9.tsv")).expect("committed pin table must parse")
    }

    /// Parses a pin table (`name  mode  0xaddress  size  sha1  provenance`,
    /// tab-separated; `#`-comment lines and blank lines ignored).
    ///
    /// # Errors
    /// Returns a [`HarnessError::Syntax`] on any malformed line or
    /// duplicate name.
    pub fn parse(text: &str) -> Result<Self, HarnessError> {
        let mut pins = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or_default().trim_end();
            if line.trim().is_empty() {
                continue;
            }
            let err = |what: &str| HarnessError::Syntax {
                line: i + 1,
                what: what.to_string(),
            };
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 6 {
                return Err(err("pin row must have 6 tab-separated fields"));
            }
            let name = fields[0].trim().to_string();
            if name.is_empty() {
                return Err(err("pin name is empty"));
            }
            if pins.iter().any(|p: &Pin| p.name == name) {
                return Err(err(&format!("duplicate pin name {name}")));
            }
            let mode = match fields[1].trim() {
                "arm" => PinMode::Arm,
                "thumb" => PinMode::Thumb,
                "data" => PinMode::Data,
                other => {
                    return Err(err(&format!("unknown pin mode {other:?} (arm|thumb|data)")));
                }
            };
            let address = parse_hex32(fields[2].trim())
                .ok_or_else(|| err("address must be 0x-prefixed hex"))?;
            let size: u32 = fields[3]
                .trim()
                .parse()
                .map_err(|_| err("size must be decimal"))?;
            let prologue_sha1 = fields[4].trim().to_ascii_lowercase();
            if prologue_sha1.len() != 40 || !prologue_sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(err("prologue-sha1 must be 40 hex digits"));
            }
            pins.push(Pin {
                name,
                mode,
                address,
                size,
                prologue_sha1,
                discovered_by: fields[5].trim().to_string(),
            });
        }
        if pins.is_empty() {
            return Err(HarnessError::Syntax {
                line: 0,
                what: "pin table has no pins".to_string(),
            });
        }
        Ok(Self { pins })
    }

    /// The pins, in table order.
    #[must_use]
    pub fn pins(&self) -> &[Pin] {
        &self.pins
    }

    /// The pin named `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Pin> {
        self.pins.iter().find(|p| p.name == name)
    }
}

/// Verifies every pin against the ARM9 image loaded at `base`.
///
/// A code pin or in-image data pin must match its recorded prologue
/// hash; a data pin whose address lies at or past the image end must be
/// `.bss` (its recorded hash must be the all-zero prologue's), which
/// the loader zeroes — anything else is drift.
///
/// # Errors
/// Returns [`HarnessError::Gate`] naming the first pin that fails.
pub fn verify_image(table: &PinTable, image: &[u8], base: u32) -> Result<(), HarnessError> {
    let end = base
        .checked_add(image.len() as u32)
        .expect("image end overflows u32");
    for pin in table.pins() {
        let gate = |what: String| HarnessError::Gate { what };
        if pin.address < base {
            return Err(gate(format!(
                "pin {} at {:#010x} is below the image base {:#010x}",
                pin.name, pin.address, base
            )));
        }
        if pin.address >= end {
            // Past the image: only .bss data pins may live there, and
            // only with the zero prologue recorded.
            if pin.mode != PinMode::Data {
                return Err(gate(format!(
                    "code pin {} at {:#010x} lies past the image end {:#010x}",
                    pin.name, pin.address, end
                )));
            }
            if pin.prologue_sha1 != pin.zero_hash() {
                return Err(gate(format!(
                    "pin {} at {:#010x} is past the image end {:#010x} but is not marked .bss",
                    pin.name, pin.address, end
                )));
            }
            continue;
        }
        let offset = (pin.address - base) as usize;
        let available = image.len().saturating_sub(offset);
        if (pin.size as usize) > available {
            return Err(gate(format!(
                "pin {} at {:#010x} straddles the image end {:#010x}",
                pin.name, pin.address, end
            )));
        }
        let bytes = &image[offset..];
        let prologue_len = match pin.mode {
            PinMode::Arm | PinMode::Thumb => 16,
            PinMode::Data => pin.size.min(16) as usize,
        };
        if bytes.len() < prologue_len {
            return Err(gate(format!(
                "pin {} at {:#010x} has no room for its prologue hash",
                pin.name, pin.address
            )));
        }
        let found = pin.hash(bytes);
        if found != pin.prologue_sha1 {
            return Err(gate(format!(
                "pin {} at {:#010x} drifted: prologue sha1 {} != recorded {}",
                pin.name, pin.address, found, pin.prologue_sha1
            )));
        }
    }
    Ok(())
}

/// Every 4-aligned address in the image (loaded at `base`) whose
/// little-endian `u32` equals `constant` — the pin-discovery helper.
///
/// Literal pools are word-aligned, so only 4-aligned hits are reported;
/// the surrounding code (the loads referencing each hit backwards from
/// the PC) is then decoded by hand to find function entries.
#[must_use]
pub fn scan_constant(image: &[u8], base: u32, constant: u32) -> Vec<u32> {
    let needle = constant.to_le_bytes();
    image
        .windows(4)
        .enumerate()
        .filter(|(i, w)| i % 4 == 0 && *w == needle)
        .map(|(i, _)| base + i as u32)
        .collect()
}

/// Parses a `0x`-prefixed hex u32 (lower- or uppercase digits).
fn parse_hex32(text: &str) -> Option<u32> {
    let digits = text.strip_prefix("0x")?;
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny two-pin table in the committed format.
    const SAMPLE: &str = "\
# comment line
# name\tmode\taddress\tsize\tprologue-sha1\tdiscovered-by
LCRandom\tthumb\t0x0201FD44\t24\tb28876d83b9d9132e7e9bd24cbc86d09b86662f9\tpool scan
sLCRNG_State\tdata\t0x021D15A8\t4\t9069ca78e7450a285173431b3e52c5c25299e473\t.bss
";

    #[test]
    fn parses_sample_table() {
        let table = PinTable::parse(SAMPLE).expect("sample must parse");
        assert_eq!(table.pins().len(), 2);
        let lc = table.get("LCRandom").expect("named pin");
        assert_eq!(lc.mode, PinMode::Thumb);
        assert_eq!(lc.address, 0x0201_FD44);
        assert_eq!(lc.size, 24);
        assert_eq!(lc.prologue_sha1, "b28876d83b9d9132e7e9bd24cbc86d09b86662f9");
        let state = table.get("sLCRNG_State").expect("named pin");
        assert_eq!(state.mode, PinMode::Data);
        assert_eq!(state.address, 0x021D_15A8);
    }

    #[test]
    fn rejects_malformed_rows() {
        // Not enough fields (space-separated, not tab).
        assert!(PinTable::parse("LCRandom thumb 0x0201FD44 24 hash here\n").is_err());
        // Unknown mode.
        assert!(PinTable::parse("a\tarmx\t0x02000000\t4\t0\tx\n").is_err());
        // Address without the 0x prefix.
        assert!(PinTable::parse("a\tarm\t201FD44\t4\t0\tx\n").is_err());
        // Prologue hash that is not 40 hex digits.
        assert!(PinTable::parse("a\tdata\t0x02000000\t4\tzz\tx\n").is_err());
        // Duplicate names.
        let dup = "a\td\t0x02000000\t4\t0\tx\na\td\t0x02000004\t4\t0\tx\n";
        assert!(PinTable::parse(dup).is_err());
        // Nothing but comments.
        assert!(PinTable::parse("# only a comment\n").is_err());
    }

    #[test]
    fn committed_arm9_table_parses() {
        let table = PinTable::arm9();
        // 12 math_util/SDK code pins + 5 math data pins, the save
        // chunk-table pins (45 code + 2 data) of Phase 4 step 4, the
        // field data table pin (sMapHeaders), the field-movement
        // pins (25 code + gMovementCmdTable) and the day/night pins
        // (9 code + 3 data) of Phase 5.
        assert_eq!(
            table.pins().len(),
            103,
            "17 math + 47 save + 1 field + 26 movement + 12 day/night pins"
        );
        // The Phase 2 differential targets are all pinned.
        for name in [
            "SetLCRNGSeed",
            "LCRandom",
            "GetLCRNGSeed",
            "SetMTRNGSeed",
            "MTRandom",
            "MATHi_CRC16InitTable",
            "MATHi_CRC16Update",
            "MATH_CalcCRC16CCITT",
            "sLCRNG_State",
            "sCRC16TablePtr",
        ] {
            assert!(table.get(name).is_some(), "missing pin {name}");
        }
        // Code pins hash 16 bytes even when the function is shorter.
        let seed = table.get("GetLCRNGSeed").expect("pin");
        assert_eq!(seed.size, 12);
        assert_eq!(seed.hash(&[0u8; 24]), hash_hex(&[0u8; 16]));
    }

    #[test]
    fn verify_image_accepts_matching_image_and_bss() {
        // LCRandom's real 16-byte prologue at the image base, plus a
        // bss pin far past the end recorded as zeros — both accepted.
        let table = PinTable::parse(
            "LCRandom\tthumb\t0x02000000\t24\tb28876d83b9d9132e7e9bd24cbc86d09b86662f9\tpool scan\n\
             sLCRNG_State\tdata\t0x021D15A8\t4\t9069ca78e7450a285173431b3e52c5c25299e473\t.bss\n",
        )
        .expect("sample must parse");
        // LCRandom's real 24 bytes.
        let image: Vec<u8> = [
            0x05, 0x49, 0x06, 0x48, 0x4a, 0x68, 0x13, 0x1c, 0x43, 0x43, 0x05, 0x48, 0x18, 0x18,
            0x48, 0x60, 0x00, 0x0c, 0x00, 0x04, 0x00, 0x0c, 0x70, 0x47,
        ]
        .to_vec();
        assert!(verify_image(&table, &image, 0x0200_0000).is_ok());

        // One drifted byte → the pin fails loudly.
        let mut drifted = image.clone();
        drifted[2] ^= 0xFF;
        let err = verify_image(&table, &drifted, 0x0200_0000).expect_err("must fail");
        assert!(err.to_string().contains("LCRandom"));
        assert!(err.to_string().contains("drifted"));
    }

    #[test]
    fn verify_image_rejects_impossible_pins() {
        // A non-bss data pin recorded with nonzero content, placed past
        // the image end.
        let table = PinTable::parse(
            "s\tdata\t0x02000010\t4\t6f2a6f2a6f2a6f2a6f2a6f2a6f2a6f2a6f2a6f2a\treal data\n",
        )
        .expect("parse");
        let err = verify_image(&table, &[0u8; 8], 0x0200_0000).expect_err("must fail");
        assert!(err.to_string().contains("not marked .bss"));

        // A code pin straddling the image end.
        let table = PinTable::parse(
            "f\tarm\t0x02000004\t16\t6f2a6f2a6f2a6f2a6f2a6f2a6f2a6f2a6f2a6f2a\tx\n",
        )
        .expect("parse");
        let err = verify_image(&table, &[0u8; 8], 0x0200_0000).expect_err("must fail");
        assert!(err.to_string().contains("straddles"));
    }

    #[test]
    fn scan_finds_aligned_constants_only() {
        let image: Vec<u8> = [
            0x6D, 0x4E, 0xC6, 0x41, // aligned hit at +0
            0xAA, 0xBB, 0xCC, 0xDD, // not the constant
            0x6D, 0x4E, 0xC6, 0x41, // aligned hit at +8
        ]
        .into_iter()
        .chain([
            0x6D, 0x4E, 0xC6, 0x41, // aligned hit at +12
        ])
        .collect();
        // Also an unaligned copy, which must be ignored.
        let mut image = image;
        image.extend_from_slice(&[0x00, 0x6D, 0x4E, 0xC6, 0x41, 0x00]);
        let hits = scan_constant(&image, 0x0200_0000, 0x41C6_4E6D);
        assert_eq!(hits, vec![0x0200_0000, 0x0200_0008, 0x0200_000C]);
    }
}
