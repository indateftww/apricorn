//! Integration tests: the boot apps and the game-state machine
//! against a retail HeartGold (US) dump — frame-indexed behavior,
//! the deterministic seam the future harness reuses.
//!
//! `apricorn_core::app`'s two Phase 3 apps port pret's boot scenes
//! tick for tick; these tests pin the timeline at exact frame
//! indices: the copyright beat's scroll, hold, `ev` ramp, and Game
//! Freak handover; the title screen's setup, flash, and logo
//! fade-in; both exits; and the `boot_chain` cycle that carries the
//! title's timeout back to the intro. The Phase 4 step 5 tests do
//! the same for the [`Game`] machine — the overlay chain boot →
//! title → save-check → new game → Oak → bedroom, the card-probe
//! status flags, and the `InitializeMainRNG` re-seed points, all at
//! exact global frame indices. Every tick is a pure function of the
//! frame index and input, so these indices are a contract.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM dump); the card-probe test also takes the retail
//! `hg.sav` when present (the `save.rs` policy).

use std::sync::{Arc, Mutex, OnceLock};

use apricorn_core::app::{
    App, BootChain, boot_chain, intro_copyright::IntroCopyright, title_screen::TitleScreen,
};
use apricorn_core::assets::AssetStore;
use apricorn_core::frame::{BlendEffect, BrightnessMode, ColorMode, DisplaySelect, plane};
use apricorn_core::input::{Input, Keys, key};
use apricorn_core::text::string::GameString;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// The shared, once-opened pinned dump (SHA-1 gate inside), or `None`
/// to skip silently.
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

/// One inputless tick.
fn idle_tick(app: &mut dyn App) {
    app.tick(apricorn_core::Frame { index: 0 }, Input::default());
}

/// An A-press tick (the button held for exactly one tick, the press
/// edge the apps consume).
fn a_tick(app: &mut dyn App) {
    app.tick(
        apricorn_core::Frame { index: 0 },
        Input {
            keys: Keys(key::A),
            touch: None,
        },
    );
}

#[test]
fn copyright_beat_frame_sequence() {
    let Some(store) = store() else {
        return;
    };
    let mut app = IntroCopyright::load(&store).expect("the pinned ROM's members load");

    // Construction: the cleared state after Scene1_Init — layers
    // configured, every plane off, blend neutral, engine B on top.
    let frame = app.frame();
    assert_eq!(frame.display, DisplaySelect::SubOnTop);
    assert!(
        frame
            .main
            .bgs
            .iter()
            .chain(frame.sub.bgs.iter())
            .all(|bg| !bg.enabled)
    );
    assert_eq!(frame.main.bgs[0].priority, 0);
    assert_eq!(frame.main.bgs[1].priority, 1);
    assert_eq!(frame.main.bgs[0].char_base, 1);
    assert_eq!(frame.main.bgs[1].char_base, 2);
    assert_eq!(frame.sub.bgs[0].char_base, 1);
    assert_eq!(
        frame.sub.bgs[1].char_base, 1,
        "the cover shares the logo's slot"
    );
    assert!(
        frame.sub.bgs[2].screen.is_none(),
        "SUB BG2 never receives a map"
    );
    assert_eq!(frame.main.char_blocks[1].len(), 1);
    assert_eq!(frame.main.char_blocks[2].len(), 1);
    assert_eq!(frame.sub.char_blocks[1].len(), 1);
    assert_eq!(frame.sub.char_blocks[4].len(), 1);
    assert_eq!(frame.main.blend.effect, BlendEffect::None);
    assert_eq!(frame.sub.blend.effect, BlendEffect::None);

    // Frame 0: APPEAR — MAIN BG0 scrolled down 128, both BG0 planes.
    idle_tick(&mut app);
    let frame = app.frame();
    assert_eq!(frame.main.bgs[0].scroll_y, 128);
    assert!(frame.main.bgs[0].enabled);
    assert!(frame.sub.bgs[0].enabled);
    assert!(!frame.sub.bgs[1].enabled, "the logo stays covered");
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Stay);

    // Frames 1–29: the hold, no blend yet.
    for _ in 1..30 {
        idle_tick(&mut app);
    }
    assert_eq!(app.frame().main.blend.effect, BlendEffect::None);

    // Frame 30: the fade starts — alpha toward the backdrop, both
    // engines, at the invisible first step.
    idle_tick(&mut app);
    for engine in [app.frame().main.blend, app.frame().sub.blend] {
        assert_eq!(engine.effect, BlendEffect::Alpha);
        assert_eq!(engine.plane1, plane::BG0);
        assert_eq!(engine.plane2, plane::BD);
        assert_eq!((engine.eva, engine.ebv), (31, 0));
    }

    // The exact ev ramp: ev = counter * 31 / 60, EVA = 31 - ev.
    // Counter 16 at frame 45: 16*31/60 = 8.
    for _ in 31..=45 {
        idle_tick(&mut app);
    }
    assert_eq!(
        (app.frame().main.blend.eva, app.frame().main.blend.ebv),
        (23, 8)
    );
    // Counter 31 at frame 60: 31*31/60 = 16.
    for _ in 46..=60 {
        idle_tick(&mut app);
    }
    assert_eq!(
        (app.frame().main.blend.eva, app.frame().main.blend.ebv),
        (15, 16)
    );
    // Counter 60 at frame 89: ev 31, the fade finished at full black.
    for _ in 61..=89 {
        idle_tick(&mut app);
    }
    assert_eq!(
        (app.frame().main.blend.eva, app.frame().main.blend.ebv),
        (0, 31)
    );

    // Frame 109: still the copyright (scrolled 128, logo covered).
    for _ in 90..=109 {
        idle_tick(&mut app);
    }
    let frame = app.frame();
    assert_eq!(frame.main.bgs[0].scroll_y, 128);
    assert!(frame.sub.bgs[0].enabled);
    assert!(!frame.sub.bgs[1].enabled);
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Stay);

    // Frame 110: the Game Freak logo — SUB BG0 off / BG1 on, MAIN
    // BG0's scroll reset, MAIN's blend back to EVA 31.
    idle_tick(&mut app);
    let frame = app.frame();
    assert!(!frame.sub.bgs[0].enabled);
    assert!(frame.sub.bgs[1].enabled);
    assert_eq!(frame.main.bgs[0].scroll_y, 0);
    assert_eq!(frame.main.blend.plane1, plane::BG0);
    assert_eq!(frame.main.blend.eva, 31);
    assert_eq!(frame.main.blend.ebv, 0);
    // SUB's blend keeps the fade's end state — BG1 is not in its
    // first-target mask, so the logo shows whole either way.
    assert_eq!(
        (app.frame().sub.blend.eva, app.frame().sub.blend.ebv),
        (0, 31)
    );

    // Frames 111–219: the logo hold.
    for _ in 111..=219 {
        idle_tick(&mut app);
    }
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Stay);

    // Frame 220: the 110-frame hold ends — the beat finishes.
    idle_tick(&mut app);
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Advance);
}

#[test]
fn copyright_beat_skip_after_gamefreak() {
    let Some(store) = store() else {
        return;
    };
    let mut app = IntroCopyright::load(&store).expect("the pinned ROM's members load");

    // A press during the copyright hold: not skippable yet.
    for _ in 0..=9 {
        idle_tick(&mut app);
    }
    a_tick(&mut app);
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Stay);
    idle_tick(&mut app);

    // Frames up to and including the Game Freak logo's frame (110):
    // a press on frame 110 is still too early — the skip becomes
    // allowed only within that frame, and the check runs at its top.
    for _ in 11..=108 {
        idle_tick(&mut app);
    }
    a_tick(&mut app);
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Stay);

    // A new press two frames later finishes the beat on that tick.
    idle_tick(&mut app);
    a_tick(&mut app);
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Advance);

    // START skips too, and so does a new stylus contact.
    let mut app = IntroCopyright::load(&store).expect("reload");
    for _ in 0..=112 {
        idle_tick(&mut app);
    }
    app.tick(
        apricorn_core::Frame { index: 0 },
        Input {
            keys: Keys(key::START),
            touch: None,
        },
    );
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Advance);
    let mut app = IntroCopyright::load(&store).expect("reload");
    for _ in 0..=112 {
        idle_tick(&mut app);
    }
    app.tick(
        apricorn_core::Frame { index: 0 },
        Input {
            keys: Keys::IDLE,
            touch: Some(apricorn_core::input::Touch { x: 128, y: 96 }),
        },
    );
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Advance);
}

#[test]
fn title_screen_frame_sequence() {
    let Some(store) = store() else {
        return;
    };
    let mut app = TitleScreen::load(&store).expect("the pinned ROM's members load");

    // Construction: the cleared state after TitleScreen_Init — every
    // plane off, engine B on top, the logo pre-blended to its second
    // target.
    let frame = app.frame();
    assert_eq!(frame.display, DisplaySelect::SubOnTop);
    assert!(
        frame
            .main
            .bgs
            .iter()
            .chain(frame.sub.bgs.iter())
            .all(|bg| !bg.enabled)
    );
    assert_eq!(frame.sub.blend.plane1, plane::BG2);
    assert_eq!(
        frame.sub.blend.plane2,
        plane::BG0 | plane::BG3 | plane::OBJ | plane::BD
    );
    assert_eq!((frame.sub.blend.eva, frame.sub.blend.ebv), (0, 31));
    assert_eq!(frame.main.bgs[0].priority, 1, "G2_SetBG0Priority(1)");
    assert!(frame.main.bgs[0].screen.is_none(), "the 3D BG0 is deferred");
    assert_eq!(frame.sub.bgs[1].color_mode, ColorMode::Bpp4);
    assert_eq!(frame.sub.bgs[2].color_mode, ColorMode::Bpp8);
    assert_eq!(frame.sub.bgs[3].color_mode, ColorMode::Bpp8);
    assert_eq!(frame.sub.bgs[1].char_base, 3);
    assert_eq!(frame.sub.bgs[2].char_base, 0);
    assert_eq!(frame.sub.bgs[3].char_base, 4);
    assert!(frame.main.char_blocks.iter().all(|block| block.is_empty()));

    // Frame 0: SETUP — the planes come on (MAIN BG3 waits for the
    // flash).
    idle_tick(&mut app);
    let frame = app.frame();
    for i in 0..3 {
        assert!(frame.main.bgs[i].enabled, "MAIN BG{i} on");
    }
    assert!(!frame.main.bgs[3].enabled);
    for i in 1..4 {
        assert!(frame.sub.bgs[i].enabled, "SUB BG{i} on");
    }

    // Frames 1–29: the initial delay — the window stays off.
    for _ in 1..=29 {
        idle_tick(&mut app);
    }
    assert!(!app.frame().main.bgs[3].enabled);

    // Frame 30: play starts; the flash timer is mid-cycle, so the
    // window's first ON is frame 46 — on for 30, off for 15, period
    // 45 from there.
    for _ in 30..=45 {
        idle_tick(&mut app);
    }
    assert!(!app.frame().main.bgs[3].enabled, "still off at frame 45");
    idle_tick(&mut app);
    assert!(
        app.frame().main.bgs[3].enabled,
        "the flash starts at frame 46"
    );
    for _ in 47..=75 {
        idle_tick(&mut app);
    }
    assert!(app.frame().main.bgs[3].enabled);
    idle_tick(&mut app);
    assert!(!app.frame().main.bgs[3].enabled, "off at frame 76");
    for _ in 77..=91 {
        idle_tick(&mut app);
    }
    assert!(app.frame().main.bgs[3].enabled, "the cycle wraps at 45");

    // The logo fade-in: a 3-frame title delay after play starts
    // (frame 33 = counter 1: offset 0, EVA 1), then EVA = t, offset
    // t/2, capped at 31 — the offset resting at 15, as in the game.
    let mut app = TitleScreen::load(&store).expect("reload");
    for _ in 0..=32 {
        idle_tick(&mut app);
    }
    assert_eq!(
        (app.frame().sub.blend.eva, app.frame().sub.blend.ebv),
        (0, 31)
    );
    assert_eq!(app.frame().sub.bgs[2].scroll_y, 0);
    idle_tick(&mut app);
    assert_eq!(app.frame().sub.blend.eva, 1);
    assert_eq!(app.frame().sub.blend.ebv, 30);
    for _ in 34..=63 {
        idle_tick(&mut app);
    }
    let frame = app.frame();
    assert_eq!(frame.sub.bgs[2].scroll_y, 15);
    assert_eq!((frame.sub.blend.eva, frame.sub.blend.ebv), (31, 0));
    idle_tick(&mut app);
    let frame = app.frame();
    assert_eq!(frame.sub.bgs[2].scroll_y, 15, "the fade stays capped");
    assert_eq!(frame.sub.blend.eva, 31);

    // The timeout: the play timer counts the flash-enabled ticks;
    // frame 2370 is the 2341st, past TITLE_SCREEN_DURATION.
    let mut app = TitleScreen::load(&store).expect("reload");
    for _ in 0..=2369 {
        idle_tick(&mut app);
    }
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Stay);
    idle_tick(&mut app);
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Advance);
    assert!(
        !app.frame().main.bgs[3].enabled,
        "the timeout turns the window off"
    );
}

#[test]
fn title_screen_press_advances_once_play_starts() {
    let Some(store) = store() else {
        return;
    };
    let mut app = TitleScreen::load(&store).expect("the pinned ROM's members load");

    // A press during the initial delay: the exits are not live yet.
    for _ in 0..5 {
        idle_tick(&mut app);
    }
    a_tick(&mut app);
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Stay);
    idle_tick(&mut app);

    // The same press once play has started exits immediately.
    for _ in 7..=50 {
        idle_tick(&mut app);
    }
    a_tick(&mut app);
    assert_eq!(app.next(), apricorn_core::app::ChainNext::Advance);
}

/// One inputless chain tick at global frame `i`.
fn chain_tick(chain: &mut BootChain, i: u32) -> &apricorn_core::frame::LogicalFrame {
    chain.tick(apricorn_core::Frame { index: i }, Input::default())
}

#[test]
fn boot_chain_intro_title_timeout_cycle() {
    let Some(store) = store() else {
        return;
    };
    let mut chain = boot_chain(store);

    // Frame 0: the intro's APPEAR — the copyright scroll.
    let frame = chain_tick(&mut chain, 0);
    assert_eq!(frame.main.bgs[0].scroll_y, 128);

    // Frames 1–220: the beat runs to its 110-frame logo hold; its
    // advancing tick (global 220) returns the beat's finishing
    // frame — the Game Freak logo — having handed the chain to the
    // title.
    let mut frame = chain_tick(&mut chain, 1);
    for i in 2..=220 {
        frame = chain_tick(&mut chain, i);
    }
    assert!(frame.sub.bgs[1].enabled, "the beat's finishing frame");

    // Frame 221: the title's SETUP — the 8bpp logo layer identifies
    // the new app, on with the rest of its planes.
    let frame = chain_tick(&mut chain, 221);
    assert_eq!(frame.sub.bgs[2].color_mode, ColorMode::Bpp8);
    assert!(frame.sub.bgs[2].enabled, "the title's SETUP planes");

    // The title's timeout (its local frame 2370, global 2591) cycles
    // the chain back to a fresh intro — global 2592 is the beat's
    // APPEAR again.
    let mut frame = chain_tick(&mut chain, 222);
    for i in 223..=2592 {
        frame = chain_tick(&mut chain, i);
    }
    assert_eq!(
        frame.main.bgs[0].scroll_y, 128,
        "the cycle restarts the copyright beat"
    );
}

// ===== The game-state machine (Phase 4, step 5) =========================

use apricorn_core::app::game::{
    Game, GameError, GameState, SAVE_STATUS_SLOT_DEGRADED, SAVE_STATUS_TOTAL_FAIL,
};
use apricorn_core::rtc::RtcDateTime;
use apricorn_core::save::{CARD_BACKUP_SIZE, FOOTER_SIZE, SLOT_SPECS, SLOT_STRIDE};

/// The retail `.sav` the card-probe test takes when present
/// (gitignored like the ROM — the `save.rs` policy).
const SAV_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg.sav");

fn sav() -> Option<Vec<u8>> {
    static SAV: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    SAV.get_or_init(|| match std::fs::read(SAV_PATH) {
        Ok(blob) => Some(blob),
        Err(_) => {
            eprintln!("skipping: {SAV_PATH} not found (supply your own retail save dump)");
            None
        }
    })
    .clone()
}

/// `FlashClobberChunkFooter` (save.rs's helper, same 0xFF fill the
/// original stamps before rewriting a sector): invalidates `slot`'s
/// `chunk` footer.
fn clobber_footer(blob: &mut [u8], slot: usize, chunk: usize) {
    let spec = SLOT_SPECS[chunk];
    let at = slot * SLOT_STRIDE + spec.offset as usize + spec.size as usize - FOOTER_SIZE;
    blob[at..at + FOOTER_SIZE].fill(0xFF);
}

/// One machine tick at global frame `i` with `input`.
fn game_tick(game: &mut Game, i: u32, input: Input) -> &apricorn_core::frame::LogicalFrame {
    game.tick(apricorn_core::Frame { index: i }, input)
}

/// The frozen clock every machine test pins: HG's US release date,
/// noon — a fixed RTC so every seed below is a constant.
fn pinned_rtc() -> RtcDateTime {
    RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0)
}

#[test]
fn game_walks_boot_to_bedroom() {
    let Some(store) = store() else {
        return;
    };
    let rtc = pinned_rtc();
    let mut game = Game::new(store, None, rtc).expect("a blank card boots a new game");
    assert_eq!(game.state(), GameState::IntroMovie);
    // NitroMain's InitializeMainRNG runs before the loop: the seed
    // reads the vblank counter at 0.
    assert_eq!(game.lcrng().seed(), rtc.rng_seed(0));

    // Frames 0–219: the copyright beat, still the intro overlay.
    for i in 0..=219 {
        game_tick(&mut game, i, Input::default());
    }
    assert_eq!(game.state(), GameState::IntroMovie);

    // Frame 220: the beat's advancing tick — the machine takes the
    // title overlay, returning the beat's finishing frame (the Game
    // Freak logo) across the swap.
    let frame = game_tick(&mut game, 220, Input::default()).clone();
    assert_eq!(game.state(), GameState::Title);
    assert!(frame.sub.bgs[1].enabled, "the beat's finishing frame");

    // Frame 221: the title's SETUP — the 8bpp logo layer identifies
    // the new scene, planes on.
    let frame = game_tick(&mut game, 221, Input::default());
    assert_eq!(frame.sub.bgs[2].color_mode, ColorMode::Bpp8);
    assert!(frame.sub.bgs[2].enabled);

    // Through the title's play start (its local frame 30), then an A
    // press at title-local 50 (global 271): the MENU exit registers
    // the save-check overlay.
    for i in 222..=270 {
        game_tick(&mut game, i, Input::default());
    }
    game_tick(
        &mut game,
        271,
        Input {
            keys: Keys(key::A),
            touch: None,
        },
    );
    assert_eq!(game.state(), GameState::CheckSave);

    // The save-check's SETUP (272): the warning BG on at char block
    // 3, the message window in place. With no status flags the app
    // walks straight to its EXIT — four ticks (272-275), the
    // advancing tick handing the machine to the menu.
    let frame = game_tick(&mut game, 272, Input::default()).clone();
    assert!(frame.main.bgs[0].enabled, "the save-check's BG");
    assert_eq!(frame.main.bgs[0].char_base, 3);
    assert_eq!(frame.main.windows.len(), 1);
    game_tick(&mut game, 273, Input::default());
    game_tick(&mut game, 274, Input::default());
    let frame = game_tick(&mut game, 275, Input::default()).clone();
    assert_eq!(game.state(), GameState::MainMenu);
    assert_eq!(frame.main.windows.len(), 0, "the EXIT pass freed the window");

    // The menu's SetupGraphics (276): three engine-A layers on at
    // pret's priorities, the screens still flipped.
    let frame = game_tick(&mut game, 276, Input::default()).clone();
    assert_eq!(frame.display, DisplaySelect::SubOnTop);
    assert!(frame.main.bgs[0].enabled && frame.main.bgs[1].enabled && frame.main.bgs[2].enabled);
    assert_eq!((frame.main.bgs[0].priority, frame.main.bgs[1].priority, frame.main.bgs[2].priority), (2, 1, 0));

    // 277: Prepare → ChooseApp; 278: the no-save probe picks NEW
    // GAME and begins the exit fade (the OUT-black quirk: the
    // register stays 0 until the last step snaps dark). No buttons
    // ever draw — the window list stays empty.
    game_tick(&mut game, 277, Input::default());
    game_tick(&mut game, 278, Input::default());
    assert_eq!(game.frame().main.windows.len(), 0, "no build without a save");
    // 279-283: the fade's six steps (283's snaps dark), 284: the
    // flag clears, 285: the poll takes the free state, 286: the free
    // pass — the menu's eleventh tick, advancing to NewGameInit with
    // NEW GAME picked.
    for i in 279..=283 {
        game_tick(&mut game, i, Input::default());
    }
    assert_eq!(
        game.frame().main.brightness.mode,
        apricorn_core::frame::BrightnessMode::Down,
        "the fade's end snap"
    );
    game_tick(&mut game, 284, Input::default());
    game_tick(&mut game, 285, Input::default());
    let frame = game_tick(&mut game, 286, Input::default()).clone();
    assert_eq!(game.state(), GameState::NewGameInit);
    assert_eq!(game.menu_exit(), Some(apricorn_core::app::main_menu::MainMenuExit::NewGame));
    assert_eq!(frame.display, DisplaySelect::MainOnTop, "FreeGraphics restores the order");
    assert_eq!(frame.main.windows.len(), 0);

    // Frame 287: NewGameInit's first tick — the ov36 init's
    // InitializeMainRNG, seeded with this frame's index — then Oak.
    game_tick(&mut game, 287, Input::default());
    assert_eq!(game.lcrng().seed(), rtc.rng_seed(287));
    assert_eq!(game.state(), GameState::OakSpeech);

    // Oak's speech runs 288–2928 (2641 ticks, its local k = global
    // − 288). The scripted walk: NO INFO at the button tutorial's
    // menu (DOWN×3 to cursor 2, A), the male gender (the resting
    // cursor, A), YES on both confirm menus (pad-init A then arm A
    // each — the multichoice's 20-frame hold runs the confirmation),
    // and "MATTS" through the naming seam. The kind-1 dialogs
    // page-wait (AUTO_SCROLL_OFF), so A is pulsed on alternate ticks
    // across each dialog's print range.
    let dialog_pulse = |i: u32| {
        i % 2 == 0
            && matches!(
                i,
                462..=628     // the noon greeting (msg 2)
                    | 651..=1114  // "Welcome to the world of Pokémon!"
                    | 1143..=1268 // "This world is inhabited…"
                    | 1361..=1738 // "We live alongside Pokémon…"
                    | 1817..=1930 // "First, tell me about yourself."
                    | 1931..=2056 // "Are you a boy or a girl?"
                    | 2074..=2122 // the gender confirm (msg 38)
                    | 2304..=2346 // the name confirm (msg 41)
                    | 2387..=2842 // "Your adventure unfolds…" (msg 43)
            )
    };
    let menu_press = |i: u32| match i {
        359 | 361 | 363 => Some(key::DOWN),
        365 | 2072 | 2124 | 2126 | 2348 | 2350 => Some(key::A),
        _ => None,
    };
    let name = GameString::from_units(&[311, 299, 318, 318, 317]); // "MATTS"
    for i in 288..=2928u32 {
        let keys = if dialog_pulse(i) || menu_press(i).is_some() {
            Keys(menu_press(i).unwrap_or(key::A))
        } else {
            Keys::IDLE
        };
        let frame = game_tick(&mut game, i, Input { keys, touch: None }).clone();
        match i {
            // 290: the tutorial menu's fade-in begun — its first step
            // (of six) already shows on the begin tick, done by 297.
            290 => {
                assert_eq!(frame.main.brightness.mode, BrightnessMode::Down);
                assert_eq!(frame.main.brightness.value, 13);
            }
            297 => assert_eq!(frame.main.brightness.value, 0),
            // 358: the tutorial menu built — the three-option
            // multichoice on the sub screen.
            358 => assert_eq!(frame.sub.windows.len(), 3),
            // 405: FadeOutTutorialMenu's free pass — the dialog
            // window gone.
            405 => assert_eq!(frame.main.windows.len(), 0),
            // 414: NoInfoNeededFadeIn — the touch-advance button's
            // window is the sub screen's one window.
            414 => assert_eq!(frame.sub.windows.len(), 1),
            // 634: ShowOak — the pic drawn and the brightness
            // transition started (the inline −16 write, then this
            // tick's tail step to −15; sixteen steps land at 0 by
            // 650).
            634 => {
                assert_eq!(frame.main.blend.effect, BlendEffect::BrightnessDown);
                assert_eq!(frame.main.blend.evy, 15);
            }
            650 => assert_eq!(frame.main.blend.evy, 0),
            // 2299: the naming launch's fade-out finished — the
            // overlay runs, the machine still Oak. Deliver the name
            // for the next tick to consume.
            2299 => {
                assert_eq!(game.state(), GameState::OakSpeech);
                game.deliver_naming_result(name.clone());
            }
            // 2303: PromptNameRestoreGraphicsAfter — the yes/no menu
            // back on the sub screen.
            2303 => assert_eq!(frame.sub.windows.len(), 2),
            _ => {}
        }
        if i < 2928 {
            assert_eq!(game.state(), GameState::OakSpeech);
        }
    }

    // 2928: the shrink anim's finish — the speech-over fade-out done,
    // the machine takes the post-Oak pass.
    assert_eq!(game.state(), GameState::AfterOakSpeech);

    // 2929: the post-Oak pass's first tick re-seeds again, then the
    // bedroom.
    game_tick(&mut game, 2929, Input::default());
    assert_eq!(game.lcrng().seed(), rtc.rng_seed(2929));
    assert_eq!(game.state(), GameState::Bedroom);

    // The bedroom is Phase 4's end state: it holds, a cleared frame
    // (the fieldsys renders the room in Phase 5).
    for i in 2930..=2939 {
        let frame = game_tick(&mut game, i, Input::default()).clone();
        assert_eq!(game.state(), GameState::Bedroom);
        assert!(
            !frame.main.bgs[0].enabled,
            "a cleared frame while the overworld is deferred"
        );
    }
}

#[test]
fn game_menu_builds_and_dialogs_with_a_clean_save() {
    let (Some(store), Some(blob)) = (store(), sav()) else {
        return;
    };
    let rtc = pinned_rtc();
    let mut game = Game::new(store, Some(&blob), rtc).expect("the retail card boots");

    // Boot to the menu: the beat (0-220), the title (221-271, an A at
    // 271), the save-check (272-275 — clean flags, same four ticks).
    for i in 0..=271 {
        let input = if i == 271 {
            Input {
                keys: Keys(key::A),
                touch: None,
            }
        } else {
            Input::default()
        };
        game_tick(&mut game, i, input);
    }
    assert_eq!(game.state(), GameState::CheckSave);
    for i in 272..=275 {
        game_tick(&mut game, i, Input::default());
    }
    assert_eq!(game.state(), GameState::MainMenu);
    assert!(game.save_file_exists(), "the menu's probe reads the card");

    // The menu with a save: SetupGraphics (276), Prepare (277), the
    // probe (278), then the build tick (279) — the five clean-save
    // buttons (CONTINUE, NEW GAME, POKEWALKER, WFC, WII SETTINGS)
    // with the focus on CONTINUE (FRAME1 tiles, bank 3), the
    // backdrop at MAIN_MENU_BACKGROUND_COLOR, and the fade-in begun
    // (the begin tick already shows step 1: down 16).
    game_tick(&mut game, 276, Input::default());
    game_tick(&mut game, 277, Input::default());
    game_tick(&mut game, 278, Input::default());
    let frame = game_tick(&mut game, 279, Input::default()).clone();
    assert_eq!(frame.main.windows.len(), 5, "the clean-save button set");
    assert_eq!(frame.main.backdrop, 0x7D8C);
    assert_eq!(
        frame.main.windows[0].frame,
        Some(apricorn_core::frame::WindowFrame {
            base_tile: 0x3EE,
            palette: 3
        }),
        "CONTINUE focused"
    );
    assert_eq!(
        frame.main.windows[1].frame,
        Some(apricorn_core::frame::WindowFrame {
            base_tile: 0x3F7,
            palette: 2
        }),
        "NEW GAME resting"
    );
    assert_eq!(frame.main.brightness.mode, apricorn_core::frame::BrightnessMode::Down);

    // The fade-in's six steps (279-284) then the free tick (285); the
    // poll tick (286) takes the interactive state, so 287 is the
    // first input tick. DOWN moves the focus to NEW GAME.
    for i in 280..=286 {
        game_tick(&mut game, i, Input::default());
    }
    let frame = game_tick(
        &mut game,
        287,
        Input {
            keys: Keys(key::DOWN),
            touch: None,
        },
    )
    .clone();
    assert_eq!(
        frame.main.windows[1].frame,
        Some(apricorn_core::frame::WindowFrame {
            base_tile: 0x3EE,
            palette: 3
        }),
        "NEW GAME focused after the DOWN"
    );
    assert_eq!(
        frame.main.windows[0].frame,
        Some(apricorn_core::frame::WindowFrame {
            base_tile: 0x3F7,
            palette: 2
        }),
        "CONTINUE resting"
    );

    // A on NEW GAME (288): the selection arms the new-game dialog —
    // no visible change on the arm tick; 289 the dialog machine's
    // case 15 consumes the arm edge; 290 loads the frame graphics;
    // 291 builds the three windows (the frameless centered warning
    // plus the two options) and flips the planes (button layers off,
    // dialog layer on); 292-321 the 30-frame countdown, its last tick
    // drawing the option focus.
    game_tick(
        &mut game,
        288,
        Input {
            keys: Keys(key::A),
            touch: None,
        },
    );
    assert_eq!(game.state(), GameState::MainMenu, "the dialog is in-scene");
    game_tick(&mut game, 289, Input::default());
    game_tick(&mut game, 290, Input::default());
    let frame = game_tick(&mut game, 291, Input::default()).clone();
    assert_eq!(frame.main.windows.len(), 8, "five buttons + three dialog windows");
    assert!(!frame.main.bgs[0].enabled, "MAIN_0 off behind the dialog");
    assert!(frame.main.bgs[1].enabled, "MAIN_1 carries the dialog");
    assert!(!frame.main.bgs[2].enabled, "MAIN_2 off behind the dialog");
    for i in 292..=320 {
        game_tick(&mut game, i, Input::default());
    }
    let frame = game_tick(&mut game, 321, Input::default()).clone();
    assert_eq!(
        frame.main.windows[6].frame,
        Some(apricorn_core::frame::WindowFrame {
            base_tile: 0x3EE,
            palette: 3
        }),
        "the countdown's end focuses Begin adventure"
    );

    // A on "Begin adventure" (322): the dialog closes — the three
    // windows truncated away, the button focus redrawn — then the
    // restore tick (323) flips the planes back and the dialog ticks
    // down; 324 the DialogDone state begins the exit fade;
    // 325-329 the six steps; 330 the flag clears; 331 the poll;
    // 332 the free pass — the machine leaves the menu for the
    // new-game init with NEW GAME picked.
    let frame = game_tick(
        &mut game,
        322,
        Input {
            keys: Keys(key::A),
            touch: None,
        },
    )
    .clone();
    assert_eq!(frame.main.windows.len(), 5, "the close truncated the dialog windows");
    game_tick(&mut game, 323, Input::default());
    assert!(game.frame().main.bgs[0].enabled, "the planes restored");
    assert!(!game.frame().main.bgs[1].enabled);
    for i in 324..=331 {
        game_tick(&mut game, i, Input::default());
    }
    let frame = game_tick(&mut game, 332, Input::default()).clone();
    assert_eq!(game.state(), GameState::NewGameInit);
    assert_eq!(game.menu_exit(), Some(apricorn_core::app::main_menu::MainMenuExit::NewGame));
    assert_eq!(frame.display, DisplaySelect::MainOnTop);
}

#[test]
fn game_title_timeout_cycles_back_to_a_fresh_intro() {
    let Some(store) = store() else {
        return;
    };
    let rtc = pinned_rtc();
    let mut game = Game::new(store, None, rtc).expect("boot");

    // The intro beat runs frames 0–220; the title takes over at 220
    // and its local frame 2370 (global 2591) is the timeout's
    // advancing tick, back to a fresh intro.
    for i in 0..=220 {
        game_tick(&mut game, i, Input::default());
    }
    assert_eq!(game.state(), GameState::Title);
    for i in 221..=2590 {
        game_tick(&mut game, i, Input::default());
    }
    assert_eq!(game.state(), GameState::Title, "still the title at 2590");
    game_tick(&mut game, 2591, Input::default());
    assert_eq!(game.state(), GameState::IntroMovie);

    // Global 2592: the fresh beat's APPEAR, and the cycle continues
    // to the title again (the fresh beat's advancing tick at 4812).
    let frame = game_tick(&mut game, 2592, Input::default());
    assert_eq!(frame.main.bgs[0].scroll_y, 128, "the cycle restarts");
    for i in 2593..=4812 {
        game_tick(&mut game, i, Input::default());
    }
    assert_eq!(game.state(), GameState::Title);

    // Nothing in the cycle re-seeds — only the ov36 inits do — so
    // the boot seed from counter 0 is still the LCRNG's state.
    assert_eq!(game.lcrng().seed(), rtc.rng_seed(0));
}

#[test]
fn game_card_probe_routes_the_status_flags() {
    let Some(store) = store() else {
        return;
    };
    let rtc = pinned_rtc();

    // No card data: the blank-card fresh start, no warning
    // (save.c's NOT_EXIST fallthrough sets no flags).
    let game = Game::new(store.clone(), None, rtc).expect("boot without a card");
    assert_eq!(game.save_status_flags(), 0);
    assert!(!game.save_file_exists());

    // A wiped card (all-erased flash) is the same fresh start.
    let blank = vec![0xFF; CARD_BACKUP_SIZE];
    let game = Game::new(store.clone(), Some(&blank), rtc).expect("a wiped card boots");
    assert_eq!(game.save_status_flags(), 0);
    assert!(!game.save_file_exists());

    // A blob that isn't a card backup at all: a caller error, the
    // one parse outcome with no screen to route to.
    let err = match Game::new(store.clone(), Some(&[0; 42]), rtc) {
        Err(err) => err,
        Ok(_) => panic!("a wrong-size blob must not boot"),
    };
    assert_eq!(err, GameError::NotACardBackup { got: 42 });

    // The retail card: loads whole, no flags — and the parse result
    // rides along for the menu's CONTINUE edge.
    let Some(blob) = sav() else {
        return;
    };
    let game = Game::new(store.clone(), Some(&blob), rtc).expect("the retail card boots");
    assert_eq!(game.save_status_flags(), 0);
    assert!(game.save_file_exists());
    assert!(game.save().is_some(), "the parsed save is carried");

    // One main chunk lost: the probe falls back, bit 0 — CheckSave's
    // "previous save file will be loaded" warning.
    let mut degraded = blob.clone();
    clobber_footer(&mut degraded, 0, 0);
    let game = Game::new(store.clone(), Some(&degraded), rtc).expect("the degraded card boots");
    assert_eq!(game.save_status_flags(), SAVE_STATUS_SLOT_DEGRADED);
    assert!(game.save_file_exists());

    // The same chunk kind lost in both slots: TOTAL_FAIL — bit 1,
    // the fresh region behind the "will be erased" warning.
    let mut corrupt = blob.clone();
    clobber_footer(&mut corrupt, 0, 0);
    clobber_footer(&mut corrupt, 1, 0);
    let game = Game::new(store.clone(), Some(&corrupt), rtc).expect("the corrupt card boots");
    assert_eq!(game.save_status_flags(), SAVE_STATUS_TOTAL_FAIL);
    assert!(!game.save_file_exists(), "the save is erased; a new game");
}
