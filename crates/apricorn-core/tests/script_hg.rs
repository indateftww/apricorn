//! The script VM against a retail HeartGold (US) dump.
//!
//! Three things are pinned here. The *opcode inventory* of the banks
//! the early game runs — `_std_init` (149), New Bark Town (842), Elm's
//! lab (843), the player's house (845) and bedroom (846), Route 29
//! (225), plus the eight `std_misc` (3) scripts they `CallStd` — is the
//! expectation the command subset in `script::exec` was chosen from:
//! every bank must decode by recursive descent with no unknown opcode,
//! and every opcode it uses must have a handler. The *init-script
//! headers* of those maps must read as pret's `scr_seq_*_hdr.s`
//! describe them. And real scripts must *run to completion* against the
//! recording mock host: bank 149's new-game init script, the New Bark
//! Town and Route 29 ON_TRANSITION scripts (synchronously, as
//! `StartMapLoadScript`), the bedroom's PC script and the player's-house
//! Mom scene (frame by frame, as `Task_RunScripts`, the latter through
//! two `CallStd`s).
//!
//! The bedroom's header (619) is a lone terminator: it has no
//! ON_TRANSITION, ON_RESUME or frame table, so the room's only scripts
//! are the PC (script 1) and a signpost-like bookshelf (script 2).
//!
//! Only structural constants are pinned — opcode counts, script ids,
//! variable and flag ids, tile coordinates — never text or bytes.
//! Skips silently when `hg_usa.nds` is absent.

use std::collections::BTreeMap;
use std::sync::Arc;

use apricorn_core::assets::AssetStore;
use apricorn_core::input::{Keys, key};
use apricorn_core::rng::Lcrng;
use apricorn_core::script::bank::std_script;
use apricorn_core::script::header::{ON_FRAME_TABLE, ON_RESUME, ON_TRANSITION};
use apricorn_core::script::host::{AppRequest, FieldAction, FieldQuery, HostEvent, ScriptHost, dir};
use apricorn_core::script::{
    Disassembly, FrameStatus, InitScriptHeader, MapBanks, Opcode, RecordingHost, ScriptBank,
    ScriptEnvironment, disassemble, disassemble_entries, is_implemented, narc_member,
    resolve_script,
};
use apricorn_core::text::string::GameString;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// `_std_init` — bank 149 (`sScriptBankMapping`).
const STD_INIT_BANK: u16 = 149;
/// `_std_misc` — bank 3.
const STD_MISC_BANK: u16 = 3;

/// The early-game maps: `(name, scripts bank, header bank, msg bank)`
/// from pret `src/data/map_headers.h`.
const EARLY_MAPS: [(&str, u16, u16, u16); 5] = [
    ("T20 New Bark Town", 842, 615, 542),
    ("T20R0101 Elm's lab 1F", 843, 616, 543),
    ("T20R0201 player's house 1F", 845, 618, 545),
    ("T20R0202 player's bedroom", 846, 619, 546),
    ("R29 Route 29", 225, 470, 373),
];

/// One bank's pinned inventory: scripts, reachable instructions,
/// movement lists, and `(opcode, count)` ascending.
struct Inventory {
    bank: u16,
    scripts: usize,
    instructions: usize,
    movements: usize,
    counts: &'static [(u16, u32)],
}

const INVENTORIES: [Inventory; 6] = [
    Inventory {
        bank: STD_INIT_BANK,
        scripts: 1,
        instructions: 145,
        movements: 0,
        counts: &[(2, 1), (30, 143), (505, 1)],
    },
    Inventory {
        bank: 842,
        scripts: 18,
        instructions: 748,
        movements: 127,
        counts: &[
            (2, 29), (3, 1), (17, 82), (20, 8), (22, 64), (28, 85), (30, 7), (31, 7), (32, 3),
            (39, 1), (41, 5), (45, 20), (50, 9), (53, 24), (55, 1), (56, 3), (57, 4), (58, 4),
            (59, 3), (60, 1), (73, 10), (75, 4), (76, 1), (77, 1), (78, 1), (79, 1), (94, 150),
            (95, 44), (96, 12), (97, 19), (99, 1), (100, 5), (101, 6), (104, 5), (105, 6),
            (132, 7), (144, 1), (146, 1), (174, 4), (175, 4), (176, 1), (190, 9), (192, 1),
            (294, 1), (307, 4), (308, 8), (309, 4), (310, 4), (311, 4), (339, 3), (386, 2),
            (438, 5), (440, 5), (484, 1), (582, 1), (596, 1), (600, 1), (602, 12), (603, 12),
            (604, 12), (609, 5), (615, 1), (618, 1), (729, 2), (746, 1), (747, 1), (748, 1),
            (845, 1),
        ],
    },
    Inventory {
        bank: 843,
        scripts: 16,
        instructions: 783,
        movements: 64,
        counts: &[
            (2, 35), (3, 2), (17, 53), (20, 9), (22, 34), (26, 2), (27, 2), (28, 67), (29, 1),
            (30, 12), (31, 7), (32, 15), (39, 2), (41, 31), (45, 52), (50, 29), (53, 67),
            (73, 14), (75, 4), (78, 3), (79, 3), (94, 76), (95, 55), (96, 14), (97, 33),
            (100, 1), (101, 3), (104, 4), (105, 2), (106, 2), (126, 1), (127, 3), (131, 1),
            (132, 30), (143, 1), (144, 1), (149, 1), (150, 1), (167, 1), (173, 1), (174, 6),
            (175, 6), (190, 28), (191, 1), (193, 2), (199, 1), (282, 1), (294, 2), (332, 1),
            (339, 5), (354, 3), (382, 1), (386, 1), (436, 1), (495, 1), (529, 2), (602, 10),
            (603, 8), (604, 8), (605, 1), (608, 1), (609, 5), (621, 1), (746, 4), (747, 3),
            (748, 4), (827, 1),
        ],
    },
    Inventory {
        bank: 845,
        scripts: 7,
        instructions: 290,
        movements: 4,
        counts: &[
            (2, 22), (3, 5), (17, 17), (20, 4), (22, 6), (28, 21), (30, 8), (31, 2), (32, 4),
            (41, 3), (42, 3), (45, 35), (50, 17), (51, 1), (53, 22), (73, 10), (78, 5), (79, 5),
            (94, 8), (95, 6), (96, 10), (97, 22), (104, 4), (128, 1), (132, 1), (190, 7),
            (294, 1), (368, 1), (609, 2), (746, 4), (747, 10), (748, 3), (750, 1), (751, 4),
            (752, 1), (793, 2), (794, 1), (795, 1), (796, 8), (838, 2),
        ],
    },
    Inventory {
        bank: 846,
        scripts: 2,
        instructions: 29,
        movements: 0,
        counts: &[
            (2, 3), (17, 1), (28, 1), (45, 3), (50, 2), (53, 3), (73, 2), (96, 2), (97, 3),
            (150, 1), (174, 2), (175, 2), (190, 1), (376, 1), (377, 1), (609, 1),
        ],
    },
    Inventory {
        bank: 225,
        scripts: 9,
        instructions: 320,
        movements: 56,
        counts: &[
            (2, 15), (3, 1), (17, 32), (20, 6), (22, 25), (27, 1), (28, 33), (29, 1), (30, 7),
            (31, 1), (32, 2), (39, 1), (41, 5), (45, 8), (50, 9), (53, 14), (55, 2), (57, 2),
            (58, 2), (60, 2), (73, 5), (76, 1), (77, 1), (78, 1), (79, 1), (94, 56), (95, 10),
            (96, 6), (97, 11), (98, 1), (99, 1), (101, 2), (104, 5), (105, 2), (127, 1),
            (132, 4), (144, 1), (190, 1), (193, 1), (251, 1), (281, 1), (294, 1), (379, 1),
            (438, 8), (440, 9), (480, 1), (481, 1), (484, 3), (529, 1), (602, 4), (603, 4),
            (604, 4), (609, 1),
        ],
    },
];

/// The `std_misc` entries the six banks `CallStd` (std_signpost,
/// std_obtain_item_verbose, std_bag_is_full, std_play_friend_music,
/// std_fade_end_friend_music, std_give_item_verbose, std_play_mom_music,
/// std_fade_end_mom_music) and their combined inventory.
const STD_MISC_ENTRIES: &[usize] = &[0, 8, 9, 29, 30, 33, 36, 38];
const STD_MISC_INSTRUCTIONS: usize = 165;
const STD_MISC_COUNTS: &[(u16, u32)] = &[
    (2, 11), (17, 28), (21, 9), (22, 13), (26, 5), (27, 10), (28, 22), (29, 6), (42, 4),
    (45, 6), (50, 2), (57, 3), (58, 1), (60, 1), (61, 1), (78, 4), (79, 1), (81, 3), (82, 1),
    (84, 1), (87, 3), (125, 2), (130, 12), (190, 2), (194, 3), (195, 8), (281, 1), (844, 2),
];

// Variable and flag ids the runs below assert on (pret
// include/constants/vars.h, flags.h).
const VAR_OBJ_0: u16 = 0x4020;
const VAR_OBJ_1: u16 = 0x4021;
const VAR_TEMP_X4007: u16 = 0x4007;
const VAR_SCENE_NEW_BARK_TOWN_OW: u16 = 0x4072;
const VAR_SCENE_PLAYERS_HOUSE_1F: u16 = 0x4106;
const VAR_SCENE_ELMS_LAB: u16 = 0x4108;
const VAR_UNK_40FC: u16 = 0x40FC;
const VAR_LOTO_NUMBER_LO: u16 = 0x403C;
const FLAG_UNK_189: u16 = 0x189;
const FLAG_HIDE_ELMS_LAB_OFFICER: u16 = 0x19D;
const FLAG_HIDE_NEW_BARK_FRIEND: u16 = 0x1A2;
const FLAG_UNK_207: u16 = 0x207;
const FLAG_HIDE_CAMERON: u16 = 0x27E;
const FLAG_HIDE_ROUTE_12_SNORLAX: u16 = 0x31B;
const FLAG_GOT_BAG: u16 = 0x11B;
const FLAG_GOT_TRAINER_CARD: u16 = 0x11C;
const FLAG_GOT_SAVE_BUTTON: u16 = 0x11D;
const FLAG_GOT_OPTIONS_BUTTON: u16 = 0x11E;
/// `SPRITE_HEROINE` — what `GetFriendSprite` gives a male player.
const SPRITE_HEROINE: u16 = 97;

fn store() -> Option<Arc<AssetStore>> {
    match AssetStore::open(std::path::Path::new(ROM_PATH)) {
        Ok(store) => Some(Arc::new(store)),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

/// A recording host whose banks come from the ROM.
fn rom_host(store: &Arc<AssetStore>, map: MapBanks) -> RecordingHost {
    let store = Arc::clone(store);
    let mut host = RecordingHost::new();
    host.loader = Some(Box::new(move |narc: &str, member: u16| {
        narc_member(&store, narc, member).ok()
    }));
    host.map_banks = map;
    host.player_name = GameString::from_units(&[305, 313, 310, 302]); // "GOLD"
    host
}

fn map_banks(index: usize) -> MapBanks {
    MapBanks {
        scripts: EARLY_MAPS[index].1,
        messages: EARLY_MAPS[index].3,
    }
}

fn header(store: &AssetStore, index: usize) -> InitScriptHeader {
    InitScriptHeader::load(store, usize::from(EARLY_MAPS[index].2)).unwrap()
}

fn assert_inventory(name: &str, d: &Disassembly, expected: &[(u16, u32)]) {
    let got: Vec<(u16, u32)> = d.counts().iter().map(|(op, n)| (op.code(), *n)).collect();
    assert_eq!(got, expected, "{name}: opcode histogram");
    for op in d.opcodes_used() {
        assert!(is_implemented(op), "{name} uses {} which has no handler", op.name());
    }
}

#[test]
fn early_game_banks_decode_clean_and_match_the_pinned_inventory() {
    let Some(store) = store() else { return };
    let mut callstd: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
    for inv in &INVENTORIES {
        let bank = ScriptBank::load(&store, usize::from(inv.bank)).unwrap();
        assert_eq!(bank.script_count(), inv.scripts, "bank {}", inv.bank);
        let d = disassemble(&bank).unwrap_or_else(|e| panic!("bank {}: {e}", inv.bank));
        assert_eq!(d.instruction_count(), inv.instructions, "bank {}", inv.bank);
        assert_eq!(d.movements.len(), inv.movements, "bank {}", inv.bank);
        assert_inventory(&format!("bank {}", inv.bank), &d, inv.counts);
        // Every movement list ends properly and every branch lands on a
        // decoded instruction.
        for list in d.movements.values() {
            assert_eq!(list.last().map(|s| s.command), Some(254));
        }
        for insn in d.instructions.values() {
            if let Some(target) = insn.target().filter(|_| !insn.opcode.targets_movement()) {
                assert!(d.instructions.contains_key(&target), "branch target {target:#x}");
            }
            if insn.opcode == Opcode::CallStd {
                let r = resolve_script(insn.operands[0] as u16, MapBanks::default());
                callstd.entry(r.script_bank).or_default().push(usize::from(r.index));
            }
        }
    }
    // The only std bank called is std_misc, at exactly these entries.
    let mut entries = callstd.remove(&STD_MISC_BANK).unwrap();
    assert!(callstd.is_empty(), "unexpected std banks: {callstd:?}");
    entries.sort_unstable();
    entries.dedup();
    assert_eq!(entries, STD_MISC_ENTRIES);
    let std = ScriptBank::load(&store, usize::from(STD_MISC_BANK)).unwrap();
    let d = disassemble_entries(&std, STD_MISC_ENTRIES).unwrap();
    assert_eq!(d.instruction_count(), STD_MISC_INSTRUCTIONS);
    assert_inventory("std_misc subset", &d, STD_MISC_COUNTS);
}

#[test]
fn early_game_headers_read_like_pret() {
    let Some(store) = store() else { return };
    // New Bark Town: a frame table, ON_TRANSITION 7, ON_RESUME 10.
    let h = header(&store, 0);
    assert_eq!(h.entries().len(), 3);
    assert_eq!(h.load_script_id(ON_TRANSITION), Some(7));
    assert_eq!(h.load_script_id(ON_RESUME), Some(10));
    let rows: Vec<(u16, u16, u16)> = h
        .frame_table(ON_FRAME_TABLE)
        .unwrap()
        .iter()
        .map(|r| (r.var1, r.var2, r.script))
        .collect();
    assert_eq!(
        rows,
        vec![
            (VAR_SCENE_PLAYERS_HOUSE_1F, 1, 4),
            (VAR_SCENE_NEW_BARK_TOWN_OW, 1, 9)
        ]
    );
    // Elm's lab: ON_RESUME 11 first, then the table.
    let h = header(&store, 1);
    assert_eq!(h.load_script_id(ON_RESUME), Some(11));
    assert_eq!(h.load_script_id(ON_TRANSITION), None);
    let rows: Vec<(u16, u16, u16)> = h
        .frame_table(ON_FRAME_TABLE)
        .unwrap()
        .iter()
        .map(|r| (r.var1, r.var2, r.script))
        .collect();
    assert_eq!(
        rows,
        vec![
            (VAR_UNK_40FC, 2, 16),
            (VAR_SCENE_ELMS_LAB, 3, 3),
            (VAR_SCENE_ELMS_LAB, 8, 15)
        ]
    );
    // The player's house: only a frame table.
    let h = header(&store, 2);
    assert_eq!(h.entries().len(), 1);
    let rows: Vec<(u16, u16, u16)> = h
        .frame_table(ON_FRAME_TABLE)
        .unwrap()
        .iter()
        .map(|r| (r.var1, r.var2, r.script))
        .collect();
    assert_eq!(
        rows,
        vec![
            (VAR_SCENE_PLAYERS_HOUSE_1F, 3, 7),
            (VAR_SCENE_PLAYERS_HOUSE_1F, 0, 1)
        ]
    );
    // The bedroom: nothing at all.
    let h = header(&store, 3);
    assert!(h.entries().is_empty());
    assert_eq!(h.bytes().len(), 4);
    assert_eq!(h.load_script_id(ON_TRANSITION), None);
    assert_eq!(h.frame_table(ON_FRAME_TABLE), None);
    // Route 29: ON_TRANSITION 1 only.
    let h = header(&store, 4);
    assert_eq!(h.entries().len(), 1);
    assert_eq!(h.load_script_id(ON_TRANSITION), Some(1));

    // Scene dispatch on a fresh save: the player's house runs its Mom
    // scene (var == 0 → script 1); New Bark Town nothing; after the
    // scene has advanced the variable, New Bark runs script 4.
    let mut host = rom_host(&store, map_banks(2));
    let env = ScriptEnvironment::new();
    let scene = |host: &mut RecordingHost, env: &ScriptEnvironment, h: &InitScriptHeader| {
        h.scene_script_id(ON_FRAME_TABLE, |v| env.var_get(host, v).unwrap())
    };
    assert_eq!(scene(&mut host, &env, &header(&store, 2)), Some(1));
    assert_eq!(scene(&mut host, &env, &header(&store, 0)), None);
    host.vars_flags().set_var(VAR_SCENE_PLAYERS_HOUSE_1F, 1);
    assert_eq!(scene(&mut host, &env, &header(&store, 0)), Some(4));
    assert_eq!(scene(&mut host, &env, &header(&store, 2)), None);
}

#[test]
fn std_init_runs_to_completion_and_seeds_the_new_game_flags() {
    let Some(store) = store() else { return };
    let mut host = rom_host(&store, map_banks(3));
    host.rng = Lcrng::new(0x1234_5678);
    let mut expected = Lcrng::new(0x1234_5678);
    let mut env = ScriptEnvironment::new();
    // One RunScriptCommand call: nothing in the script yields.
    assert_eq!(env.run_map_load_script(&mut host, std_script::INIT, 1000), Ok(1));
    assert_eq!(env.active_count(), 0);
    assert!(host.events_without_polls().is_empty(), "no field side effects");
    // 143 SetFlags, all hide-object story flags: the first executed
    // hides Elm's lab officer, the last Route 12's Snorlax, and every
    // one lies in the 0x194.. hide-flag range.
    let set: Vec<u16> = (1..2912).filter(|&f| host.vars_flags().flag(f)).collect();
    assert_eq!(set.len(), 143);
    assert!(set.contains(&FLAG_HIDE_ELMS_LAB_OFFICER));
    assert!(set.contains(&FLAG_HIDE_ROUTE_12_SNORLAX));
    assert_eq!(set.first(), Some(&0x194));
    assert!(set.iter().all(|&f| (0x194..0x400).contains(&f)), "{set:#x?}");
    assert!(host.temp_flags.bytes().iter().all(|&b| b == 0));
    // LotoIDSet: two LCRandom draws, the second landing in LO.
    let _lo = expected.next_u16();
    let hi = expected.next_u16();
    assert_eq!(host.vars_flags().var(VAR_LOTO_NUMBER_LO), Some(hi));
    assert_eq!(host.rng, expected);
    // The only variable written.
    let vars: Vec<u16> = (0x4000..=0x416F)
        .filter(|&v| host.vars_flags().var(v) != Some(0))
        .collect();
    assert_eq!(vars, vec![VAR_LOTO_NUMBER_LO]);
}

#[test]
fn new_bark_transition_and_resume_run_as_map_load_scripts() {
    let Some(store) = store() else { return };
    // ON_TRANSITION (script 7): GetFriendSprite → VAR_OBJ_0; FLAG_UNK_189
    // is clear on a new game, so the Cameron check runs: no Plain Badge
    // → hide Cameron.
    let mut host = rom_host(&store, map_banks(0));
    let mut env = ScriptEnvironment::new();
    assert_eq!(env.run_map_load_script(&mut host, 7, 100), Ok(2), "GetFriendSprite yields once");
    assert_eq!(host.vars_flags().var(VAR_OBJ_0), Some(SPRITE_HEROINE));
    assert_eq!(host.vars_flags().var(VAR_TEMP_X4007), Some(0));
    assert!(host.vars_flags().flag(FLAG_HIDE_CAMERON));
    assert!(host.events_without_polls().is_empty());
    // With FLAG_UNK_189 set the script just clears it.
    let mut host = rom_host(&store, map_banks(0));
    host.vars_flags().set_flag(FLAG_UNK_189);
    let mut env = ScriptEnvironment::new();
    assert_eq!(env.run_map_load_script(&mut host, 7, 100), Ok(2));
    assert!(!host.vars_flags().flag(FLAG_UNK_189));
    assert!(!host.vars_flags().flag(FLAG_HIDE_CAMERON));
    // A female player gets the hero's sprite (0).
    let mut host = rom_host(&store, map_banks(0));
    host.queries.insert(FieldQuery::PlayerGender, 1);
    let mut env = ScriptEnvironment::new();
    env.run_map_load_script(&mut host, 7, 100).unwrap();
    assert_eq!(host.vars_flags().var(VAR_OBJ_0), Some(0));

    // ON_RESUME (script 10): nothing until VAR_SCENE_NEW_BARK_TOWN_OW is
    // 1, then the friend and her Marill are shown and placed.
    let mut host = rom_host(&store, map_banks(0));
    let mut env = ScriptEnvironment::new();
    assert_eq!(env.run_map_load_script(&mut host, 10, 100), Ok(1));
    assert!(host.events_without_polls().is_empty());
    host.vars_flags().set_var(VAR_SCENE_NEW_BARK_TOWN_OW, 1);
    let mut env = ScriptEnvironment::new();
    assert_eq!(env.run_map_load_script(&mut host, 10, 100), Ok(1));
    assert!(!host.vars_flags().flag(FLAG_HIDE_NEW_BARK_FRIEND));
    let shown = host
        .actions()
        .iter()
        .filter(|a| matches!(a, FieldAction::ShowObject(_)))
        .count();
    assert_eq!(shown, 2);
    let placed: Vec<(u16, u16, u16)> = host
        .actions()
        .iter()
        .filter_map(|a| match a {
            FieldAction::SetObjectPosition { x, z, direction, .. } => Some((*x, *z, *direction)),
            _ => None,
        })
        .collect();
    assert_eq!(placed, vec![(686, 396, dir::WEST), (685, 396, dir::SOUTH)]);
}

#[test]
fn route_29_transition_runs_as_a_map_load_script() {
    let Some(store) = store() else { return };
    let mut host = rom_host(&store, map_banks(4));
    let mut env = ScriptEnvironment::new();
    assert_eq!(env.run_map_load_script(&mut host, 1, 100), Ok(2));
    assert_eq!(host.vars_flags().var(VAR_OBJ_1), Some(SPRITE_HEROINE));
    assert!(host.vars_flags().flag(FLAG_UNK_207), "no Zephyr Badge: hide the sibling");
    assert!(host.events_without_polls().is_empty());
}

/// Runs a scene script frame by frame with A held, returning the
/// frame count.
fn run_scene(host: &mut RecordingHost, env: &mut ScriptEnvironment, max: usize) -> usize {
    host.keys = Keys(key::A);
    for frame in 1..=max {
        match env.run_frame(host).unwrap() {
            FrameStatus::Running => {}
            FrameStatus::Finished { callback } => {
                assert!(!callback);
                return frame;
            }
        }
    }
    panic!("scene did not finish in {max} frames");
}

#[test]
fn bedroom_pc_script_runs_frame_by_frame() {
    let Some(store) = store() else { return };
    // Script 1 (the PC), talked to from the front, no mail: the greeting,
    // then the "no mail" message dismissed with A.
    let mut host = rom_host(&store, map_banks(3));
    let mut env = ScriptEnvironment::new();
    env.setup(1, Some(0), dir::NORTH);
    let frames = run_scene(&mut host, &mut env, 100);
    assert!((5..=12).contains(&frames), "{frames} frames");
    assert_eq!(host.prints().len(), 2);
    let name = host.player_name.units().to_vec();
    assert!(
        host.prints()[0].units().windows(name.len()).any(|w| w == name),
        "the greeting names the player"
    );
    assert_eq!(
        host.events.iter().filter(|e| **e == HostEvent::DialogOpen).count(),
        2
    );
    assert_eq!(
        host.events.iter().filter(|e| **e == HostEvent::DialogClose).count(),
        2
    );
    let actions = host.actions();
    assert_eq!(
        actions[0],
        &FieldAction::LockAll {
            last_interacted: Some(0)
        }
    );
    assert!(matches!(actions[1], FieldAction::PlaySe(_)));
    assert_eq!(actions.last(), Some(&&FieldAction::ReleaseAll));
    assert!(!env.window_open() && !env.textbox_open());

    // With mail waiting: fade out, the mail application, restore the
    // overworld (a child task), fade in.
    let mut host = rom_host(&store, map_banks(3));
    host.queries.insert(FieldQuery::MailboxCount, 1);
    let mut env = ScriptEnvironment::new();
    env.setup(1, Some(0), dir::NORTH);
    run_scene(&mut host, &mut env, 100);
    assert_eq!(host.prints().len(), 1);
    assert!(host.events.contains(&HostEvent::Launch(AppRequest::Mail)));
    let fades = host
        .actions()
        .iter()
        .filter(|a| matches!(a, FieldAction::FadeScreen { .. }))
        .count();
    assert_eq!(fades, 2);
    assert!(host.actions().contains(&&FieldAction::RestoreOverworld));

    // Script 2 (the bookshelf): one message, dismissed with A.
    let mut host = rom_host(&store, map_banks(3));
    let mut env = ScriptEnvironment::new();
    env.setup(2, None, dir::NORTH);
    run_scene(&mut host, &mut env, 100);
    assert_eq!(host.prints().len(), 1);
}

#[test]
fn mom_scene_runs_through_two_call_stds() {
    let Some(store) = store() else { return };
    // The player's house on a new game: the frame table picks script 1.
    let mut host = rom_host(&store, map_banks(2));
    let env = ScriptEnvironment::new();
    let script = header(&store, 2)
        .scene_script_id(ON_FRAME_TABLE, |v| env.var_get(&mut host, v).unwrap())
        .unwrap();
    assert_eq!(script, 1);
    let mut env = ScriptEnvironment::new();
    env.setup(script, None, dir::SOUTH);
    let frames = run_scene(&mut host, &mut env, 400);
    // Two Waits (30 + 15 frames) plus the yields around them.
    assert!((50..=120).contains(&frames), "{frames} frames");
    for flag in [
        FLAG_GOT_BAG,
        FLAG_GOT_TRAINER_CARD,
        FLAG_GOT_SAVE_BUTTON,
        FLAG_GOT_OPTIONS_BUTTON,
    ] {
        assert!(host.vars_flags().flag(flag), "flag {flag:#x}");
    }
    assert_eq!(host.vars_flags().var(VAR_SCENE_PLAYERS_HOUSE_1F), Some(1));
    assert_eq!(env.active_count(), 0);
    let movements = host
        .events
        .iter()
        .filter(|e| matches!(e, HostEvent::Movement { .. }))
        .count();
    assert_eq!(movements, 4, "the player once, Mom three times");
    assert_eq!(host.prints().len(), 5);
    let fanfares = host
        .actions()
        .iter()
        .filter(|a| matches!(a, FieldAction::PlayFanfare(_)))
        .count();
    assert_eq!(fanfares, 4);
    // std_play_mom_music / std_fade_end_mom_music ran in a second context.
    let actions = host.actions();
    assert!(actions.iter().any(|a| matches!(a, FieldAction::TempBgm(_))));
    assert!(actions.iter().any(|a| matches!(a, FieldAction::FadeOutBgm { .. })));
    assert!(actions.iter().any(|a| matches!(a, FieldAction::ResetBgm)));
    assert_eq!(
        actions.iter().filter(|a| matches!(a, FieldAction::StopBgm)).count(),
        2
    );
    assert_eq!(actions.last(), Some(&&FieldAction::ReleaseAll));
}
