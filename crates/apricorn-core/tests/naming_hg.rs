//! Retail player-name entry tests. No ROM content is committed.
use apricorn_core::app::{
    App, ChainNext,
    naming::{NamingScreen, NamingState},
};
use apricorn_core::assets::AssetStore;
use apricorn_core::input::{Input, Keys, Touch, key};
use apricorn_core::rng::Lcrng;
use std::sync::{Mutex, OnceLock};

fn store() -> Option<&'static Mutex<AssetStore>> {
    static STORE: OnceLock<Option<Mutex<AssetStore>>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            let path =
                std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds"));
            path.exists()
                .then(|| Mutex::new(AssetStore::open(path).unwrap()))
        })
        .as_ref()
}
fn tick(app: &mut NamingScreen, keys: u16, touch: Option<Touch>) {
    app.tick(
        apricorn_core::Frame { index: 0 },
        Input {
            keys: Keys(keys),
            touch,
        },
    );
}
fn press(app: &mut NamingScreen, keys: u16) {
    tick(app, 0, None);
    tick(app, keys, None);
}
fn ready(store: &Mutex<AssetStore>, gender: u8) -> NamingScreen {
    let mut app = NamingScreen::load(store, gender).unwrap();
    for _ in 0..24 {
        tick(&mut app, 0, None);
    }
    assert!(app.accepts_input());
    app
}
fn finish(app: &mut NamingScreen) {
    press(app, key::START);
    assert_eq!(app.cursor(), (12, 0));
    assert_eq!(app.next(), ChainNext::Stay, "Start only focuses OK");
    press(app, key::A);
    for _ in 0..34 {
        tick(app, 0, None);
    }
    assert_eq!(app.state(), NamingState::Done);
}

#[test]
fn keyboard_entry_pages_delete_repeat_and_limit() {
    let Some(store) = store() else {
        return;
    };
    let mut app = ready(store, 0);
    assert_eq!(&app.keyboard(0)[1][..10], &(299..309).collect::<Vec<_>>());
    assert_eq!(&app.keyboard(1)[1][..10], &(325..335).collect::<Vec<_>>());
    press(&mut app, key::LEFT);
    assert_eq!(app.cursor(), (12, 1), "left wraps");
    press(&mut app, key::RIGHT);
    press(&mut app, key::A);
    for _ in 0..20 {
        tick(&mut app, key::A, None);
    }
    assert_eq!(app.entry(), &[299], "held A must not type repeatedly");
    press(&mut app, key::R);
    assert_eq!(app.entry(), &[299], "R only converts Japanese code units");
    press(&mut app, key::SELECT);
    assert!(!app.accepts_input());
    for _ in 0..14 {
        tick(&mut app, 0, None);
    }
    assert!(app.accepts_input());
    assert_eq!(app.page(), 1);
    press(&mut app, key::A);
    assert_eq!(app.entry(), &[299, 325]);
    press(&mut app, key::B);
    assert_eq!(app.entry(), &[299]);
    for _ in 0..6 {
        press(&mut app, key::A);
    }
    assert_eq!(app.entry().len(), 7);
    assert!(
        !app.accepts_input(),
        "full-entry cursor animation gates input"
    );
    for _ in 0..30 {
        tick(&mut app, 0, None);
    }
    assert_eq!(app.cursor(), (12, 0), "full name moves focus to OK");
    let mut rng = Lcrng::new(123);
    finish(&mut app);
    assert_eq!(
        app.result(&mut rng).units(),
        &[299, 325, 325, 325, 325, 325, 325]
    );
    assert_eq!(rng.seed(), 123, "typed names do not consume RNG");
}

#[test]
fn home_row_skips_duplicates_and_touch_boundaries_are_inclusive() {
    let Some(store) = store() else {
        return;
    };
    let mut app = ready(store, 0);
    press(&mut app, key::UP);
    assert_eq!(app.cursor(), (0, 0));
    for expected in [2, 4, 8, 11, 0] {
        press(&mut app, key::RIGHT);
        assert_eq!(app.cursor(), (expected, 0));
    }
    // x=44 lies on A's inclusive right boundary as well as B's left.
    tick(&mut app, 0, Some(Touch { x: 44, y: 89 }));
    assert_eq!(app.entry(), &[299]);
    tick(&mut app, 0, Some(Touch { x: 61, y: 89 }));
    assert_eq!(app.entry(), &[299], "dragging held stylus is not a new key");
    tick(&mut app, 0, None);
    tick(&mut app, 0, Some(Touch { x: 61, y: 89 }));
    assert_eq!(app.entry(), &[299, 301]);
}

#[test]
fn blank_and_spaces_choose_gender_default_with_exactly_one_rng_draw() {
    let Some(store) = store() else {
        return;
    };
    for gender in 0..2 {
        for spaces in [false, true] {
            let mut app = ready(store, gender);
            if spaces {
                tick(&mut app, 0, Some(Touch { x: 189, y: 89 }));
                assert_eq!(app.entry(), &[478]);
            }
            finish(&mut app);
            let mut rng = Lcrng::new(0x12345678);
            let mut expected = rng;
            let index = usize::from(expected.next_u16() % 18) + usize::from(gender) * 18;
            let expected_name = {
                let mut store = store.lock().unwrap();
                let bank = store.load_msg_bank("a/0/2/7", 254).unwrap();
                apricorn_core::text::string::GameString::from_units(
                    store.msg_bank(bank).unwrap().message(index).unwrap(),
                )
            };
            assert_eq!(app.result(&mut rng), &expected_name);
            assert_eq!(rng.seed(), expected.seed());
            assert_eq!(app.result(&mut rng), &expected_name);
            assert_eq!(
                rng.seed(),
                expected.seed(),
                "result retrieval is idempotent"
            );
        }
    }
}

#[test]
fn keyboard_effects_glow_wiggle_and_press_from_rom() {
    let Some(store) = store() else {
        return;
    };
    let mut app = ready(store, 0);
    let mut colors = std::collections::BTreeSet::new();
    for _ in 0..38 {
        tick(&mut app, 0, None);
        let &(slot, color) = app.frame().main.obj_palette_overrides.first().unwrap();
        assert_eq!(slot, 0x1d);
        assert_eq!(color & 31, 29);
        assert_eq!(color >> 10, 0);
        colors.insert((color >> 5) & 31);
    }
    assert!(colors.len() > 8, "cursor must pulse while idle");
    press(&mut app, key::SELECT);
    let mut positions = Vec::new();
    for _ in 0..30 {
        tick(&mut app, 0, None);
        positions.push(
            app.frame()
                .main
                .sprites
                .iter()
                .find(|s| s.sequence == 37)
                .unwrap()
                .x,
        );
    }
    assert!(
        positions
            .windows(7)
            .any(|p| p == [26, 26, 19, 19, 24, 24, 22])
    );
    assert_eq!(*positions.last().unwrap(), 22);
    press(&mut app, key::A);
    press(&mut app, key::B);
    assert!(
        app.frame()
            .main
            .sprites
            .iter()
            .any(|s| s.sequence == 24 && s.elapsed == 0)
    );
    press(&mut app, key::START);
    press(&mut app, key::A);
    assert!(
        app.frame()
            .main
            .sprites
            .iter()
            .any(|s| s.sequence == 26 && s.elapsed == 0)
    );
}
