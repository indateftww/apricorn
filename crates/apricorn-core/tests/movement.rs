//! ROM-free Phase 5 movement tests: the per-frame trajectories and the
//! player's turn / walk / bump rules, driven through the public API
//! exactly as a field frame drives them (see `docs/field-movement.md`
//! for the timing table these numbers come from).

use apricorn_core::field::avatar::{
    AvatarMoveState, EncounterSteps, MoveOutcome, PlayerAvatar, PlayerMoveState, PlayerSaveData,
    PlayerState,
};
use apricorn_core::field::input::FieldInputContext;
use apricorn_core::field::map_object::{
    ATTR_NONE, Collision, Direction, MapObject, MovementCmd, MovementEntry, MovementError,
    MovementList, NoTerrain, TILE_FX32, VecFx32, flag,
};
use apricorn_core::input::{Keys, key};

/// An 8x8 floor whose border tiles carry the impassable attribute —
/// the bedroom's shape in miniature.
struct Floor;

impl Collision for Floor {
    fn attr(&self, x: i32, z: i32) -> u16 {
        if !(0..8).contains(&x) || !(0..8).contains(&z) {
            ATTR_NONE
        } else if x == 0 || z == 0 || x == 7 || z == 7 {
            0x8000
        } else {
            0
        }
    }
}

fn object_at(x: i32, z: i32, facing: Direction) -> MapObject {
    let mut obj = MapObject::create(x, z, facing, 0, 1);
    obj.set_flags(flag::IGNORE_HEIGHTS);
    obj.tick(&NoTerrain); // consume the creation START flag
    obj
}

/// The player at tile `(x, z)` facing `facing`, heights ignored (no
/// BDHC is loaded in these tests).
fn player_at(x: i32, z: i32, facing: Direction) -> PlayerAvatar {
    let mut avatar = PlayerAvatar::new(
        x,
        z,
        facing,
        PlayerState::Walking,
        0,
        0,
        PlayerSaveData::default(),
    );
    avatar.object.set_flags(flag::IGNORE_HEIGHTS);
    avatar
}

/// The spawn: (6, 6) facing south — one tile north of the `Floor`'s
/// south wall, so "walk south" bumps and "walk north" is free.
fn player() -> PlayerAvatar {
    player_at(6, 6, Direction::South)
}

fn frame(avatar: &mut PlayerAvatar, held: u16, terrain: &dyn Collision) -> MoveOutcome {
    avatar
        .run_frame(Keys::IDLE, Keys(held), terrain, FieldInputContext::default(), &mut 0)
        .outcome
}

// ===== trajectories ===================================================

#[test]
fn an_eight_frame_walk_moves_0x2000_per_frame() {
    let mut obj = object_at(3, 3, Direction::South);
    obj.set_held_movement(MovementCmd::WalkNormalEast).unwrap();
    let start = obj.position;
    let mut xs = Vec::new();
    for _ in 0..8 {
        obj.tick(&NoTerrain);
        xs.push(obj.position.x - start.x);
    }
    assert_eq!(
        xs,
        [0x2000, 0x4000, 0x6000, 0x8000, 0xA000, 0xC000, 0xE000, 0x10000]
    );
    assert_eq!(obj.position, VecFx32::from_tile(4, 0, 3));
    assert_eq!(obj.current, [4, 0, 3]);
    assert_eq!(obj.previous, [4, 0, 3]);
    assert_eq!(obj.current_facing, Direction::East);
    assert_eq!(obj.previous_facing, Direction::South);
    assert_eq!(obj.next_facing, Direction::East);
    assert!(obj.is_movement_idle());
}

#[test]
fn a_four_frame_run_moves_0x4000_per_frame() {
    let mut obj = object_at(3, 3, Direction::North);
    obj.set_held_movement(MovementCmd::RunNorth).unwrap();
    let start = obj.position;
    let mut zs = Vec::new();
    for _ in 0..4 {
        let report = obj.tick(&NoTerrain);
        zs.push(start.z - obj.position.z);
        assert_eq!(report.ended, zs.len() == 4);
    }
    assert_eq!(zs, [0x4000, 0x8000, 0xC000, 0x10000]);
    assert_eq!(obj.anim_group, 0, "the anim group resets when the move ends");
    assert_eq!(obj.current, [3, 0, 2]);
}

#[test]
fn every_linear_speed_lands_exactly_on_the_next_tile_centre() {
    for (cmd, frames) in [
        (MovementCmd::WalkSlowestSouth, 32),
        (MovementCmd::WalkSlowerSouth, 16),
        (MovementCmd::WalkNormalSouth, 8),
        (MovementCmd::WalkFasterSouth, 4),
        (MovementCmd::WalkFastestSouth, 2),
        (MovementCmd::WalkInstantSouth, 1),
        (MovementCmd::RunSouth, 4),
    ] {
        let mut obj = object_at(2, 2, Direction::South);
        obj.set_held_movement(cmd).unwrap();
        let speed = TILE_FX32 / frames;
        for tick in 1..=frames {
            obj.tick(&NoTerrain);
            assert_eq!(
                obj.position.z - VecFx32::from_tile(2, 0, 2).z,
                speed * tick,
                "{cmd:?} tick {tick}"
            );
            assert_eq!(obj.is_movement_idle(), tick == frames, "{cmd:?} tick {tick}");
        }
        assert_eq!(obj.position, VecFx32::from_tile(2, 0, 3), "{cmd:?}");
    }
}

#[test]
fn walk_on_spot_lasts_one_tick_longer_than_its_frame_count() {
    for (cmd, frames) in [
        (MovementCmd::WalkOnSpotSlowestWest, 32),
        (MovementCmd::WalkOnSpotSlowerWest, 16),
        (MovementCmd::WalkOnSpotNormalWest, 8),
        (MovementCmd::WalkOnSpotFasterWest, 4),
        (MovementCmd::WalkOnSpotFastestWest, 2),
    ] {
        let mut obj = object_at(2, 2, Direction::South);
        obj.set_held_movement(cmd).unwrap();
        let mut busy = 0;
        while !obj.is_movement_idle() {
            obj.tick(&NoTerrain);
            busy += 1;
            assert!(busy <= 40, "{cmd:?} never finished");
        }
        assert_eq!(busy, frames + 1, "{cmd:?}");
        assert_eq!(obj.current_facing, Direction::West);
        assert_eq!(obj.position, VecFx32::from_tile(2, 0, 2));
        assert_eq!(obj.current, [2, 0, 2]);
    }
}

// ===== the movement list and END ======================================

#[test]
fn movement_list_end_marker_finishes_the_machine() {
    let list = [
        MovementEntry {
            cmd: MovementCmd::WalkFasterNorth,
            count: 3,
        },
        MovementEntry {
            cmd: MovementCmd::WalkOnSpotFastestEast,
            count: 1,
        },
        MovementEntry {
            cmd: MovementCmd::End,
            count: 0,
        },
    ];
    let mut obj = object_at(4, 6, Direction::South);
    let mut machine = MovementList::new(&list);
    let mut frames = 0;
    while !machine.is_finished() {
        machine.tick(&mut obj).unwrap();
        obj.tick(&NoTerrain);
        frames += 1;
        assert!(frames < 100);
    }
    // Three 4-frame walks (12 ticks, each next command loading on the
    // tick the previous is seen finished: 0-3, 4-7, 8-11), the turn on
    // 12-14, the END check on 15.
    assert_eq!(frames, 16);
    assert_eq!(obj.current, [4, 0, 3]);
    assert_eq!(obj.position, VecFx32::from_tile(4, 0, 3));
    assert_eq!(obj.current_facing, Direction::East);
    // Ticking a finished list changes nothing.
    machine.tick(&mut obj).unwrap();
    assert!(machine.is_finished());
}

#[test]
fn end_and_none_cannot_be_held() {
    let mut obj = object_at(0, 0, Direction::North);
    assert_eq!(
        obj.set_held_movement(MovementCmd::End),
        Err(MovementError::NotHoldable(MovementCmd::End))
    );
    assert_eq!(
        obj.set_held_movement(MovementCmd::None),
        Err(MovementError::NotHoldable(MovementCmd::None))
    );
    let list = [MovementEntry {
        cmd: MovementCmd::None,
        count: 1,
    }];
    let mut machine = MovementList::new(&list);
    assert!(machine.tick(&mut obj).is_err());
    assert!(machine.is_finished());
}

// ===== the player's rules =============================================

#[test]
fn walking_starts_on_the_press_frame_when_already_facing() {
    let mut a = player_at(6, 6, Direction::North);
    assert_eq!(frame(&mut a, key::UP, &Floor), MoveOutcome::Walk(Direction::North));
    // The tile advanced at once; the position took its first 0x2000.
    assert_eq!(a.object.current, [6, 0, 5]);
    assert_eq!(a.object.position.z - VecFx32::from_tile(6, 0, 6).z, -0x2000);
}

#[test]
fn a_new_direction_costs_a_three_frame_turn() {
    let mut a = player();
    let outcomes: Vec<_> = (0..5).map(|_| frame(&mut a, key::LEFT, &Floor)).collect();
    assert_eq!(
        outcomes,
        [
            MoveOutcome::Turn(Direction::West),
            MoveOutcome::Busy,
            MoveOutcome::Busy,
            MoveOutcome::Walk(Direction::West),
            MoveOutcome::Busy,
        ]
    );
    assert_eq!(a.object.current, [5, 0, 6]);
    assert_eq!(a.object.position.x, VecFx32::from_tile(6, 0, 6).x - 2 * 0x2000);
}

#[test]
fn a_direction_tap_only_turns() {
    let mut a = player();
    assert_eq!(frame(&mut a, key::UP, &Floor), MoveOutcome::Turn(Direction::North));
    assert_eq!(a.facing(), Direction::North, "the facing flips on the first frame");
    assert_eq!(frame(&mut a, 0, &Floor), MoveOutcome::Busy);
    assert_eq!(frame(&mut a, 0, &Floor), MoveOutcome::Busy);
    assert_eq!(frame(&mut a, 0, &Floor), MoveOutcome::Idle);
    assert_eq!(a.object.movement_cmd, MovementCmd::FaceNorth);
    assert_eq!(a.move_state, AvatarMoveState::None);
    assert_eq!(a.object.current, [6, 0, 6]);
}

#[test]
fn turning_while_walking_is_free() {
    let mut a = player_at(6, 6, Direction::North);
    for _ in 0..8 {
        frame(&mut a, key::UP, &Floor);
    }
    assert_eq!(a.object.current, [6, 0, 5]);
    // Frame 8: the step is seen finished, the new direction walks at once.
    assert_eq!(frame(&mut a, key::LEFT, &Floor), MoveOutcome::Walk(Direction::West));
    assert_eq!(a.facing(), Direction::West);
    assert_eq!(a.object.current, [5, 0, 5]);
    // But after stopping, a different direction turns first again.
    for _ in 0..7 {
        frame(&mut a, key::LEFT, &Floor);
    }
    assert_eq!(frame(&mut a, 0, &Floor), MoveOutcome::Idle);
    assert_eq!(frame(&mut a, key::UP, &Floor), MoveOutcome::Turn(Direction::North));
}

#[test]
fn player_move_state_phases_per_frame() {
    let mut a = player_at(6, 6, Direction::North);
    let mut phases = Vec::new();
    let mut movement_flags = Vec::new();
    for _ in 0..17 {
        let report = a.run_frame(Keys::IDLE, Keys(key::UP), &Floor, FieldInputContext::default(), &mut 0);
        phases.push(a.player_move_state);
        movement_flags.push(report.input.movement);
    }
    use PlayerMoveState::{End, Moving, None as N, Start};
    assert_eq!(
        phases,
        [N, Start, Moving, Moving, Moving, Moving, Moving, Moving, End, Start, Moving, Moving, Moving, Moving, Moving, Moving, End]
    );
    // `movement` (a completed step) is raised on the END frames only.
    let steps: Vec<usize> = movement_flags.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i).collect();
    assert_eq!(steps, [8, 16]);
}

#[test]
fn a_wall_bumps_for_seventeen_frames_without_moving() {
    let mut a = player();
    // (6, 7) is the impassable south wall, one step away.
    let mut outcomes = Vec::new();
    for _ in 0..18 {
        outcomes.push(frame(&mut a, key::DOWN, &Floor));
    }
    // Frame 0 loads the bump (family 28: 16 + 1 ticks); frames 1-16
    // are refused; frame 17 sees it finished and bumps again.
    assert_eq!(outcomes[0], MoveOutcome::Bump(Direction::South));
    assert!(outcomes[1..17].iter().all(|o| *o == MoveOutcome::Busy));
    assert_eq!(outcomes[17], MoveOutcome::Bump(Direction::South));
    assert_eq!(a.object.current, [6, 0, 6]);
    assert_eq!(a.object.position, VecFx32::from_tile(6, 0, 6));
    assert_eq!(a.object.movement_cmd, MovementCmd::WalkOnSpotSlowerSouth);
    assert_eq!(a.object.next_facing, Direction::South);
    assert_eq!(a.steps_walked, 0);
}

#[test]
fn a_bump_is_interrupted_by_a_free_direction_but_not_a_blocked_one() {
    let mut a = player();
    frame(&mut a, key::DOWN, &Floor); // bump into z = 7
    frame(&mut a, key::DOWN, &Floor);
    // East (7, 6) is also wall: the bump continues.
    assert_eq!(frame(&mut a, key::RIGHT, &Floor), MoveOutcome::Busy);
    // West (5, 6) is free: the walk starts immediately.
    assert_eq!(frame(&mut a, key::LEFT, &Floor), MoveOutcome::Walk(Direction::West));
    assert_eq!(a.object.current, [5, 0, 6]);
}

#[test]
fn releasing_during_a_bump_lets_it_finish() {
    let mut a = player();
    frame(&mut a, key::DOWN, &Floor);
    for _ in 0..16 {
        assert_eq!(frame(&mut a, 0, &Floor), MoveOutcome::Busy);
    }
    assert_eq!(frame(&mut a, 0, &Floor), MoveOutcome::Idle);
}

#[test]
fn running_shoes_gate_b() {
    let mut a = player();
    assert_eq!(frame(&mut a, key::UP | key::B, &Floor), MoveOutcome::Turn(Direction::North));
    for _ in 0..2 {
        frame(&mut a, key::UP | key::B, &Floor);
    }
    assert_eq!(frame(&mut a, key::UP | key::B, &Floor), MoveOutcome::Walk(Direction::North));
    let mut a = player_at(6, 6, Direction::North);
    a.save.has_running_shoes = true;
    assert_eq!(frame(&mut a, key::UP, &Floor), MoveOutcome::Walk(Direction::North));
    assert_eq!(a.object.movement_cmd, MovementCmd::WalkNormalNorth, "shoes without B → walk");
    let mut a = player_at(6, 5, Direction::South);
    a.save.has_running_shoes = true;
    assert_eq!(frame(&mut a, key::DOWN | key::B, &Floor), MoveOutcome::Run(Direction::South));
    for _ in 0..3 {
        assert_eq!(frame(&mut a, key::DOWN | key::B, &Floor), MoveOutcome::Busy);
    }
    assert_eq!(a.object.position, VecFx32::from_tile(6, 0, 6));
    assert_eq!(frame(&mut a, key::DOWN | key::B, &Floor), MoveOutcome::Bump(Direction::South));
}

#[test]
fn running_shoes_lock_forces_running() {
    let mut a = player_at(6, 6, Direction::North);
    a.save.has_running_shoes = true;
    a.save.running_shoes_lock = true;
    assert_eq!(frame(&mut a, key::UP, &Floor), MoveOutcome::Run(Direction::North));
    assert_eq!(a.object.movement_cmd, MovementCmd::RunNorth);
}

#[test]
fn the_map_edge_blocks() {
    // Inside the walls a step is free ...
    let mut a = player_at(1, 1, Direction::South);
    assert_eq!(frame(&mut a, key::DOWN, &Floor), MoveOutcome::Walk(Direction::South));
    // ... but a tile the map does not cover reports ATTR_NONE and blocks.
    let mut a = player();
    assert_eq!(frame(&mut a, key::DOWN, &NoTerrain), MoveOutcome::Bump(Direction::South));
}

#[test]
fn encounter_step_counters_advance_on_move_ends() {
    let mut a = player();
    let mut steps = EncounterSteps::default();
    let mut ends = 0;
    for _ in 0..25 {
        let report = a.run_frame(Keys::IDLE, Keys(key::UP), &Floor, FieldInputContext::default(), &mut 0);
        if report.input.end_movement {
            steps.on_end_movement();
            ends += 1;
            if steps.encounter_allowed() {
                steps.update_turn_frame_counter(a.facing());
            }
        }
    }
    // The turn ends on frame 3, then steps end on frames 11 and 19.
    assert_eq!(ends, 3);
    assert_eq!(steps.encounter_inhibit_steps, 3);
    assert!(!steps.encounter_allowed());
}
