//! Integration tests: the field start menu (pret `src/start_menu.c`
//! + overlay 27's sub-screen grid) against a retail HeartGold (US)
//! dump — the entry sets the save flags produce, the d-pad walk and
//! its column wrap, the A/touch picks, and the B/X close.
//!
//! The new-game facts come from pret's scripts: a fresh save carries
//! none of the unlock flags, and Mom's bedroom scene
//! (`scr_seq_0845_T20R0201.s:29-41`) sets BAG, TRAINER CARD, SAVE,
//! and OPTIONS in turn; the starter (`scr_seq_0843_T20R0101.s:170`),
//! Pokégear (`scr_seq_0845_T20R0201.s:124`), and Pokédex
//! (`scr_seq_0229_R30R0201.s:240`) come later.
//!
//! Skips silently when `hg_usa.nds` is absent.

use std::sync::{Arc, Mutex, OnceLock};

use apricorn_core::app::start_menu::{
    MapLoadType, StartMenu, StartMenuAction, StartMenuEvent, StartMenuHost, flag,
};
use apricorn_core::assets::AssetStore;
use apricorn_core::frame::LogicalFrame;
use apricorn_core::input::{Input, Keys, Touch, key};
use apricorn_core::text::string::GameString;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// The shared, once-opened pinned dump, or `None` to skip silently.
fn store() -> Option<Arc<Mutex<AssetStore>>> {
    static STORE: OnceLock<Option<Arc<Mutex<AssetStore>>>> = OnceLock::new();
    STORE
        .get_or_init(|| match AssetStore::open(std::path::Path::new(ROM_PATH)) {
            Ok(store) => Some(Arc::new(Mutex::new(store))),
            Err(_) => {
                eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
                None
            }
        })
        .clone()
}

/// A flag-set field: the save flags a test names, the player "ETHAN".
struct Host {
    flags: Vec<u16>,
}

impl Host {
    fn new(flags: &[u16]) -> Self {
        Self {
            flags: flags.to_vec(),
        }
    }
}

impl StartMenuHost for Host {
    fn flag(&self, id: u16) -> bool {
        self.flags.contains(&id)
    }
    fn var(&self, _id: u16) -> u16 {
        0
    }
    fn party_count(&self) -> u8 {
        0
    }
    fn player_name(&self) -> GameString {
        GameString::from_units(&[0x12F, 0x13E, 0x132, 0x12B, 0x138])
    }
    fn player_gender(&self) -> u8 {
        0
    }
    fn has_running_shoes(&self) -> bool {
        false
    }
    fn map_load_type(&self) -> MapLoadType {
        MapLoadType::Overworld
    }
}

/// Mom's four gifts.
const AFTER_MOM: [u16; 4] = [
    flag::GOT_BAG,
    flag::GOT_TRAINER_CARD,
    flag::GOT_SAVE_BUTTON,
    flag::GOT_OPTIONS_BUTTON,
];

/// Every icon unlocked.
const FULL: [u16; 7] = [
    flag::GOT_POKEDEX,
    flag::GOT_STARTER,
    flag::GOT_BAG,
    flag::GOT_POKEGEAR,
    flag::GOT_TRAINER_CARD,
    flag::GOT_SAVE_BUTTON,
    flag::GOT_OPTIONS_BUTTON,
];

/// The field's X press that opens the menu (`field_control.c:303-306`)
/// — the input of the opening frame, which seeds the menu's edge state
/// the way the global `gSystem.newKeys` does.
const X_PRESS: Input = Input {
    keys: Keys(key::X),
    touch: None,
};

/// One inputless tick.
fn idle(menu: &mut StartMenu, host: &dyn StartMenuHost) -> StartMenuEvent {
    menu.tick(apricorn_core::Frame { index: 0 }, Input::default(), host)
}

/// A press: one tick with `keys` held, one with them released (the
/// menu reads `gSystem.newKeys` edges, so a held key is one press).
/// Returns the first event the pair produced.
fn press(menu: &mut StartMenu, keys: u16, host: &dyn StartMenuHost) -> StartMenuEvent {
    let event = menu.tick(
        apricorn_core::Frame { index: 0 },
        Input {
            keys: Keys(keys),
            touch: None,
        },
        host,
    );
    let released = idle(menu, host);
    if event == StartMenuEvent::None {
        released
    } else {
        event
    }
}

/// One tick with the stylus at `(x, y)`.
fn touch(menu: &mut StartMenu, x: u16, y: u16, host: &dyn StartMenuHost) -> StartMenuEvent {
    menu.tick(
        apricorn_core::Frame { index: 0 },
        Input {
            keys: Keys::IDLE,
            touch: Some(Touch { x, y }),
        },
        host,
    )
}

/// Idle ticks until a terminal event (or `limit` ticks pass).
fn run_until_event(
    menu: &mut StartMenu,
    host: &dyn StartMenuHost,
    limit: u32,
) -> (StartMenuEvent, u32) {
    for i in 1..=limit {
        let event = idle(menu, host);
        if event != StartMenuEvent::None {
            return (event, i);
        }
    }
    (StartMenuEvent::None, limit)
}

#[test]
fn a_fresh_save_lists_the_three_default_entries_but_draws_no_icon() {
    let Some(store) = store() else { return };
    let host = Host::new(&[]);
    let base = LogicalFrame::default();
    let menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    // StartMenu_BuildActionLists over the normal mask: TRAINER CARD,
    // SAVE, OPTIONS, EXIT in order, then 9 and 10 into display slots 7/8.
    assert_eq!(
        menu.insertion_order(),
        [
            StartMenuAction::TrainerCard,
            StartMenuAction::Save,
            StartMenuAction::Options,
            StartMenuAction::RunningShoes,
            StartMenuAction::Action9,
            StartMenuAction::Action10,
        ]
    );
    assert_eq!(
        &menu.actions()[..4],
        &[
            StartMenuAction::TrainerCard,
            StartMenuAction::Save,
            StartMenuAction::Options,
            StartMenuAction::RunningShoes,
        ]
    );
    assert!(!menu.is_union_room());
    assert_eq!(menu.layout(), 0);
    // FieldSystem_ShouldDrawStartMenuIcon is false for every icon: the
    // grid is empty, no label window, no header, no cursor slot.
    assert!(menu.grid().iter().all(|&(_, visible)| !visible));
    assert_eq!(menu.selected_slot(), None);
    assert!(menu.frame().sub.windows.is_empty());
    assert!(menu.frame().sub.sprites.is_empty());
    // The top-screen bar and its cursor are up regardless.
    assert!(menu.frame().main.bgs[3].enabled);
    assert!(menu.frame().main.bgs[3].screen.is_some());
    assert_eq!(menu.frame().main.sprites.len(), 1);
    let cursor = &menu.frame().main.sprites[0];
    assert_eq!((cursor.x, cursor.y), (100, 144));
    // A does nothing: unkD3 = 0 picks TRAINER CARD, whose icon gate
    // is shut, so FieldSystem_StartMenuActionIsAvailable refuses it.
    let mut menu = menu;
    assert_eq!(press(&mut menu, key::A, &host), StartMenuEvent::None);
    assert!(menu.is_open());
    assert_eq!(menu.last_action(), None);
}

#[test]
fn after_moms_gifts_the_four_icons_light_and_the_walk_wraps_the_right_column() {
    let Some(store) = store() else { return };
    let host = Host::new(&AFTER_MOM);
    let base = LogicalFrame::default();
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    // Layout row 0: slot 2 BAG, 4 TRAINER CARD, 5 SAVE, 6 OPTIONS lit.
    let visible: Vec<usize> = menu
        .grid()
        .iter()
        .enumerate()
        .filter(|&(_, &(_, v))| v)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(visible, [2, 4, 5, 6]);
    assert_eq!(
        menu.insertion_order()[..4],
        [
            StartMenuAction::Bag,
            StartMenuAction::TrainerCard,
            StartMenuAction::Save,
            StartMenuAction::Options,
        ]
    );
    // Four label windows plus the MENU header; four icons and the
    // (X) glyph.
    assert_eq!(menu.frame().sub.windows.len(), 5);
    assert_eq!(menu.frame().sub.sprites.len(), 5);
    // The cursor lands on the first visible slot, the BAG.
    assert_eq!(menu.selected_slot(), Some(2));
    assert_eq!(menu.selection_index(), 0);
    // UP/DOWN in the left column find no other lit slot: no move.
    press(&mut menu, key::UP, &host);
    assert_eq!(menu.selected_slot(), Some(2));
    press(&mut menu, key::DOWN, &host);
    assert_eq!(menu.selected_slot(), Some(2));
    // RIGHT crosses to the right column's row-2 slot (OPTIONS).
    press(&mut menu, key::RIGHT, &host);
    assert_eq!(menu.selected_slot(), Some(6));
    assert_eq!(menu.selection_index(), 3);
    // UP walks the column and wraps from the top back to the bottom.
    press(&mut menu, key::UP, &host);
    assert_eq!(menu.selected_slot(), Some(5));
    assert_eq!(menu.selection_index(), 2);
    press(&mut menu, key::UP, &host);
    assert_eq!(menu.selected_slot(), Some(4));
    assert_eq!(menu.selection_index(), 1);
    press(&mut menu, key::UP, &host);
    assert_eq!(menu.selected_slot(), Some(6), "UP from the top wraps");
    press(&mut menu, key::DOWN, &host);
    assert_eq!(menu.selected_slot(), Some(4), "DOWN from the bottom wraps");
    // A held key is one edge: holding UP does not keep walking.
    press(&mut menu, key::UP, &host);
    assert_eq!(menu.selected_slot(), Some(6));
    for _ in 0..3 {
        menu.tick(
            apricorn_core::Frame { index: 0 },
            Input {
                keys: Keys(key::UP),
                touch: None,
            },
            &host,
        );
    }
    assert_eq!(
        menu.selected_slot(),
        Some(5),
        "one edge, however long the hold"
    );
    // LEFT from SAVE: `ov27_0225D0B4[5][LEFT]` is POKéMON, then SAVE
    // itself — the unlit POKéMON falls back to staying put.
    press(&mut menu, key::LEFT, &host);
    assert_eq!(
        menu.selected_slot(),
        Some(5),
        "no lit slot across from SAVE"
    );
    // LEFT from OPTIONS crosses to the BAG.
    press(&mut menu, key::DOWN, &host);
    assert_eq!(menu.selected_slot(), Some(6));
    press(&mut menu, key::LEFT, &host);
    assert_eq!(menu.selected_slot(), Some(2));
    assert_eq!(menu.selection_index(), 0);
}

#[test]
fn the_full_grid_wraps_both_columns_and_remembers_the_field_selection() {
    let Some(store) = store() else { return };
    let host = Host::new(&FULL);
    let base = LogicalFrame::default();
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    assert_eq!(
        menu.insertion_order()[..8],
        [
            StartMenuAction::Pokedex,
            StartMenuAction::Pokemon,
            StartMenuAction::Bag,
            StartMenuAction::Pokegear,
            StartMenuAction::TrainerCard,
            StartMenuAction::Save,
            StartMenuAction::Options,
            StartMenuAction::RunningShoes,
        ]
    );
    assert!(menu.grid().iter().all(|&(_, v)| v));
    assert_eq!(menu.frame().sub.windows.len(), 8);
    assert_eq!(menu.selected_slot(), Some(0));
    press(&mut menu, key::UP, &host);
    assert_eq!(
        menu.selected_slot(),
        Some(3),
        "the left column wraps to POKéGEAR"
    );
    press(&mut menu, key::DOWN, &host);
    assert_eq!(menu.selected_slot(), Some(0));
    press(&mut menu, key::RIGHT, &host);
    assert_eq!(menu.selected_slot(), Some(4));
    press(&mut menu, key::UP, &host);
    assert_eq!(
        menu.selected_slot(),
        Some(6),
        "the right column wraps to OPTIONS"
    );
    press(&mut menu, key::LEFT, &host);
    assert_eq!(menu.selected_slot(), Some(2), "row 2 of the left column");
    assert_eq!(menu.selection_index(), 2);

    // The open restores the field's unkD3 (the k-th visible slot).
    struct Remembering(Host);
    impl StartMenuHost for Remembering {
        fn flag(&self, id: u16) -> bool {
            self.0.flag(id)
        }
        fn var(&self, id: u16) -> u16 {
            self.0.var(id)
        }
        fn party_count(&self) -> u8 {
            0
        }
        fn player_name(&self) -> GameString {
            self.0.player_name()
        }
        fn player_gender(&self) -> u8 {
            1
        }
        fn has_running_shoes(&self) -> bool {
            false
        }
        fn last_menu_selection(&self) -> u8 {
            5
        }
    }
    let remembering = Remembering(Host::new(&FULL));
    let menu = StartMenu::open(&store, &remembering, &base, X_PRESS).unwrap();
    assert_eq!(menu.selected_slot(), Some(5), "SAVE was the last pick");
    assert_eq!(menu.selection_index(), 5);
}

#[test]
fn a_picks_the_cursor_slot_after_the_fade_and_save_switches_at_once() {
    let Some(store) = store() else { return };
    let host = Host::new(&FULL);
    let base = LogicalFrame::default();
    // OPTIONS: the six-step brightness fade, then the launch with the
    // bar cleared and the screens black.
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    press(&mut menu, key::RIGHT, &host);
    press(&mut menu, key::UP, &host);
    assert_eq!(menu.selected_slot(), Some(6));
    assert_eq!(press(&mut menu, key::A, &host), StartMenuEvent::None);
    assert_eq!(menu.last_action(), Some(StartMenuAction::Options));
    assert!(!menu.input_touch_mode());
    let (event, ticks) = run_until_event(&mut menu, &host, 30);
    assert_eq!(event, StartMenuEvent::Selected(StartMenuAction::Options));
    assert!(
        (3..=8).contains(&ticks),
        "FieldMap_FadeScreen: six one-frame steps (one spent on the release), got {ticks}"
    );
    assert!(!menu.is_open());
    assert_eq!(
        menu.frame().main.bgs[3].screen,
        None,
        "sub_0203C38C cleared the bar"
    );
    assert!(menu.frame().main.sprites.is_empty(), "the cursor is gone");
    assert_eq!(menu.frame().main.brightness, menu.frame().sub.brightness);
    assert_ne!(
        menu.frame().main.brightness,
        base.main.brightness,
        "faded out"
    );
    assert_eq!(
        press(&mut menu, key::A, &host),
        StartMenuEvent::None,
        "done stays done"
    );
    assert_eq!(idle(&mut menu, &host), StartMenuEvent::None);

    // SAVE: no fade — the touch save app takes the sub screen at once.
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    press(&mut menu, key::RIGHT, &host);
    press(&mut menu, key::DOWN, &host);
    assert_eq!(menu.selected_slot(), Some(5));
    assert_eq!(
        press(&mut menu, key::A, &host),
        StartMenuEvent::Selected(StartMenuAction::Save)
    );
    assert!(!menu.is_open());

    // The dex on the first slot, straight away.
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    press(&mut menu, key::A, &host);
    let (event, _) = run_until_event(&mut menu, &host, 30);
    assert_eq!(event, StartMenuEvent::Selected(StartMenuAction::Pokedex));
}

#[test]
fn b_and_x_close_and_restore_the_base_frame_but_start_does_not() {
    let Some(store) = store() else { return };
    let host = Host::new(&FULL);
    let mut base = LogicalFrame::default();
    base.main.backdrop = 0x7C1F;
    base.sub.backdrop = 0x03E0;
    for close in [key::B, key::X] {
        let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
        assert_ne!(menu.frame(), &base);
        assert_eq!(
            menu.frame().sub.backdrop,
            0x03E0,
            "the sub backdrop carries over"
        );
        // START is not in Task_StartMenu_HandleInput's close mask.
        assert_eq!(press(&mut menu, key::START, &host), StartMenuEvent::None);
        assert!(menu.is_open());
        // The edge sets START_MENU_STATE_CLOSE; the next pass closes.
        assert_eq!(press(&mut menu, close, &host), StartMenuEvent::Closed);
        assert!(!menu.is_open());
        assert_eq!(menu.frame(), &base);
        assert_eq!(idle(&mut menu, &host), StartMenuEvent::None);
    }
    // The header strip is the touch close: ov27 queues 1, the C task
    // reads it on its next pass.
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    assert_eq!(touch(&mut menu, 100, 5, &host), StartMenuEvent::None);
    assert_eq!(menu.last_touch_menu_input(), 1);
    assert_eq!(
        idle(&mut menu, &host),
        StartMenuEvent::None,
        "the C task consumes it"
    );
    assert_eq!(idle(&mut menu, &host), StartMenuEvent::Closed);
    assert_eq!(menu.frame(), &base);
}

#[test]
fn a_stylus_press_on_an_icon_selects_it_through_the_touch_queue() {
    let Some(store) = store() else { return };
    let host = Host::new(&FULL);
    let base = LogicalFrame::default();
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    // Slot 5 (SAVE) sits at (96..156, 62..94).
    assert_eq!(touch(&mut menu, 120, 75, &host), StartMenuEvent::None);
    assert_eq!(menu.selected_slot(), Some(5));
    assert_eq!(menu.last_touch_menu_input(), 2 + 5);
    // The next C-task pass reads the queue: SAVE switches at once.
    assert_eq!(
        idle(&mut menu, &host),
        StartMenuEvent::Selected(StartMenuAction::Save)
    );
    assert!(menu.input_touch_mode());
    assert_eq!(menu.last_touch_menu_input(), 0);

    // A press on an unlit slot is swallowed: nothing queued, no move.
    let host = Host::new(&AFTER_MOM);
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    assert_eq!(touch(&mut menu, 30, 30, &host), StartMenuEvent::None);
    assert_eq!(menu.selected_slot(), Some(2));
    assert_eq!(menu.last_touch_menu_input(), 0);
    // Holding the stylus is one press; releasing and pressing again
    // on the BAG queues it.
    assert_eq!(touch(&mut menu, 30, 110, &host), StartMenuEvent::None);
    assert_eq!(menu.last_touch_menu_input(), 0, "still the same contact");
    idle(&mut menu, &host);
    assert_eq!(touch(&mut menu, 30, 110, &host), StartMenuEvent::None);
    assert_eq!(menu.last_touch_menu_input(), 2);
    assert_eq!(
        idle(&mut menu, &host),
        StartMenuEvent::None,
        "the fade begins"
    );
    let (event, _) = run_until_event(&mut menu, &host, 30);
    assert_eq!(event, StartMenuEvent::Selected(StartMenuAction::Bag));
}

#[test]
fn a_button_or_stylus_held_across_the_open_is_not_a_new_press() {
    let Some(store) = store() else { return };
    let host = Host::new(&FULL);
    let base = LogicalFrame::default();
    let held = |keys: u16| Input {
        keys: Keys(keys),
        touch: None,
    };
    let tick = |menu: &mut StartMenu, input: Input| {
        menu.tick(apricorn_core::Frame { index: 0 }, input, &host)
    };

    // The X that opened the menu is the field's edge (gSystem.newKeys
    // is global): however long it stays down, Task_StartMenu_HandleInput
    // never sees it as new, so the menu stays up.
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    for _ in 0..10 {
        assert_eq!(tick(&mut menu, held(key::X)), StartMenuEvent::None);
        assert!(menu.is_open(), "a held X does not close the menu");
    }
    // Released and pressed again, it is an edge: CLOSE, then Closed.
    assert_eq!(tick(&mut menu, Input::default()), StartMenuEvent::None);
    assert_eq!(tick(&mut menu, held(key::X)), StartMenuEvent::None);
    assert_eq!(tick(&mut menu, Input::default()), StartMenuEvent::Closed);
    assert_eq!(menu.frame(), &base);

    // The same for B, and for a B that outlives the X it was held with.
    let mut menu = StartMenu::open(&store, &host, &base, held(key::X | key::B)).unwrap();
    for _ in 0..3 {
        assert_eq!(tick(&mut menu, held(key::B)), StartMenuEvent::None);
        assert!(menu.is_open());
    }
    assert_eq!(tick(&mut menu, Input::default()), StartMenuEvent::None);
    assert_eq!(tick(&mut menu, held(key::B)), StartMenuEvent::None);
    assert_eq!(tick(&mut menu, Input::default()), StartMenuEvent::Closed);

    // A held A picks nothing; a fresh A picks the cursor slot.
    let mut menu = StartMenu::open(&store, &host, &base, held(key::X | key::A)).unwrap();
    for _ in 0..3 {
        assert_eq!(tick(&mut menu, held(key::A)), StartMenuEvent::None);
        assert_eq!(menu.last_action(), None, "a held A picks nothing");
    }
    assert_eq!(tick(&mut menu, Input::default()), StartMenuEvent::None);
    assert_eq!(tick(&mut menu, held(key::A)), StartMenuEvent::None);
    assert_eq!(menu.last_action(), Some(StartMenuAction::Pokedex));

    // A stylus already down at the open is not a new touch
    // (TouchscreenHitbox_FindRectAtTouchNew): resting on SAVE queues
    // nothing and moves nothing until it is lifted and pressed again.
    let on_save = Input {
        keys: Keys::IDLE,
        touch: Some(Touch { x: 120, y: 75 }),
    };
    let mut menu = StartMenu::open(&store, &host, &base, on_save).unwrap();
    for _ in 0..3 {
        assert_eq!(tick(&mut menu, on_save), StartMenuEvent::None);
        assert_eq!(menu.selected_slot(), Some(0), "the cursor stays on the dex");
        assert_eq!(menu.last_touch_menu_input(), 0, "nothing queued");
    }
    assert_eq!(tick(&mut menu, Input::default()), StartMenuEvent::None);
    assert_eq!(tick(&mut menu, on_save), StartMenuEvent::None);
    assert_eq!(menu.selected_slot(), Some(5));
    assert_eq!(menu.last_touch_menu_input(), 2 + 5);
    assert_eq!(
        tick(&mut menu, Input::default()),
        StartMenuEvent::Selected(StartMenuAction::Save)
    );
}
