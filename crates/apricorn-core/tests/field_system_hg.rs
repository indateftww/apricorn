//! ROM-gated tests of the live field system (`docs/field-system.md`):
//! the new-game fade-in, per-tick movement trajectories, walls, the
//! turn-in-place rule, the stairs warp with its measured timing, the
//! house door into New Bark Town, and the camera target following the
//! player. Skips silently without `hg_usa.nds`.

use apricorn_core::assets::AssetStore;
use apricorn_core::field::avatar::MoveOutcome;
use apricorn_core::field::map_object::{Direction, VecFx32};
use apricorn_core::field::system::{
    FieldEvent, FieldPhase, FieldSystem, Location, Transition, TransitionKind, TransitionStage,
};
use apricorn_core::frame::{BrightnessMode, MasterBrightness};
use apricorn_core::input::{Input, Keys, key};
use std::path::Path;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

fn open_rom() -> Option<AssetStore> {
    let path = Path::new(ROM_PATH);
    if !path.exists() {
        eprintln!("skipping: no ROM at {}", path.display());
        return None;
    }
    Some(AssetStore::open(path).expect("the pinned ROM opens"))
}

fn held(keys: u16) -> Input {
    Input {
        keys: Keys(keys),
        touch: None,
    }
}

fn down(value: u8) -> MasterBrightness {
    MasterBrightness {
        mode: BrightnessMode::Down,
        value,
    }
}

/// A new game's field, ticked through the fade-in until movement is
/// allowed.
fn bedroom(store: &AssetStore) -> FieldSystem {
    let mut field = FieldSystem::new_game(store, 0).expect("the bedroom loads");
    while !field.movement_allowed() {
        field.tick(Input::default(), store);
    }
    field
}

fn tile(field: &FieldSystem) -> (i32, i32) {
    (field.avatar().x(), field.avatar().z())
}

#[test]
fn new_game_fades_in_over_six_steps_then_allows_movement() {
    let Some(store) = open_rom() else {
        return;
    };
    let mut field = FieldSystem::new_game(&store, 0).unwrap();
    assert_eq!(field.location(), Location::PLAYER_ROOM);
    assert_eq!(field.phase(), FieldPhase::FadeIn);
    assert_eq!(field.frame().main.brightness, down(16), "black at construction");
    assert_eq!(tile(&field), (6, 6));
    assert_eq!(field.avatar().facing(), Direction::South);
    // CallTask_FadeFromBlack: the 6 × 1 IN fade (asm/unk_02055244.s:137),
    // the flag clearing on the seventh update, the task unwinding on the
    // eighth tick and movement allowed from the ninth.
    let expected = [down(13), down(10), down(8), down(5), down(2), MasterBrightness::default()];
    for (i, want) in expected.iter().enumerate() {
        let frame = field.tick(Input::default(), &store);
        assert_eq!(frame.main.brightness, *want, "fade-in step {}", i + 1);
        assert_eq!(frame.sub.brightness, *want, "both screens fade");
        assert!(!field.movement_allowed());
    }
    field.tick(held(key::DOWN), &store);
    assert!(!field.movement_allowed(), "the flag clears this tick");
    field.tick(held(key::DOWN), &store);
    assert!(
        field.movement_allowed() && field.last_outcome().is_none(),
        "the task unwinds this tick, after FieldSystem_Control ran"
    );
    assert_eq!(tile(&field), (6, 6), "input is ignored while the task runs");
    field.tick(held(key::DOWN), &store);
    assert!(field.movement_allowed());
    assert_eq!(field.phase(), FieldPhase::Running);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Walk(Direction::South)));
    let frame = field.frame();
    let view = frame.main.field.as_ref().expect("the field is bound");
    assert!(frame.main.bgs[0].enabled && frame.main.bgs[0].priority == 1);
    assert_eq!(view.scene.map_id, 64);
    assert_eq!(view.objects.len(), 1, "the player is the one object");
    assert_eq!(view.objects[0].rect.2, 32);
    assert_eq!(view.objects[0].size_px, (32, 32));
}

#[test]
fn holding_down_moves_one_tile_in_eight_ticks_with_the_pinned_trajectory() {
    let Some(store) = open_rom() else {
        return;
    };
    let mut field = bedroom(&store);
    let start = VecFx32::from_tile(6, 0, 6);
    assert_eq!(field.avatar().object.position, start);
    // MOVEMENT_STEP_DOWN: 0x2000 per tick for eight ticks; the tile
    // coordinates advance at the first step, the position follows.
    let mut trajectory = Vec::new();
    for _ in 0..8 {
        field.tick(held(key::DOWN), &store);
        trajectory.push(field.avatar().object.position.z - start.z);
    }
    assert_eq!(
        trajectory,
        [0x2000, 0x4000, 0x6000, 0x8000, 0xA000, 0xC000, 0xE000, 0x10000]
    );
    assert_eq!(tile(&field), (6, 7));
    assert_eq!(field.avatar().object.position, VecFx32::from_tile(6, 0, 7));
    assert_eq!(field.avatar().steps_walked, 1);
    // The camera's fixed target is the position vector, every tick.
    let target = field.frame().main.field.as_ref().unwrap().camera_target;
    // The frame shows the objects where the previous tick's step left
    // them (the retail display latency): 7 steps in, one to go.
    assert_eq!(target, [start.x, 0, start.z + 0xE000]);
    field.tick(Input::default(), &store);
    let target = field.frame().main.field.as_ref().unwrap().camera_target;
    assert_eq!(target, [start.x, 0, start.z + 0x10000]);
    assert_eq!(
        field.frame().main.field.as_ref().unwrap().objects[0].world_pos[0],
        start.x
    );
}

#[test]
fn holding_right_turns_first_then_walks_one_tile() {
    let Some(store) = open_rom() else {
        return;
    };
    let mut field = bedroom(&store);
    let start = VecFx32::from_tile(6, 0, 6);
    // Facing south, RIGHT: the turn (family 40, three ticks) then the
    // step — eleven ticks to the next tile east.
    field.tick(held(key::RIGHT), &store);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Turn(Direction::East)));
    assert_eq!(field.avatar().facing(), Direction::East);
    for _ in 0..2 {
        field.tick(held(key::RIGHT), &store);
        assert_eq!(field.last_outcome(), Some(MoveOutcome::Busy));
        assert_eq!(field.avatar().object.position, start);
    }
    let mut xs = Vec::new();
    for _ in 0..8 {
        field.tick(held(key::RIGHT), &store);
        xs.push(field.avatar().object.position.x - start.x);
    }
    assert_eq!(xs, [0x2000, 0x4000, 0x6000, 0x8000, 0xA000, 0xC000, 0xE000, 0x10000]);
    assert_eq!(tile(&field), (7, 6));
}

#[test]
fn a_tap_turns_in_place_without_moving() {
    let Some(store) = open_rom() else {
        return;
    };
    let mut field = bedroom(&store);
    field.tick(held(key::LEFT), &store);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Turn(Direction::West)));
    field.tick(Input::default(), &store);
    field.tick(Input::default(), &store);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Busy));
    field.tick(Input::default(), &store);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Idle));
    assert_eq!(field.avatar().facing(), Direction::West);
    assert_eq!(tile(&field), (6, 6));
    assert_eq!(field.avatar().object.position, VecFx32::from_tile(6, 0, 6));
    // The sprite shows the west animation's standing frame (hero.11:
    // strip row 10) after the turn's snap.
    assert_eq!(field.sprite().facing, Direction::West);
    assert_eq!(field.sprite().texture_index(), 10);
}

#[test]
fn walls_block_and_the_bedroom_has_83_of_them() {
    let Some(store) = open_rom() else {
        return;
    };
    let mut field = bedroom(&store);
    let attrs = field.scene().terrain.cell(0, 0).expect("the bedroom's one cell");
    let walls = attrs.iter().filter(|&&a| a == 0x8000).count();
    assert_eq!(walls, 83);
    assert_eq!(attrs.iter().filter(|&&a| a == 0x8086).count(), 4);
    assert_eq!(attrs.iter().filter(|&&a| a == 0x005F).count(), 1);
    // North: (6, 5) is floor, (6, 4) a wall — one step, then the bump
    // (family 28, seventeen ticks) without moving.
    field.tick(held(key::UP), &store);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Turn(Direction::North)));
    field.tick(held(key::UP), &store);
    field.tick(held(key::UP), &store);
    for _ in 0..8 {
        field.tick(held(key::UP), &store);
    }
    assert_eq!(tile(&field), (6, 5));
    assert_eq!(field.avatar().object.position, VecFx32::from_tile(6, 0, 5));
    field.tick(held(key::UP), &store);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Bump(Direction::North)));
    for _ in 0..16 {
        field.tick(held(key::UP), &store);
        assert_eq!(field.last_outcome(), Some(MoveOutcome::Busy));
    }
    field.tick(held(key::UP), &store);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Bump(Direction::North)));
    assert_eq!(tile(&field), (6, 5));
    assert_eq!(field.avatar().object.position, VecFx32::from_tile(6, 0, 5));
}

/// Walks the bedroom's spawn to the stairs tile (3, 4) facing north:
/// LEFT to x = 3, UP to z = 4.
fn walk_to_the_stairs(field: &mut FieldSystem, store: &AssetStore) {
    // Turn (3) + three steps (24).
    for _ in 0..27 {
        field.tick(held(key::LEFT), store);
    }
    field.tick(Input::default(), store);
    assert_eq!(tile(field), (3, 6));
    // Turn (3) + two steps (16).
    for _ in 0..19 {
        field.tick(held(key::UP), store);
    }
    field.tick(Input::default(), store);
    assert_eq!(tile(field), (3, 4));
    assert_eq!(field.scene().terrain.behavior(3, 4), 0x5F);
}

#[test]
fn the_stairs_warp_to_the_house_with_the_measured_timing() {
    let Some(store) = open_rom() else {
        return;
    };
    let mut field = bedroom(&store);
    walk_to_the_stairs(&mut field, &store);
    // Facing north on the stairs: LEFT turns west (three ticks); the
    // standing tick after the turn digests the held direction, and
    // FieldSystem_CheckMapTransition starts transition 3 (T).
    field.tick(held(key::LEFT), &store);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Turn(Direction::West)));
    field.tick(held(key::LEFT), &store);
    field.tick(held(key::LEFT), &store);
    assert!(field.movement_allowed());
    field.tick(held(key::LEFT), &store);
    assert_eq!(field.last_event(), Some(FieldEvent::Warp(TransitionKind::Stairs)));
    assert_eq!(field.phase(), FieldPhase::Transition);
    let t = field.transition().expect("a transition runs");
    assert_eq!(t.kind, TransitionKind::Stairs);
    assert_eq!(t.destination, Location::new(63, 1, 3, 4, Direction::West));
    assert_eq!(field.entrance(), Location::new(64, 0, 3, 4, Direction::West));
    let stairs = VecFx32::from_tile(3, 0, 4);
    assert_eq!(field.avatar().object.position, stairs);

    // T+1..T+3: the task's prologue — nothing moves.
    for _ in 1..=3 {
        field.tick(held(key::LEFT), &store);
        assert_eq!(field.avatar().object.position, stairs);
        assert_eq!(field.frame().main.brightness, MasterBrightness::default());
    }
    // T+4..T+19: the exit walk, WalkSlowerWest — 0x1000 per tick into
    // the wall behind the stairs (no collision on held movements).
    for i in 1..=16 {
        field.tick(held(key::LEFT), &store);
        assert_eq!(field.avatar().object.position.x, stairs.x - 0x1000 * i, "walk tick {i}");
    }
    assert_eq!(tile(&field), (2, 4));
    // T+20: done; T+21..T+26: the fade-out's six steps.
    field.tick(Input::default(), &store);
    assert_eq!(field.frame().main.brightness, MasterBrightness::default());
    let out = [down(2), down(5), down(7), down(10), down(13), down(16)];
    for (i, want) in out.iter().enumerate() {
        field.tick(Input::default(), &store);
        assert_eq!(field.frame().main.brightness, *want, "fade-out step {}", i + 1);
        assert_eq!(field.scene().map_id, 64, "still the bedroom while fading");
    }
    // Black: the destination loads at T+28; the stairs placement puts
    // the player one tile into the wall west of 1F's stairs, facing east.
    for _ in 27..=Transition::WALK_EXIT_FADE_TICK + Transition::LOAD_AFTER_FADE_TICKS {
        field.tick(Input::default(), &store);
        assert_eq!(field.frame().main.brightness, down(16));
    }
    assert_eq!(field.scene().map_id, 63);
    assert_eq!(field.transition().unwrap().stage, TransitionStage::Black);
    assert_eq!(field.location(), Location::new(63, 1, 3, 3, Direction::East));
    assert_eq!(field.previous(), Location::PLAYER_ROOM);
    assert_eq!(tile(&field), (2, 3));
    assert_eq!(field.avatar().facing(), Direction::East);
    assert_eq!(field.scene().terrain.behavior(3, 3), 0x5F, "1F's stairs tile");
    let fade_in = Transition::WALK_EXIT_FADE_TICK + Transition::FADE_TO_FADE_TICKS;
    let mut ticks = Transition::WALK_EXIT_FADE_TICK + Transition::LOAD_AFTER_FADE_TICKS;
    while ticks + 1 < fade_in {
        field.tick(Input::default(), &store);
        assert_eq!(field.frame().main.brightness, down(16));
        ticks += 1;
    }
    // T+65: the fade-in begins and the arrival walk (WalkSlowerEast)
    // starts in the same tick; the frame still shows the previous
    // position (display latency) and the fade's first step.
    let wall = VecFx32::from_tile(2, 0, 3);
    field.tick(Input::default(), &store);
    assert_eq!(field.frame().main.brightness, down(13));
    assert_eq!(field.frame().main.field.as_ref().unwrap().camera_target, [wall.x, 0, wall.z]);
    assert_eq!(field.avatar().object.position.x, wall.x + 0x1000);
    assert_eq!(field.transition().unwrap().stage, TransitionStage::Enter);
    let mut brightness = vec![];
    for i in 2..=16 {
        field.tick(Input::default(), &store);
        brightness.push(field.frame().main.brightness);
        assert_eq!(field.avatar().object.position.x, wall.x + 0x1000 * i);
    }
    assert_eq!(&brightness[..5], &[down(10), down(8), down(5), down(2), MasterBrightness::default()]);
    assert_eq!(tile(&field), (3, 3));
    assert_eq!(field.avatar().object.position, VecFx32::from_tile(3, 0, 3));
    assert!(!field.movement_allowed());
    field.tick(Input::default(), &store);
    assert!(!field.movement_allowed(), "the routine sees the walk's end");
    field.tick(Input::default(), &store);
    assert_eq!(field.phase(), FieldPhase::Running, "the task unwinds at T+82");
    assert!(field.transition().is_none());
    field.tick(Input::default(), &store);
    assert_eq!(field.last_outcome(), Some(MoveOutcome::Idle), "movement from T+83");
    assert_eq!(field.encounter_steps().encounter_inhibit_steps, 0);
}

/// From 1F's stairs: south down the hall to the doormat (3, 10) and out.
#[test]
fn the_house_door_leads_to_new_bark_town() {
    let Some(store) = open_rom() else {
        return;
    };
    let mut field = FieldSystem::enter(&store, Location::new(63, 1, 0, 0, Direction::East), 0)
        .expect("the house loads");
    assert_eq!(field.location(), Location::new(63, 1, 3, 3, Direction::East));
    while !field.movement_allowed() {
        field.tick(Input::default(), &store);
    }
    // Turn (3) + seven steps (56) to (3, 10); the step's END tick with
    // DOWN still held digests the warp entrance south.
    for _ in 0..59 {
        field.tick(held(key::DOWN), &store);
    }
    assert_eq!(tile(&field), (3, 10));
    assert_eq!(field.scene().terrain.behavior(3, 10), 101, "WARP_ENTRANCE_SOUTH");
    field.tick(held(key::DOWN), &store);
    assert_eq!(field.last_event(), Some(FieldEvent::Warp(TransitionKind::Entrance)));
    let t = field.transition().unwrap();
    assert_eq!(t.destination, Location::new(60, 1, 3, 10, Direction::South));
    assert_eq!(field.entrance(), Location::new(63, 0, 3, 10, Direction::South));
    // The entrance exit fades at once (T+3) — no walk.
    let mat = VecFx32::from_tile(3, 0, 10);
    for _ in 1..=3 {
        field.tick(held(key::DOWN), &store);
    }
    assert_eq!(field.frame().main.brightness, down(2));
    assert_eq!(field.avatar().object.position, mat);
    let mut ticks = 3;
    while field.transition().is_some() {
        field.tick(held(key::DOWN), &store);
        ticks += 1;
        assert!(ticks < 200, "the transition ends");
    }
    assert_eq!(ticks, t.end_tick());
    assert_eq!(field.scene().map_id, 60);
    assert_eq!(field.location(), Location::new(60, 1, 695, 396, Direction::South));
    // The arrival walk carried the player one tile south off the door.
    assert_eq!(tile(&field), (695, 397));
    assert_eq!(field.avatar().object.position, VecFx32::from_tile(695, 0, 397));
    assert!(field.scene().cells.len() >= 4, "the town's cell window");
    assert!(!field.scene().props.is_empty());
    // Outdoors: preset 0, perspective.
    assert_eq!(field.camera().perspective_type, 0);
    assert_eq!(field.frame().main.field.as_ref().unwrap().camera, field.camera());
    // Walk on: DOWN keeps walking south; the camera target follows.
    let mut last = field.avatar().object.position;
    for _ in 0..8 {
        field.tick(held(key::DOWN), &store);
        let now = field.avatar().object.position;
        assert_eq!(now.z, last.z + 0x2000);
        last = now;
    }
    assert_eq!(tile(&field), (695, 398));
    field.tick(Input::default(), &store);
    let view = field.frame().main.field.as_ref().unwrap();
    assert_eq!(view.camera_target, [last.x, last.y, last.z]);
    assert_eq!(view.objects[0].world_pos[0], last.x);
}
