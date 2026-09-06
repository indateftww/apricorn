//! Integration tests: the boot apps against a retail HeartGold (US)
//! dump — frame-indexed behavior, the deterministic seam the future
//! harness reuses.
//!
//! `apricorn_core::app`'s two Phase 3 apps port pret's boot scenes
//! tick for tick; these tests pin the timeline at exact frame
//! indices: the copyright beat's scroll, hold, `ev` ramp, and Game
//! Freak handover; the title screen's setup, flash, and logo
//! fade-in; both exits; and the `boot_chain` cycle that carries the
//! title's timeout back to the intro. Every tick is a pure function
//! of the frame index and input, so these indices are a contract.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM dump).

use std::sync::{Arc, Mutex, OnceLock};

use apricorn_core::app::{
    App, BootChain, boot_chain, intro_copyright::IntroCopyright, title_screen::TitleScreen,
};
use apricorn_core::assets::AssetStore;
use apricorn_core::frame::{BlendEffect, ColorMode, DisplaySelect, plane};
use apricorn_core::input::{Input, Keys, key};

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
    assert!(frame.main.char_blocks[1].is_some());
    assert!(frame.main.char_blocks[2].is_some());
    assert!(frame.sub.char_blocks[1].is_some());
    assert!(frame.sub.char_blocks[4].is_some());
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
    assert!(frame.main.char_blocks.iter().all(|block| block.is_none()));

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
