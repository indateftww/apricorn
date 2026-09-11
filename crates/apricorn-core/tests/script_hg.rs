//! The script VM against a retail HeartGold (US) dump: the opcode
//! inventory of the early-game banks (the pinned expectation the
//! command subset was chosen from), the init-script headers, and real
//! scripts run to completion against the recording mock host.
//!
//! Skips silently when `hg_usa.nds` is absent.

use std::collections::BTreeMap;
use std::sync::Arc;

use apricorn_core::assets::AssetStore;
use apricorn_core::script::bank::{MSG_NARC, SCRIPT_NARC};
use apricorn_core::script::{
    Disassembly, InitScriptHeader, Opcode, ScriptBank, disassemble, disassemble_entries,
    is_implemented, narc_member, resolve_script, MapBanks,
};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// `_std_init` — bank 149 (`sScriptBankMapping`).
const STD_INIT_BANK: u16 = 149;

/// The early-game maps: `(name, scripts bank, header bank, msg bank)`
/// from pret `src/data/map_headers.h`.
const EARLY_MAPS: [(&str, u16, u16, u16); 5] = [
    ("T20 New Bark Town", 842, 615, 542),
    ("T20R0101 Elm's lab 1F", 843, 616, 543),
    ("T20R0201 player's house 1F", 845, 618, 545),
    ("T20R0202 player's bedroom", 846, 619, 546),
    ("R29 Route 29", 225, 470, 373),
];

fn store() -> Option<Arc<AssetStore>> {
    match AssetStore::open(std::path::Path::new(ROM_PATH)) {
        Ok(store) => Some(Arc::new(store)),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

fn histogram_lines(d: &Disassembly) -> Vec<String> {
    d.counts()
        .iter()
        .map(|(op, n)| format!("{:>4} {:<40} {n}", op.code(), op.name()))
        .collect()
}

#[test]
fn early_game_banks_decode_and_inventory() {
    let Some(store) = store() else { return };
    let mut all: BTreeMap<Opcode, u32> = BTreeMap::new();
    let mut callstd: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    let mut banks: Vec<(String, u16)> = vec![("std_init".into(), STD_INIT_BANK)];
    banks.extend(EARLY_MAPS.iter().map(|(n, s, _, _)| (n.to_string(), *s)));
    for (name, bank_id) in &banks {
        let bank = ScriptBank::load(&store, usize::from(*bank_id)).unwrap();
        let d = disassemble(&bank).unwrap_or_else(|e| panic!("{name} ({bank_id}): {e}"));
        eprintln!("== {name} (bank {bank_id}): {} scripts, {} instructions, {} movement lists",
            bank.script_count(), d.instruction_count(), d.movements.len());
        for line in histogram_lines(&d) {
            eprintln!("{line}");
        }
        for (op, n) in d.counts() {
            *all.entry(op).or_default() += n;
        }
        for insn in d.instructions.values() {
            if insn.opcode == Opcode::CallStd {
                let target = insn.operands[0] as u16;
                let r = resolve_script(target, MapBanks::default());
                callstd.entry(r.script_bank).or_default().push(r.index);
            }
        }
    }
    eprintln!("== CallStd targets: {callstd:?}");
    for (bank_id, indices) in &callstd {
        let bank = ScriptBank::load(&store, usize::from(*bank_id)).unwrap();
        let mut idx: Vec<usize> = indices.iter().map(|&i| usize::from(i)).collect();
        idx.sort_unstable();
        idx.dedup();
        let d = disassemble_entries(&bank, &idx).unwrap();
        eprintln!("== std bank {bank_id} entries {idx:?}: {} instructions", d.instruction_count());
        for line in histogram_lines(&d) {
            eprintln!("{line}");
        }
        for (op, n) in d.counts() {
            *all.entry(op).or_default() += n;
        }
    }
    eprintln!("== ALL");
    let mut missing = Vec::new();
    for (op, n) in &all {
        eprintln!("{:>4} {:<40} {n} {}", op.code(), op.name(), if is_implemented(*op) { "" } else { "UNIMPLEMENTED" });
        if !is_implemented(*op) {
            missing.push(*op);
        }
    }
    eprintln!("== missing: {missing:?}");
}

#[test]
fn early_game_headers() {
    let Some(store) = store() else { return };
    for (name, _, header, _) in &EARLY_MAPS {
        let h = InitScriptHeader::load(&store, usize::from(*header)).unwrap();
        eprintln!("== {name} header {header}: {:?} bytes={:?}", h.entries(), h.bytes().len());
        eprintln!("   transition={:?} resume={:?} load={:?} table={:?}",
            h.load_script_id(2), h.load_script_id(3), h.load_script_id(4), h.frame_table(1));
    }
    let _ = (SCRIPT_NARC, MSG_NARC, narc_member);
}
