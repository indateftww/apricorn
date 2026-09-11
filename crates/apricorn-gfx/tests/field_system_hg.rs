//! ROM-gated goldens of the live field through the whole pipeline
//! (`FieldSystem` → `FieldFrame` → BG0 → compositor), SHA-1 only: the
//! bedroom mid-step, house 1F on arrival, New Bark Town on arrival.
//! `APRICORN_RENDER_OUT` writes the review PNGs
//! (`field-system-*.png`); no pixels are committed.

use apricorn_core::assets::AssetStore;
use apricorn_core::field::map_object::Direction;
use apricorn_core::field::system::{FieldPhase, FieldSystem, Location};
use apricorn_core::input::{Input, Keys, key};
use sha1::{Digest, Sha1};
use std::path::Path;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Engine-A hashes: the bedroom four ticks into a step south (the
/// player mid-tile, the camera with him), house 1F at the first fully
/// faded-in tick after the stairs, New Bark Town at the first fully
/// faded-in tick after the front door.
///
/// Re-pinned with the BDHC height solver (`field::height`): the player
/// and the camera target sit on the land surface, so New Bark Town's
/// frame moved down by the 16-unit ground height (the mailbox's top
/// row 50 → 62, the oracle's 62; the player's outline rows 52-90, the
/// cap alone, → 64-98, the whole body, the oracle's 62-98 at frame
/// 7372) and 1F's faintly tilted floor (0.03 units) re-rasterized the
/// billboard's edges without moving a landmark (outline rows 30-97
/// before and after, the oracle's 30-99). The bedroom's plate is at 0
/// and its hash is unchanged.
const BEDROOM_MID_STEP: &str = "0867fcf6cf096f3b1aae90cf6b8ad00e9851a213";
const HOUSE_1F_ARRIVAL: &str = "99fe2418595825c63b2e6ed2ac774fd67fb69622";
const NEW_BARK_ARRIVAL: &str = "702d762517b6563eab476d612fc48ac2bd327982";

fn open_rom() -> Option<AssetStore> {
    let path = Path::new(ROM_PATH);
    if !path.exists() {
        eprintln!("skipping: no ROM at {}", path.display());
        return None;
    }
    Some(AssetStore::open(path).expect("the pinned ROM opens"))
}

fn sha1(pixels: &[[u8; 4]]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(pixels.as_flattened());
    format!("{:x}", hasher.finalize())
}

fn write_png(name: &str, pixels: &[[u8; 4]]) {
    let Ok(dir) = std::env::var("APRICORN_RENDER_OUT") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(Path::new(&dir).join(name)).unwrap();
    let mut enc = png::Encoder::new(file, 256, 192);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .unwrap()
        .write_image_data(pixels.as_flattened())
        .unwrap();
}

fn held(keys: u16) -> Input {
    Input {
        keys: Keys(keys),
        touch: None,
    }
}

fn render(field: &FieldSystem, store: &AssetStore, name: &str) -> String {
    let [main, sub] = apricorn_gfx::render(field.frame(), store);
    let pixels = main.as_rgba();
    write_png(&format!("{name}.png"), pixels);
    assert!(
        sub.as_rgba().iter().all(|p| p[..3] == [0, 0, 0]),
        "the touch screen stays black for now"
    );
    sha1(pixels)
}

fn run_until_running(field: &mut FieldSystem, store: &AssetStore) {
    while field.phase() != FieldPhase::Running {
        field.tick(Input::default(), store);
    }
}

#[test]
fn bedroom_mid_step_golden() {
    let Some(store) = open_rom() else {
        return;
    };
    let mut field = FieldSystem::new_game(&store, 0).unwrap();
    run_until_running(&mut field, &store);
    // Four ticks of DOWN: the position vector half a tile south; the
    // frame shows the previous tick's position (three steps) and the
    // walk frame.
    for _ in 0..4 {
        field.tick(held(key::DOWN), &store);
    }
    let hash = render(&field, &store, "field-system-bedroom-mid-step");
    assert_eq!(hash, BEDROOM_MID_STEP, "bedroom mid-step engine A hash");
}

#[test]
fn house_1f_arrival_golden() {
    let Some(store) = open_rom() else {
        return;
    };
    // The stairs' arrival: one tile into the wall west of 1F's stairs,
    // walking out — ticked through the transition's fade-in to the
    // first fully bright frame.
    let mut field = FieldSystem::new_game(&store, 0).unwrap();
    run_until_running(&mut field, &store);
    for _ in 0..27 {
        field.tick(held(key::LEFT), &store);
    }
    field.tick(Input::default(), &store);
    for _ in 0..19 {
        field.tick(held(key::UP), &store);
    }
    field.tick(Input::default(), &store);
    for _ in 0..4 {
        field.tick(held(key::LEFT), &store);
    }
    assert_eq!(field.phase(), FieldPhase::Transition);
    let mut ticks = 0;
    while field.frame().main.brightness != apricorn_core::frame::MasterBrightness::default()
        || field.scene().map_id != 63
    {
        field.tick(Input::default(), &store);
        ticks += 1;
        assert!(ticks < 200);
    }
    assert_eq!(field.scene().map_id, 63);
    assert_eq!(field.avatar().facing(), Direction::East);
    let hash = render(&field, &store, "field-system-house-1f-arrival");
    assert_eq!(hash, HOUSE_1F_ARRIVAL, "house 1F arrival engine A hash");
}

#[test]
fn new_bark_arrival_golden() {
    let Some(store) = open_rom() else {
        return;
    };
    let mut field = FieldSystem::enter(&store, Location::new(63, 1, 0, 0, Direction::East), 0)
        .unwrap();
    run_until_running(&mut field, &store);
    for _ in 0..60 {
        field.tick(held(key::DOWN), &store);
    }
    assert_eq!(field.phase(), FieldPhase::Transition);
    let mut ticks = 0;
    while field.frame().main.brightness != apricorn_core::frame::MasterBrightness::default()
        || field.scene().map_id != 60
    {
        field.tick(Input::default(), &store);
        ticks += 1;
        assert!(ticks < 200);
    }
    assert_eq!(field.scene().map_id, 60);
    let hash = render(&field, &store, "field-system-new-bark-arrival");
    // A few more frames for review: the walk off the door and a turn.
    for _ in 0..12 {
        field.tick(Input::default(), &store);
    }
    render(&field, &store, "field-system-new-bark-standing");
    for _ in 0..30 {
        field.tick(held(key::RIGHT), &store);
    }
    render(&field, &store, "field-system-new-bark-east");
    assert_eq!(hash, NEW_BARK_ARRIVAL, "New Bark Town arrival engine A hash");
}
