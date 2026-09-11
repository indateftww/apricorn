//! Replays the reported intro screens against the user's retail ROM.
//! Set APRICORN_RENDER_OUT to save local PNGs for visual review.

use apricorn_core::app::{App, oak_speech::OakSpeech};
use apricorn_core::assets::AssetStore;
use apricorn_core::input::{Input, Keys, key};
use apricorn_core::rtc::RtcDateTime;
use apricorn_gfx::render;
use sha1::{Digest, Sha1};
use std::sync::Mutex;

/// The down arrow's three poses (index 3 repeats index 1), as the
/// SHA-1 of the 16×16 RGBA block at tiles `(left+width+1,
/// top+height-2)` of Oak's dialog. Pinned 2026-09-11 after a
/// pixel-for-pixel match (modulo the oracle's 5→8-bit color
/// expansion) against the retail ROM's held paragraph wait; see
/// `docs/game-flow.md`, "Rendering fixes".
const ARROW_POSE_GOLDEN: [&str; 4] = [
    "32e395d1da58d08dc9e474e7a3b9cef249fe2ba1",
    "d84ece097b4458075b65edc3fbd66c55ba0726b7",
    "99fa7e7383096868b5328ef4fa393d4cc0bd1e12",
    "d84ece097b4458075b65edc3fbd66c55ba0726b7",
];

/// The top LCD at frame 2071 of the gender-question tick schedule
/// below — Oak's "…are you a girl?" dialog fully printed with the
/// `{YESNO 0}` screen-focus icon (the two stacked screens, bottom
/// highlighted) in its reserved column, no arrow. Pinned after a
/// pixel comparison with the retail ROM's frame 3580 (the corpus
/// `new-game` case), the same as `apricorn-run`'s frame 2071 of
/// `scripts/engine-new-game.apin`.
const GENDER_QUESTION_TOP_GOLDEN: &str = "ab7eac6811acc36ca66a8b54f6af49ab8b09a471";

fn sha1_hex(screen: &apricorn_gfx::ScreenBuffer) -> String {
    format!("{:x}", Sha1::digest(screen.as_rgba().as_flattened()))
}

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

#[test]
fn gender_highlight_moves_pulses_and_survives_confirmation() {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds"));
    if !path.exists() {
        return;
    }
    let store = Mutex::new(AssetStore::open(path).unwrap());
    for gender in 0..2 {
        let mut oak = OakSpeech::load(&store, RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0)).unwrap();
        for index in 288..=2071 {
            let pulse = index % 2 == 0
                && matches!(index,
                462..=628 | 651..=1114 | 1143..=1268 | 1361..=1738 | 1817..=1930 | 1931..=2056);
            let keys = match index {
                359 | 361 | 363 => key::DOWN,
                365 => key::A,
                _ if pulse => key::A,
                _ => 0,
            };
            oak.tick(
                apricorn_core::Frame { index },
                Input {
                    keys: Keys(keys),
                    touch: None,
                },
            );
        }
        // The printed question with its screen-focus icon (the
        // retail-verified golden, both genders alike).
        let question = render(oak.frame(), &*store.lock().unwrap());
        // Oak's frame selects MAIN for the top LCD (`render`'s [0]).
        assert_eq!(
            sha1_hex(&question[0]),
            GENDER_QUESTION_TOP_GOLDEN,
            "frame 2071 top LCD"
        );
        save_screens("gender-question.png", &question);
        // The tutorial left pad mode active. Check both directions.
        for (index, keys, selected) in [
            (2072, key::LEFT, None),
            (2073, 0, Some(0)),
            (2074, key::RIGHT, None),
            (2075, 0, Some(1)),
            (2076, key::LEFT, None),
            (2077, 0, Some(0)),
            (2078, if gender == 1 { key::RIGHT } else { 0 }, None),
            (2079, 0, Some(gender)),
        ] {
            oak.tick(
                apricorn_core::Frame { index },
                Input {
                    keys: Keys(keys),
                    touch: None,
                },
            );
            if let Some(selected) = selected {
                let screens = render(oak.frame(), &*store.lock().unwrap());
                for panel in 0..2 {
                    let red = (24..174)
                        .flat_map(|y| (panel * 128..(panel + 1) * 128).map(move |x| (x, y)))
                        .filter(|&(x, y)| screens[1].pixel(x, y) == [255, 57, 57, 255])
                        .count();
                    assert_eq!(
                        red > 100,
                        panel == selected,
                        "selected={selected}, panel={panel}, red={red}"
                    );
                }
                save_screens(&format!("gender-focus-{selected}.png"), &screens);
            }
        }
        let mut fills = std::collections::BTreeSet::new();
        for index in 2080..2116 {
            oak.tick(apricorn_core::Frame { index }, Input::default());
            let overrides = &oak.frame().sub.palette_overrides;
            assert_eq!(
                overrides.len(),
                4,
                "animation must replace palette words without accumulating writes"
            );
            fills.insert(
                overrides
                    .iter()
                    .find(|&&(i, _)| i == 12 + gender as u16 * 2)
                    .unwrap()
                    .1,
            );
        }
        assert!(
            fills.len() > 10,
            "selected panel must pulse through its brightness cycle"
        );
        oak.tick(
            apricorn_core::Frame { index: 2116 },
            Input {
                keys: Keys(key::A),
                touch: None,
            },
        );
        assert_eq!(oak.player_gender(), gender as u8);
        let selected_screen = render(oak.frame(), &*store.lock().unwrap());
        let mut reached_confirmation = false;
        for index in 2117..2500 {
            oak.tick(
                apricorn_core::Frame { index },
                Input {
                    keys: Keys(if index % 2 == 0 { key::A } else { 0 }),
                    touch: None,
                },
            );
            if oak.frame().sub.windows.len() >= 2 && oak.frame().sub.bgs[2].enabled {
                let screens = render(oak.frame(), &*store.lock().unwrap());
                for bg in [0, 1, 2] {
                    assert_eq!(
                        oak.frame().sub.bgs[bg].scroll_x,
                        if gender == 0 { 136 } else { 0 },
                        "confirmation text, buttons and cursor belong opposite the portrait"
                    );
                }
                for y in 24..174 {
                    for x in gender * 128..(gender + 1) * 128 {
                        assert_eq!(
                            screens[1].pixel(x, y),
                            selected_screen[1].pixel(x, y),
                            "confirmation must preserve the selected portrait and its panel at {x},{y}"
                        );
                    }
                }
                save_screens(&format!("gender-confirm-{gender}.png"), &screens);
                reached_confirmation = true;
                break;
            }
        }
        assert!(reached_confirmation);
    }
}

#[test]
fn paragraph_pause_draws_all_arrow_frames_without_holes() {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds"));
    if !path.exists() {
        return;
    }
    let store = Mutex::new(AssetStore::open(path).expect("pinned retail ROM"));
    let mut oak = OakSpeech::load(&store, RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0)).unwrap();
    let mut seen = [false; 4];
    let mut first_seen = [None; 4];
    let mut pose_hashes: [String; 4] = Default::default();
    let mut arrow_rect = None;
    let mut restored_border = Vec::new();
    for index in 288..4000 {
        let keys = match index {
            359 | 361 | 363 => key::DOWN,
            365 => key::A,
            _ if (462..=628).contains(&index) && index % 2 == 0 => key::A,
            _ => 0,
        };
        oak.tick(
            apricorn_core::Frame { index },
            Input {
                keys: Keys(keys),
                touch: None,
            },
        );
        let frame = oak.frame();
        if index < 651 {
            continue;
        } // Wait until Oak is on screen.
        let Some(window) = frame.main.windows.iter().find(|w| w.arrow.is_some()) else {
            continue;
        };
        let arrow = window.arrow.unwrap();
        if seen[usize::from(arrow.index)] {
            continue;
        }
        first_seen[usize::from(arrow.index)] = Some(index);
        let border = window.frame.unwrap();
        assert_eq!(
            arrow.base_tile, border.base_tile,
            "the printer must address its dialogue frame"
        );
        let (x, y) = (
            usize::from(window.left + window.width + 1) * 8,
            usize::from(window.top + window.height - 2) * 8,
        );
        arrow_rect = Some((x, y));
        let mut plain = frame.clone();
        for window in &mut plain.main.windows {
            window.arrow = None;
        }
        let assets = store.lock().unwrap();
        let actual = render(frame, &*assets);
        let background = render(&plain, &*assets);
        let mut behind = plain.clone();
        behind.main.bgs[0].enabled = false;
        let behind = render(&behind, &*assets);
        let mut ink = 0;
        restored_border.clear();
        for py in y..y + 16 {
            for px in x..x + 16 {
                let pixel = actual[0].pixel(px, py);
                if background[0].pixel(px, py) != behind[0].pixel(px, py) {
                    assert_ne!(
                        pixel,
                        behind[0].pixel(px, py),
                        "arrow must not punch holes through the border"
                    );
                }
                restored_border.push(background[0].pixel(px, py));
                ink += usize::from(pixel != background[0].pixel(px, py));
            }
        }
        assert!(
            (1..96).contains(&ink),
            "arrow glyph over the intact border: {ink} changed pixels"
        );
        // The pose itself: the 2×2-tile block's SHA-1, pinned after
        // a pixel comparison with the retail ROM's held paragraph
        // wait (the oracle's frames 2124–2200 with the A pulses
        // removed; `docs/game-flow.md`, "Rendering fixes"). Index 3
        // re-draws index 1's tiles (`sDownArrowTileOffsets`).
        let block: Vec<u8> = (y..y + 16)
            .flat_map(|py| (x..x + 16).map(move |px| (px, py)))
            .flat_map(|(px, py)| actual[0].pixel(px, py))
            .collect();
        pose_hashes[usize::from(arrow.index)] = format!("{:x}", Sha1::digest(&block));
        if let Some(out) = std::env::var_os("APRICORN_RENDER_OUT") {
            let out = std::path::PathBuf::from(out);
            std::fs::create_dir_all(&out).unwrap();
            let mut encoder = png::Encoder::new(
                std::fs::File::create(out.join(format!("oak-arrow-{}.png", arrow.index))).unwrap(),
                256,
                192,
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(actual[0].as_rgba().as_flattened())
                .unwrap();
        }
        seen[usize::from(arrow.index)] = true;
        if seen.iter().all(|&seen| seen) {
            break;
        }
    }
    assert!(
        seen.iter().all(|&seen| seen),
        "the script must reach and hold a paragraph pause"
    );
    // The cycle's timing: the first wait frame draws pose 0
    // (`downArrowDelay` starts at 0), then `downArrowDelay = 8`
    // holds each pose for nine frames — the retail sequence is
    // 0×9, 1×9, 2×9, 1×9, 0… (oracle frames 2124–2200 of the held
    // wait: pose changes at 2133, 2142, 2151, 2160).
    assert_eq!(
        pose_hashes.each_ref().map(String::as_str),
        ARROW_POSE_GOLDEN,
        "arrow pose block hashes"
    );
    let first_seen = first_seen.map(|frame| frame.expect("every pose was seen"));
    assert_eq!(
        [
            first_seen[1] - first_seen[0],
            first_seen[2] - first_seen[1],
            first_seen[3] - first_seen[2],
        ],
        [9, 9, 9],
        "nine frames per pose: {first_seen:?}"
    );
    oak.tick(
        apricorn_core::Frame { index: 4000 },
        Input {
            keys: Keys(key::A),
            touch: None,
        },
    );
    let frame = oak.frame();
    assert!(
        frame.main.windows.iter().all(|w| w.arrow.is_none()),
        "advancing clears the arrow"
    );
    let (x, y) = arrow_rect.unwrap();
    let assets = store.lock().unwrap();
    let cleared = render(frame, &*assets);
    for py in y..y + 16 {
        for px in x..x + 16 {
            assert_eq!(
                cleared[0].pixel(px, py),
                restored_border[(py - y) * 16 + px - x],
                "clearing restores the border"
            );
        }
    }
}

#[test]
fn tutorial_dialogue_and_gender_portraits_render() {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds"));
    if !path.exists() {
        return;
    }
    let store = Mutex::new(AssetStore::open(path).expect("pinned retail ROM"));
    let mut oak = OakSpeech::load(&store, RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0)).unwrap();
    for index in 288..=2071 {
        let pulse = index % 2 == 0
            && matches!(index,
            462..=628 | 651..=1114 | 1143..=1268 | 1361..=1738 | 1817..=1930 | 1931..=2056);
        let keys = match index {
            359 | 361 | 363 => key::DOWN,
            365 => key::A,
            _ if pulse => key::A,
            _ => 0,
        };
        oak.tick(
            apricorn_core::Frame { index },
            Input {
                keys: Keys(keys),
                touch: None,
            },
        );
        if !matches!(index, 358 | 700 | 1400 | 2071) {
            continue;
        }
        let frame = oak.frame();
        let store = store.lock().unwrap();
        let screens = render(frame, &*store);
        if index == 358 {
            // A zero-filled tutorial window must reveal the BG below it,
            // regardless of the conspicuous color stored at palette index 0.
            let mut under = frame.clone();
            under.main.windows.clear();
            under.sub.windows.clear();
            let underneath = render(&under, &*store);
            let mut blank = frame.clone();
            for engine in [&mut blank.main, &mut blank.sub] {
                for window in &mut engine.windows {
                    window.glyphs.clear();
                }
            }
            let blank = render(&blank, &*store);
            assert_eq!(
                blank, underneath,
                "zero-filled tutorial windows are transparent"
            );
        }
        if index == 1400 || index == 2071 {
            let engine = if index == 1400 { 0 } else { 1 };
            let mut under = frame.clone();
            under.main.sprites.clear();
            under.sub.sprites.clear();
            let underneath = render(&under, &*store);
            let ranges = if engine == 0 {
                vec![40..128]
            } else {
                vec![20..110, 146..238]
            };
            for range in ranges {
                let changed = (24..174)
                    .flat_map(|y| range.clone().map(move |x| (x, y)))
                    .filter(|&(x, y)| screens[engine].pixel(x, y) != underneath[engine].pixel(x, y))
                    .count();
                assert!(
                    changed > 500,
                    "frame {index}: visible sprite pixels: {changed}"
                );
            }
        }
        if let Some(out) = std::env::var_os("APRICORN_RENDER_OUT") {
            let out = std::path::PathBuf::from(out);
            std::fs::create_dir_all(&out).unwrap();
            let file = std::fs::File::create(out.join(format!("oak-{index}.png"))).unwrap();
            let mut encoder = png::Encoder::new(file, 256, 384);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let pixels: Vec<u8> = screens
                .iter()
                .flat_map(|s| s.as_rgba().as_flattened().iter().copied())
                .collect();
            writer.write_image_data(&pixels).unwrap();
        }
    }
}

#[test]
fn pokeball_is_visible_before_the_opening_flash() {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds"));
    if !path.exists() {
        return;
    }
    let store = Mutex::new(AssetStore::open(path).unwrap());
    let mut oak = OakSpeech::load(&store, RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0)).unwrap();
    let mut ball_frames = 0;
    for index in 288..=1400 {
        let pulse = index % 2 == 0
            && matches!(index,
            462..=628 | 651..=1114 | 1143..=1268 | 1361..=1738);
        let keys = match index {
            359 | 361 | 363 => key::DOWN,
            365 => key::A,
            _ if pulse => key::A,
            _ => 0,
        };
        oak.tick(
            apricorn_core::Frame { index },
            Input {
                keys: Keys(keys),
                touch: None,
            },
        );
        let frame = oak.frame();
        if frame.main.sprites.iter().any(|s| s.sequence == 3) {
            ball_frames += 1;
            if ball_frames != 1 {
                continue;
            }
            let store = store.lock().unwrap();
            let screens = render(frame, &*store);
            let mut without_ball = frame.clone();
            without_ball.main.sprites.clear();
            let underneath = render(&without_ball, &*store);
            // NCER cell 2: a 16x16 ball at sprite (160,80) + (-9,-12).
            let changed = (68..84)
                .flat_map(|y| (151..167).map(move |x| (x, y)))
                .filter(|&(x, y)| screens[0].pixel(x, y) != underneath[0].pixel(x, y))
                .count();
            assert!(
                changed > 20,
                "ball vanished before the flash: {changed} pixels"
            );
            save_screens("oak-ball.png", &screens);
        }
    }
    assert!(
        ball_frames >= 30,
        "the ball must remain visible during the opening hold"
    );
}
