//! Renders the field start menu (pret `src/start_menu.c` + overlay
//! 27's sub-screen icon grid) against the user's retail ROM and pins
//! the composed screens by SHA-1. Set `APRICORN_RENDER_OUT` to save
//! review PNGs.

use apricorn_core::app::start_menu::{MapLoadType, StartMenu, StartMenuEvent, StartMenuHost, flag};
use apricorn_core::assets::AssetStore;
use apricorn_core::frame::LogicalFrame;
use apricorn_core::input::{Input, Keys, key};
use apricorn_core::text::string::GameString;
use apricorn_gfx::render;
use sha1::{Digest, Sha1};
use std::sync::Mutex;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

fn save_screens(name: &str, screens: &[apricorn_gfx::ScreenBuffer; 2]) {
    if let Some(out) = std::env::var_os("APRICORN_RENDER_OUT") {
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        let mut encoder =
            png::Encoder::new(std::fs::File::create(out.join(name)).unwrap(), 256, 384);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = screens
            .iter()
            .flat_map(|s| s.as_rgba().as_flattened().iter().copied())
            .collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
}

fn sha1(screens: &[apricorn_gfx::ScreenBuffer; 2]) -> String {
    let mut hasher = Sha1::new();
    for screen in screens {
        hasher.update(screen.as_rgba().as_flattened());
    }
    format!("{:x}", hasher.finalize())
}

/// A flag-set field: the save flags a test names, the player "ETHAN".
struct Host {
    flags: Vec<u16>,
    gender: u8,
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
        // "ETHAN" in the Gen IV charmap (A = 0x12B).
        GameString::from_units(&[0x12F, 0x13E, 0x132, 0x12B, 0x138])
    }
    fn player_gender(&self) -> u8 {
        self.gender
    }
    fn has_running_shoes(&self) -> bool {
        false
    }
    fn map_load_type(&self) -> MapLoadType {
        MapLoadType::Overworld
    }
}

/// Every icon unlocked: the seven-icon grid of the mid-game field.
fn full_host(gender: u8) -> Host {
    Host {
        flags: vec![
            flag::GOT_POKEDEX,
            flag::GOT_STARTER,
            flag::GOT_BAG,
            flag::GOT_POKEGEAR,
            flag::GOT_TRAINER_CARD,
            flag::GOT_SAVE_BUTTON,
            flag::GOT_OPTIONS_BUTTON,
        ],
        gender,
    }
}

/// The field's X press that opens the menu — the opening frame's input,
/// which seeds the menu's `gSystem.newKeys` edge state.
const X_PRESS: Input = Input {
    keys: Keys(key::X),
    touch: None,
};

fn tick(menu: &mut StartMenu, index: u32, keys: u16, host: &dyn StartMenuHost) -> StartMenuEvent {
    menu.tick(
        apricorn_core::Frame { index },
        Input {
            keys: Keys(keys),
            touch: None,
        },
        host,
    )
}

#[test]
fn opened_menu_over_black_pins_both_screens() {
    let path = std::path::Path::new(ROM_PATH);
    if !path.exists() {
        eprintln!("skipping: no ROM at {}", path.display());
        return;
    }
    let store = Mutex::new(AssetStore::open(path).unwrap());
    let base = LogicalFrame::default();

    // The full grid, male BAG: the first frame after the open pass.
    let host = full_host(0);
    let menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    let screens = render(menu.frame(), &*store.lock().unwrap());
    save_screens("start-menu-full-open.png", &screens);
    // The top screen: black field, the bar along the bottom rows only.
    let top = &screens[0];
    let bar_lit = (136..192)
        .flat_map(|y| (0..256).map(move |x| (x, y)))
        .filter(|&(x, y)| top.pixel(x, y)[..3] != [0, 0, 0])
        .count();
    let above_lit = (0..136)
        .flat_map(|y| (0..256).map(move |x| (x, y)))
        .filter(|&(x, y)| top.pixel(x, y)[..3] != [0, 0, 0])
        .count();
    assert!(
        bar_lit > 2000,
        "the MAIN BG3 bar fills the bottom rows: {bar_lit}"
    );
    assert_eq!(above_lit, 0, "nothing above the bar over a black base");
    // The touch screen: the panel, seven icons, labels — mostly lit.
    let bottom = &screens[1];
    let lit = (0..192)
        .flat_map(|y| (0..256).map(move |x| (x, y)))
        .filter(|&(x, y)| bottom.pixel(x, y)[..3] != [0, 0, 0])
        .count();
    assert!(lit > 30000, "the sub-screen panel covers the LCD: {lit}");

    // The new game after Mom's gifts (`scr_seq_0845_T20R0201.s:29-41`
    // sets BAG, TRAINER CARD, SAVE, OPTIONS): the female BAG, four icons.
    let after_mom = Host {
        flags: vec![
            flag::GOT_BAG,
            flag::GOT_TRAINER_CARD,
            flag::GOT_SAVE_BUTTON,
            flag::GOT_OPTIONS_BUTTON,
        ],
        gender: 1,
    };
    let menu = StartMenu::open(&store, &after_mom, &base, X_PRESS).unwrap();
    assert_eq!(
        menu.selected_slot(),
        Some(2),
        "the BAG is the first visible slot"
    );
    let screens_mom = render(menu.frame(), &*store.lock().unwrap());
    save_screens("start-menu-after-mom-open.png", &screens_mom);

    // The fresh bedroom landing: no flag set, so the C lists TRAINER
    // CARD/SAVE/OPTIONS/EXIT but the grid draws no icon, label, or
    // header — the bare panel.
    let fresh = Host {
        flags: Vec::new(),
        gender: 0,
    };
    let menu = StartMenu::open(&store, &fresh, &base, X_PRESS).unwrap();
    assert_eq!(menu.selected_slot(), None);
    assert!(menu.frame().sub.sprites.is_empty());
    assert!(menu.frame().sub.windows.is_empty());
    let screens_fresh = render(menu.frame(), &*store.lock().unwrap());
    save_screens("start-menu-fresh-open.png", &screens_fresh);

    assert_eq!(
        [sha1(&screens), sha1(&screens_mom), sha1(&screens_fresh)],
        [
            "193a08064b45deec6af3e6b4f020949228f3e1e2",
            "7b3012bce4ea5e73e7602602f829af63c3d2add8",
            "f1169c68e6436b78115ac4938961122bf2c4e7a5"
        ],
        "the full, post-Mom, and fresh menus over a black base frame"
    );
}

#[test]
fn cursor_walk_moves_the_highlight_and_close_restores_the_base() {
    let path = std::path::Path::new(ROM_PATH);
    if !path.exists() {
        eprintln!("skipping: no ROM at {}", path.display());
        return;
    }
    let store = Mutex::new(AssetStore::open(path).unwrap());
    let base = LogicalFrame::default();
    let host = full_host(1);
    let mut menu = StartMenu::open(&store, &host, &base, X_PRESS).unwrap();
    let opened = render(menu.frame(), &*store.lock().unwrap());
    assert_eq!(tick(&mut menu, 1, key::DOWN, &host), StartMenuEvent::None);
    assert_eq!(menu.selected_slot(), Some(1));
    let moved = render(menu.frame(), &*store.lock().unwrap());
    save_screens("start-menu-full-down.png", &moved);
    // The highlight palette follows the cursor: icon 0's box changes.
    let differs =
        |a: &apricorn_gfx::ScreenBuffer, b: &apricorn_gfx::ScreenBuffer, x0, x1, y0, y1| {
            (y0..y1)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|&(x, y)| a.pixel(x, y) != b.pixel(x, y))
                .count()
        };
    assert!(
        differs(&opened[1], &moved[1], 16, 76, 22, 54) > 50,
        "icon 0 lost its highlight"
    );
    assert!(
        differs(&opened[1], &moved[1], 16, 76, 62, 94) > 50,
        "icon 1 gained it"
    );
    assert_eq!(
        differs(&opened[1], &moved[1], 96, 156, 22, 54),
        0,
        "icon 4 untouched"
    );
    assert_eq!(tick(&mut menu, 2, key::B, &host), StartMenuEvent::None);
    assert_eq!(tick(&mut menu, 3, 0, &host), StartMenuEvent::Closed);
    assert_eq!(menu.frame(), &base, "close restores the base frame");
    let closed = render(menu.frame(), &*store.lock().unwrap());
    assert!(
        closed
            .iter()
            .all(|s| s.as_rgba().iter().all(|c| c[..3] == [0, 0, 0]))
    );
}
