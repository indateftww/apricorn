//! Phase 5: the field movement machine (`apricorn_core::field`) locked
//! to the *original* ARM9 step functions, tick by tick, through
//! arm-runner.
//!
//! The map-object step machine is asm in pret (`asm/unk_02062108.s`),
//! so a synthesized `LocalMapObject` (`include/map_object.h:48`, 0x12C
//! bytes) is placed in scratch RAM and driven exactly as the game's
//! SysTask drives it: `MapObject_SetHeldMovement` once, then
//! `sub_02062400` per frame, which dispatches through the ROM's own
//! `gMovementCmdTable`. After every frame the object's position vector,
//! tile coordinates, facing set, anim group, command/step, flags and
//! 16-byte work buffer must equal the Rust `MapObject`'s. Every ported
//! command family (000–043, 084–091) is run from a fresh object.
//!
//! The avatar-side leaves that are pure functions of their inputs get
//! the same treatment over exhaustive input grids: the pad-to-direction
//! resolver (`sub_0205DDD4`), the turn-versus-walk decision
//! (`sub_0205D450`), the `playerMoveState` derivation
//! (`PlayerAvatar_UpdateMovement`), the family/direction table lookups
//! (`sub_0206234C` / `sub_02062390`), and the bump predicate
//! (`sub_0205DE64`). `PlayerAvatar_MoveControl` itself walks the
//! `FieldSystem` (terrain callbacks, object manager) and is not called
//! here; its decision tree is covered by the ROM-free tests.
//!
//! Runs only when `hg_usa.nds` sits at the repo root (same policy as
//! `rng_hg.rs`; CI has no ROM and skips silently). Every load
//! re-verifies the pin table, so a wrong dump fails before any call.

use apricorn_core::field::avatar::{
    AvatarMoveState, PlayerAvatar, PlayerMoveState, PlayerSaveData, PlayerState, avatar_flag,
};
use apricorn_core::field::input::direction_from_keys;
use apricorn_core::field::map_object::{
    Direction, MOVEMENT_CMD_COUNT, MapObject, MovementCmd, StepResult, VecFx32, flag,
};
use apricorn_core::input::Keys;
use apricorn_harness::arm::mem::SCRATCH_STACK_TOP;
use apricorn_harness::arm::retail::RetailArm9;
use apricorn_harness::pins::{PinMode, PinTable};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Scratch above the loaded image (ends 0x0211_1EF8), below the stack
/// (0x0230_0000): the synthesized `LocalMapObject`.
const OBJECT: u32 = 0x0220_0000;
/// The synthesized `PlayerAvatar` (0x40 bytes).
const AVATAR: u32 = 0x0220_1000;
/// `sizeof(LocalMapObject)`.
const OBJECT_SIZE: u32 = 0x12C;
/// A fifth argument lives at `[SCRATCH_STACK_TOP]` — `prepare_call`
/// points r13 there and the callee reads it at `[sp + 0x18]` after its
/// six-register push.
const STACK_TOP: u32 = SCRATCH_STACK_TOP;

// LocalMapObject offsets (include/map_object.h:48, annotated).
const OFF_FLAGS: u32 = 0x00;
const OFF_FLAGS2: u32 = 0x04;
const OFF_ID: u32 = 0x08;
const OFF_SPRITE: u32 = 0x10;
const OFF_MOVEMENT: u32 = 0x14;
const OFF_INITIAL_FACING: u32 = 0x24;
const OFF_CURRENT_FACING: u32 = 0x28;
const OFF_NEXT_FACING: u32 = 0x2C;
const OFF_PREVIOUS_FACING: u32 = 0x30;
const OFF_NEXT_FACING_BACKUP: u32 = 0x34;
const OFF_XRANGE: u32 = 0x44;
const OFF_YRANGE: u32 = 0x48;
const OFF_INITIAL: u32 = 0x4C;
const OFF_PREVIOUS: u32 = 0x58;
const OFF_CURRENT: u32 = 0x64;
const OFF_POSITION: u32 = 0x70;
const OFF_ANIM: u32 = 0xA0;
const OFF_CMD: u32 = 0xA4;
const OFF_STEP: u32 = 0xA8;
const OFF_UNKAC: u32 = 0xAC;
const OFF_UNKAE: u32 = 0xAE;
const OFF_UNKC8: u32 = 0xC8;
const OFF_SCRATCH: u32 = 0xF8;

// PlayerAvatar offsets (include/player_avatar.h:26).
const AV_FLAGS: u32 = 0x00;
const AV_UNK8: u32 = 0x08;
const AV_UNKC: u32 = 0x0C;
const AV_MOVE_STATE: u32 = 0x10;
const AV_PLAYER_MOVE_STATE: u32 = 0x14;
const AV_STATE: u32 = 0x18;
const AV_GENDER: u32 = 0x1C;
const AV_UNK24: u32 = 0x24;
const AV_UNK28: u32 = 0x28;
const AV_UNK2C: u32 = 0x2C;
const AV_MAP_OBJECT: u32 = 0x30;
const AVATAR_SIZE: u32 = 0x40;

fn load() -> Option<RetailArm9> {
    let data = match std::fs::read(ROM_PATH) {
        Ok(data) => data,
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            return None;
        }
    };
    let arm9 = match RetailArm9::load(&data) {
        Ok(arm9) => arm9,
        Err(e) => panic!("retail ROM failed to load: {e}"),
    };
    Some(arm9)
}

/// The entry address of a pinned Thumb function (bit 0 set).
fn entry(name: &str) -> u32 {
    let table = PinTable::arm9();
    let pin = table.get(name).unwrap_or_else(|| panic!("pin {name}"));
    assert_ne!(pin.mode, PinMode::Data, "{name} is a data pin");
    pin.address | u32::from(pin.mode == PinMode::Thumb)
}

/// Calls the pinned function `name` with `args`, returning r0.
fn call(arm9: &mut RetailArm9, name: &str, args: &[u32]) -> u32 {
    let cpu = arm9.cpu();
    cpu.prepare_call(entry(name), args);
    cpu.run_default()
        .unwrap_or_else(|e| panic!("{name} faulted: {e}"))
        .r0
}

fn w32(arm9: &mut RetailArm9, addr: u32, value: u32) {
    arm9.cpu().mem_mut().write32(addr, value).expect("scratch write");
}

fn r32(arm9: &mut RetailArm9, addr: u32) -> u32 {
    arm9.cpu().mem().read32(addr).expect("scratch read")
}

fn zero(arm9: &mut RetailArm9, addr: u32, len: u32) {
    for off in (0..len).step_by(4) {
        w32(arm9, addr + off, 0);
    }
}

/// Writes the Rust object into the ARM `LocalMapObject` slot, with the
/// sprite callback (`unkC8`, run by `sub_0205F484` when a move ends)
/// pointed at the no-op `MovementScriptMachineSub_Done`.
fn write_object(arm9: &mut RetailArm9, obj: &MapObject) {
    zero(arm9, OBJECT, OBJECT_SIZE);
    w32(arm9, OBJECT + OFF_FLAGS, obj.flags);
    w32(arm9, OBJECT + OFF_FLAGS2, obj.flags2);
    w32(arm9, OBJECT + OFF_ID, obj.id);
    w32(arm9, OBJECT + OFF_SPRITE, obj.sprite_id);
    w32(arm9, OBJECT + OFF_MOVEMENT, obj.movement);
    w32(arm9, OBJECT + OFF_INITIAL_FACING, obj.initial_facing.index());
    w32(arm9, OBJECT + OFF_CURRENT_FACING, obj.current_facing.index());
    w32(arm9, OBJECT + OFF_NEXT_FACING, obj.next_facing.index());
    w32(arm9, OBJECT + OFF_PREVIOUS_FACING, obj.previous_facing.index());
    w32(arm9, OBJECT + OFF_NEXT_FACING_BACKUP, obj.next_facing_backup.index());
    w32(arm9, OBJECT + OFF_XRANGE, obj.x_range as u32);
    w32(arm9, OBJECT + OFF_YRANGE, obj.y_range as u32);
    for axis in 0..3 {
        let a = axis as u32 * 4;
        w32(arm9, OBJECT + OFF_INITIAL + a, obj.initial[axis] as u32);
        w32(arm9, OBJECT + OFF_PREVIOUS + a, obj.previous[axis] as u32);
        w32(arm9, OBJECT + OFF_CURRENT + a, obj.current[axis] as u32);
    }
    w32(arm9, OBJECT + OFF_POSITION, obj.position.x as u32);
    w32(arm9, OBJECT + OFF_POSITION + 4, obj.position.y as u32);
    w32(arm9, OBJECT + OFF_POSITION + 8, obj.position.z as u32);
    w32(arm9, OBJECT + OFF_ANIM, obj.anim_group);
    w32(arm9, OBJECT + OFF_CMD, u32::from(obj.movement_cmd.as_u8()));
    w32(arm9, OBJECT + OFF_STEP, obj.movement_step);
    let behaviors = u32::from(obj.current_behavior) | (u32::from(obj.previous_behavior) << 16);
    w32(arm9, OBJECT + OFF_UNKAC, behaviors);
    w32(arm9, OBJECT + OFF_UNKC8, entry("MovementScriptMachineSub_Done"));
    for (i, chunk) in obj.scratch.chunks(4).enumerate() {
        let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        w32(arm9, OBJECT + OFF_SCRATCH + i as u32 * 4, word);
    }
    let _ = OFF_UNKAE;
}

/// The movement-visible state of a `LocalMapObject`, read from either
/// machine.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    flags: u32,
    facing: [u32; 4],
    previous: [i32; 3],
    current: [i32; 3],
    position: VecFx32,
    anim: u32,
    cmd: u32,
    step: u32,
    scratch: [u8; 16],
}

fn snapshot_arm(arm9: &mut RetailArm9) -> Snapshot {
    let v3 = |arm9: &mut RetailArm9, off: u32| {
        [
            r32(arm9, OBJECT + off) as i32,
            r32(arm9, OBJECT + off + 4) as i32,
            r32(arm9, OBJECT + off + 8) as i32,
        ]
    };
    let position = v3(arm9, OFF_POSITION);
    let mut scratch = [0u8; 16];
    for i in 0..4 {
        let word = r32(arm9, OBJECT + OFF_SCRATCH + i as u32 * 4);
        scratch[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    Snapshot {
        flags: r32(arm9, OBJECT + OFF_FLAGS),
        facing: [
            r32(arm9, OBJECT + OFF_CURRENT_FACING),
            r32(arm9, OBJECT + OFF_NEXT_FACING),
            r32(arm9, OBJECT + OFF_PREVIOUS_FACING),
            r32(arm9, OBJECT + OFF_NEXT_FACING_BACKUP),
        ],
        previous: v3(arm9, OFF_PREVIOUS),
        current: v3(arm9, OFF_CURRENT),
        position: VecFx32 {
            x: position[0],
            y: position[1],
            z: position[2],
        },
        anim: r32(arm9, OBJECT + OFF_ANIM),
        cmd: r32(arm9, OBJECT + OFF_CMD),
        step: r32(arm9, OBJECT + OFF_STEP),
        scratch,
    }
}

fn snapshot_rust(obj: &MapObject) -> Snapshot {
    Snapshot {
        flags: obj.flags,
        facing: [
            obj.current_facing.index(),
            obj.next_facing.index(),
            obj.previous_facing.index(),
            obj.next_facing_backup.index(),
        ],
        previous: obj.previous,
        current: obj.current,
        position: obj.position,
        anim: obj.anim_group,
        cmd: u32::from(obj.movement_cmd.as_u8()),
        step: obj.movement_step,
        scratch: obj.scratch,
    }
}

/// Writes a `PlayerAvatar` (and its object at `OBJECT`).
fn write_avatar(arm9: &mut RetailArm9, avatar: &PlayerAvatar) {
    write_object(arm9, &avatar.object);
    zero(arm9, AVATAR, AVATAR_SIZE);
    w32(arm9, AVATAR + AV_FLAGS, avatar.flags);
    w32(arm9, AVATAR + AV_UNK8, u32::from(avatar.last_command));
    w32(arm9, AVATAR + AV_UNKC, u32::from(avatar.walk_toggle));
    w32(arm9, AVATAR + AV_MOVE_STATE, avatar.move_state as u32);
    w32(arm9, AVATAR + AV_PLAYER_MOVE_STATE, avatar.player_move_state as u32);
    w32(arm9, AVATAR + AV_STATE, avatar.state as u32);
    w32(arm9, AVATAR + AV_GENDER, avatar.gender);
    w32(arm9, AVATAR + AV_UNK24, avatar.bike_gear as u32);
    let dir = |d: Option<Direction>| d.map_or(u32::MAX, Direction::index);
    w32(arm9, AVATAR + AV_UNK28, dir(avatar.held_horizontal));
    w32(arm9, AVATAR + AV_UNK2C, dir(avatar.held_vertical));
    w32(arm9, AVATAR + AV_MAP_OBJECT, OBJECT);
}

fn fresh_object(facing: Direction) -> MapObject {
    let mut obj = MapObject::create(6, 6, facing, 0, 1);
    obj.set_flags(flag::IGNORE_HEIGHTS);
    obj
}

fn fresh_avatar() -> PlayerAvatar {
    let mut avatar = PlayerAvatar::new(
        6,
        6,
        Direction::South,
        PlayerState::Walking,
        0,
        0,
        PlayerSaveData::default(),
    );
    avatar.object.set_flags(flag::IGNORE_HEIGHTS);
    avatar
}

/// The commands the Rust machine implements: face, the five walk
/// speeds, the five walk-on-spot speeds, the instant step, and run.
fn ported_commands() -> impl Iterator<Item = MovementCmd> {
    (0..44u8)
        .chain(84..92)
        .map(|v| MovementCmd::from_u8(v).expect("table command"))
}

// ===== the step machine =============================================

#[test]
fn step_machine_matches_the_original_per_frame() {
    let Some(mut arm9) = load() else { return };

    for cmd in ported_commands() {
        // Start facing something other than the command's direction so
        // previousFacing/nextFacingBackup move too.
        let start = Direction::from_index((u32::from(cmd.as_u8()) + 1) % 4).unwrap();
        let mut obj = fresh_object(start);
        write_object(&mut arm9, &obj);

        call(&mut arm9, "MapObject_SetHeldMovement", &[OBJECT, u32::from(cmd.as_u8())]);
        obj.set_held_movement(cmd).expect("holdable");
        assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "{cmd:?} after SetHeldMovement");

        // Long enough for the slowest 32-frame moves (33 on-spot ticks)
        // plus the idle re-runs of the terminal step.
        for tick in 0..36 {
            call(&mut arm9, "sub_02062400", &[OBJECT]);
            let unimplemented = obj.run_held_movement();
            assert_eq!(unimplemented, None, "{cmd:?} tick {tick}");
            assert_eq!(
                snapshot_arm(&mut arm9),
                snapshot_rust(&obj),
                "{cmd:?} tick {tick}"
            );
        }
        // Sanity on the trajectory's endpoint, not just agreement.
        if let Some(direction) = cmd.direction() {
            if (4..24).contains(&cmd.as_u8()) || cmd.as_u8() >= 84 {
                let expected = VecFx32::from_tile(6 + direction.delta_x(), 0, 6 + direction.delta_z());
                assert_eq!(obj.position, expected, "{cmd:?} lands on the tile centre");
            } else {
                assert_eq!(obj.position, VecFx32::from_tile(6, 0, 6), "{cmd:?} stays put");
            }
        }
        assert!(obj.is_movement_idle(), "{cmd:?} finished");
    }
}

/// `(speed, frames, anim group)` each linear family's `Step0` passes to
/// `sub_020624CC` (`asm/unk_02062108.s:668`–`:1043`), restated from
/// the listing so the direct calls do not trust the port's table.
fn linear_params(cmd: MovementCmd) -> Option<(u32, u32, u32)> {
    Some(match cmd.as_u8() & !3 {
        4 => (0x800, 32, 1),
        8 => (0x1000, 16, 2),
        12 => (0x2000, 8, 3),
        16 => (0x4000, 4, 4),
        20 => (0x8000, 2, 5),
        84 => (0x10000, 1, 0),
        88 => (0x4000, 4, 9),
        _ => return None,
    })
}

/// `(frames, anim group)` each walk-on-spot family's `Step0` passes to
/// `sub_020627B0` (`asm/unk_02062108.s:1084`–`:1315`).
fn on_spot_params(cmd: MovementCmd) -> Option<(u32, u32)> {
    Some(match cmd.as_u8() & !3 {
        24 => (32, 1),
        28 => (16, 2),
        32 => (8, 3),
        36 => (4, 4),
        40 => (2, 5),
        _ => return None,
    })
}

#[test]
fn step_functions_match_when_called_directly() {
    let Some(mut arm9) = load() else { return };

    for cmd in ported_commands() {
        let direction = cmd.direction().expect("every ported command has a direction");
        let mut obj = fresh_object(direction.reverse());
        obj.set_held_movement(cmd).expect("holdable");
        write_object(&mut arm9, &obj);

        if let Some((speed, frames, anim)) = linear_params(cmd) {
            // Step0 = sub_020624CC(object, direction, speed, frames, anim).
            w32(&mut arm9, STACK_TOP, anim);
            call(&mut arm9, "sub_020624CC", &[OBJECT, direction.index(), speed, frames]);
            assert_eq!(obj.run_step(cmd, 0), Ok(StepResult::Continue), "{cmd:?} Step0");
            assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "{cmd:?} after Step0");
            // Step1 = MapObjectMovementCmd090_Step1, once per frame; it
            // returns 0 while frames remain and 1 on the frame it lands.
            for tick in 0..frames {
                let r0 = call(&mut arm9, "MapObjectMovementCmd090_Step1", &[OBJECT]);
                let want = obj.run_step(cmd, 1).expect("ported");
                assert_eq!(r0 == 1, want == StepResult::Continue, "{cmd:?} Step1 tick {tick} result");
                assert_eq!(want == StepResult::Continue, tick + 1 == frames, "{cmd:?} lands on tick {tick}");
                assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "{cmd:?} Step1 tick {tick}");
            }
        } else if let Some((frames, anim)) = on_spot_params(cmd) {
            // Step0 = sub_020627B0(object, direction, frames, anim).
            call(&mut arm9, "sub_020627B0", &[OBJECT, direction.index(), frames, anim]);
            assert_eq!(obj.run_step(cmd, 0), Ok(StepResult::Continue), "{cmd:?} Step0");
            assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "{cmd:?} after Step0");
            // Step1 = MapObjectMovementCmd040_Step1: the stored count is
            // frames + 1, so it takes one more tick than the walk.
            for tick in 0..=frames {
                let r0 = call(&mut arm9, "MapObjectMovementCmd040_Step1", &[OBJECT]);
                let want = obj.run_step(cmd, 1).expect("ported");
                assert_eq!(r0 == 1, want == StepResult::Continue, "{cmd:?} Step1 tick {tick} result");
                assert_eq!(want == StepResult::Continue, tick == frames, "{cmd:?} finishes on tick {tick}");
                assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "{cmd:?} Step1 tick {tick}");
            }
        } else {
            // Step0 = sub_0206247C(object, direction): the face step.
            call(&mut arm9, "sub_0206247C", &[OBJECT, direction.index()]);
            assert_eq!(obj.run_step(cmd, 0), Ok(StepResult::Continue), "{cmd:?} Step0");
            assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "{cmd:?} after Step0");
        }

        // The shared terminal step (MapObjectMovementCmd098_Step2) marks
        // the movement finished and yields without advancing the step.
        let r0 = call(&mut arm9, "MapObjectMovementCmd098_Step2", &[OBJECT]);
        assert_eq!(r0, 0, "{cmd:?} Step2 yields");
        let step = obj.movement_step;
        assert_eq!(obj.run_step(cmd, step), Ok(StepResult::Yield), "{cmd:?} Step2");
        assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "{cmd:?} after Step2");
        assert!(obj.is_movement_idle(), "{cmd:?} finished");
    }
}

#[test]
fn coordinate_helpers_match_for_every_direction() {
    let Some(mut arm9) = load() else { return };

    // sub_0206101C(object, direction, speed): the position advance —
    // a Thumb jump table on the direction, so every case is walked,
    // with speeds beyond the table's to catch any hidden clamp.
    let speeds: [i32; 8] = [0x800, 0x1000, 0x2000, 0x4000, 0x8000, 0x10000, 0x1234, -0x2000];
    for direction in Direction::ALL {
        for speed in speeds {
            let mut obj = fresh_object(direction);
            write_object(&mut arm9, &obj);
            call(&mut arm9, "sub_0206101C", &[OBJECT, direction.index(), speed as u32]);
            obj.advance_position(direction, speed);
            assert_eq!(
                snapshot_arm(&mut arm9),
                snapshot_rust(&obj),
                "sub_0206101C {direction:?} speed {speed:#x}"
            );
        }
    }
    // sub_02060F24(object, direction): previous := current, then the
    // tile steps; sub_02060F78(object): previous := current.
    for direction in Direction::ALL {
        let mut obj = fresh_object(direction);
        obj.current = [3, 1, 9];
        write_object(&mut arm9, &obj);
        call(&mut arm9, "sub_02060F24", &[OBJECT, direction.index()]);
        obj.advance_tile(direction);
        assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "sub_02060F24 {direction:?}");
        obj.current = [7, 0, 2];
        write_object(&mut arm9, &obj);
        call(&mut arm9, "sub_02060F78", &[OBJECT]);
        obj.latch_previous();
        assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "sub_02060F78 {direction:?}");
    }
}

#[test]
fn held_movement_predicates_match() {
    let Some(mut arm9) = load() else { return };

    let words = [
        0,
        flag::ACTIVE,
        flag::ACTIVE | flag::HELD_MOVEMENT,
        flag::ACTIVE | flag::HELD_MOVEMENT | flag::HELD_MOVEMENT_DONE,
        flag::ACTIVE | flag::HELD_MOVEMENT_DONE,
        flag::ACTIVE | flag::SINGLE_MOVEMENT,
        flag::ACTIVE | flag::SINGLE_MOVEMENT | flag::HELD_MOVEMENT | flag::HELD_MOVEMENT_DONE,
        flag::HELD_MOVEMENT,
        flag::HELD_MOVEMENT | flag::HELD_MOVEMENT_DONE,
    ];
    for flags in words {
        let mut obj = fresh_object(Direction::North);
        obj.flags = flags;
        write_object(&mut arm9, &obj);
        assert_eq!(
            call(&mut arm9, "MapObject_AreBitsSetForMovementScriptInit", &[OBJECT]) != 0,
            obj.ready_for_movement(),
            "AreBitsSet flags {flags:#x}"
        );
        assert_eq!(
            call(&mut arm9, "MapObject_IsMovementPaused", &[OBJECT]) != 0,
            obj.is_movement_idle(),
            "IsMovementPaused flags {flags:#x}"
        );
        call(&mut arm9, "MapObject_ClearHeldMovement", &[OBJECT]);
        obj.clear_held_movement();
        assert_eq!(snapshot_arm(&mut arm9), snapshot_rust(&obj), "ClearHeldMovement {flags:#x}");
    }
}

#[test]
fn family_and_direction_lookups_match() {
    let Some(mut arm9) = load() else { return };

    // sub_0206234C over every row's family and every direction (rows
    // are found by any member, so families 0x0D-style also resolve).
    for family in 0..MOVEMENT_CMD_COUNT {
        let Some(_) = MovementCmd::for_direction(family, Direction::North) else {
            continue; // pret asserts here; not callable
        };
        for direction in Direction::ALL {
            let got = call(&mut arm9, "sub_0206234C", &[direction.index(), u32::from(family)]);
            let want = MovementCmd::for_direction(family, direction).unwrap();
            assert_eq!(got, u32::from(want.as_u8()), "family {family} {direction:?}");
        }
    }
    // sub_02062390 and sub_0205DE64 over the whole command range.
    for value in 0..MOVEMENT_CMD_COUNT {
        let cmd = MovementCmd::from_u8(value).unwrap();
        let got = call(&mut arm9, "sub_02062390", &[u32::from(value)]) as i32;
        let want = cmd.direction().map_or(-1, |d| d.index() as i32);
        assert_eq!(got, want, "direction of {cmd:?}");
        let bump = call(&mut arm9, "sub_0205DE64", &[u32::from(value)]) != 0;
        assert_eq!(bump, cmd.is_bump(), "is_bump {cmd:?}");
    }
}

// ===== the avatar leaves ============================================

#[test]
fn pad_direction_resolution_matches_for_all_inputs() {
    let Some(mut arm9) = load() else { return };

    let remembered_x = [None, Some(Direction::West), Some(Direction::East)];
    let remembered_y = [None, Some(Direction::North), Some(Direction::South)];
    for next in Direction::ALL {
        for hx in remembered_x {
            for hy in remembered_y {
                for pad in 0u16..16 {
                    let held = Keys(pad << 4); // RIGHT LEFT UP DOWN bits
                    let mut avatar = fresh_avatar();
                    avatar.object.set_next_facing_direction(next);
                    avatar.held_horizontal = hx;
                    avatar.held_vertical = hy;
                    write_avatar(&mut arm9, &avatar);
                    let got = call(&mut arm9, "sub_0205DDD4", &[AVATAR, 0, u32::from(held.0)]) as i32;
                    let want = direction_from_keys(&avatar, Keys::IDLE, held).map_or(-1, |d| d.index() as i32);
                    assert_eq!(got, want, "next {next:?} hx {hx:?} hy {hy:?} pad {pad:#x}");
                }
            }
        }
    }
}

#[test]
fn turn_versus_walk_decision_matches() {
    let Some(mut arm9) = load() else { return };

    let directions = [None, Some(Direction::North), Some(Direction::South), Some(Direction::West), Some(Direction::East)];
    let states = [AvatarMoveState::None, AvatarMoveState::Moving, AvatarMoveState::Turning];
    for facing in Direction::ALL {
        for direction in directions {
            for state in states {
                let mut avatar = fresh_avatar();
                avatar.object.set_facing_direction_direct(facing);
                avatar.move_state = state;
                write_avatar(&mut arm9, &avatar);
                let arg = direction.map_or(u32::MAX, Direction::index);
                let got = call(&mut arm9, "sub_0205D450", &[AVATAR, arg]);
                let want = avatar.decide(direction);
                assert_eq!(got, want, "facing {facing:?} dir {direction:?} state {state:?}");
                let arm_state = r32(&mut arm9, AVATAR + AV_MOVE_STATE);
                assert_eq!(arm_state, avatar.move_state as u32, "moveState after decide");
            }
        }
    }
}

#[test]
fn player_move_state_derivation_matches() {
    let Some(mut arm9) = load() else { return };

    let move_states = [AvatarMoveState::None, AvatarMoveState::Moving, AvatarMoveState::Turning];
    let phases = [
        PlayerMoveState::None,
        PlayerMoveState::Start,
        PlayerMoveState::Moving,
        PlayerMoveState::End,
    ];
    let object_flags = [
        flag::ACTIVE,
        flag::ACTIVE | flag::HELD_MOVEMENT,
        flag::ACTIVE | flag::HELD_MOVEMENT | flag::HELD_MOVEMENT_DONE,
    ];
    let commands = [MovementCmd::WalkNormalNorth, MovementCmd::WalkOnSpotSlowerNorth];
    for move_state in move_states {
        for phase in phases {
            for flags in object_flags {
                for cmd in commands {
                    for flag1 in [true, false] {
                        let mut avatar = fresh_avatar();
                        avatar.move_state = move_state;
                        avatar.player_move_state = phase;
                        avatar.object.flags = flags | flag::IGNORE_HEIGHTS;
                        avatar.object.movement_cmd = cmd;
                        avatar.set_flag(avatar_flag::UNK1, flag1);
                        write_avatar(&mut arm9, &avatar);
                        call(&mut arm9, "PlayerAvatar_UpdateMovement", &[AVATAR]);
                        avatar.update_movement();
                        let got = r32(&mut arm9, AVATAR + AV_PLAYER_MOVE_STATE);
                        assert_eq!(
                            got,
                            avatar.player_move_state as u32,
                            "move {move_state:?} phase {phase:?} flags {flags:#x} {cmd:?} flag1 {flag1}"
                        );
                        assert_eq!(r32(&mut arm9, AVATAR + AV_MOVE_STATE), avatar.move_state as u32);
                    }
                }
            }
        }
    }
}
