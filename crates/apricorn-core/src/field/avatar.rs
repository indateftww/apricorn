//! The player avatar: input-driven movement over a [`MapObject`].
//!
//! Ports `PlayerAvatar` (`include/player_avatar.h:26`) with the C
//! accessors of `src/player_avatar.c` and the asm control path of
//! `asm/unk_0205CB48.s`:
//!
//! * `:18` `PlayerAvatar_MoveControl` — the per-frame entry;
//! * `:100` `sub_0205CBEC` — may a new command start this frame;
//! * `:401` `PlayerAvatar_UpdateMovement` — the `playerMoveState`
//!   (`START`/`MOVING`/`END`) derivation, run before the input digest;
//! * `:1123` `sub_0205D40C` / `:1169` `sub_0205D450` — the turn-versus-
//!   walk decision; `:1204` `sub_0205D494` (stand), `:1221`
//!   `sub_0205D4B4` (walk / run / bump), `:1380` `sub_0205D610` (turn);
//! * `:1921` `sub_0205DA34` / `:1983` `sub_0205DAA8` — the collision
//!   aggregate, with `sub_020549F4` (`asm/unk_02054648.s:561`) and
//!   `sub_02060DEC` / `sub_02060D94` (`asm/unk_0205FD20.s:2103,:2055`).
//!
//! The `FieldSystem` step counters (`include/field_system.h:207`) and
//! their update rules (`src/field/field_control.c:471`,
//! `src/field/encounter_check.c:1397`) live here as [`EncounterSteps`].
//!
//! Frame order (`src/main.c:95`, `src/field_system.c:207`): the field
//! process runs `UpdateMovement → FieldInput_Update → FieldInput_Process
//! → MoveControl`, then the SysTask queue ticks every map object. A
//! command loaded by `MoveControl` therefore takes its first step in
//! the same frame — [`PlayerAvatar::run_frame`] keeps that order.

use super::input::{FieldInput, FieldInputContext, direction_from_keys, horizontal, vertical};
use super::map_object::{
    BEHAVIOR_NONE, Collision, Direction, MapObject, MovementCmd, TickReport, family, flag,
};
use crate::input::{Keys, key};

/// `obj_player`: the player object's id.
pub const OBJ_PLAYER: u32 = 0xFF;

/// `PLAYER_STATE_*` (`include/constants/global_fieldmap.h:27`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
#[allow(missing_docs)]
pub enum PlayerState {
    Walking = 0,
    Cycling = 1,
    Surfing = 2,
    Rocket = 3,
    UseHm = 4,
    Watering = 5,
    Pokeathlon = 6,
    Fishing = 7,
    Poketch = 8,
    Saving = 9,
    Heal = 10,
    Ladder = 11,
    RocketHeal = 12,
    ApricornShake = 13,
    RocketSaving = 14,
}

/// `AvatarMoveState` (`include/constants/player_avatar.h:15`): what
/// the avatar decided to do with the last accepted input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum AvatarMoveState {
    /// `AVATAR_MOVE_STATE_NONE`: standing.
    #[default]
    None = 0,
    /// `AVATAR_MOVE_STATE_MOVING`: a step (or bump) is in progress.
    Moving = 1,
    /// `AVATAR_MOVE_STATE_TURNING`: a turn in place is in progress.
    Turning = 2,
}

/// `PlayerMoveState` (`include/constants/player_avatar.h:8`): the
/// per-frame phase the field event dispatcher reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum PlayerMoveState {
    /// `PLAYER_MOVE_STATE_NONE`.
    #[default]
    None = 0,
    /// `PLAYER_MOVE_STATE_START`: the first frame after a move began.
    Start = 1,
    /// `PLAYER_MOVE_STATE_MOVING`.
    Moving = 2,
    /// `PLAYER_MOVE_STATE_END`: the frame after a move completed.
    End = 3,
}

/// `PlayerAvatarFlags` (`include/player_avatar.h:9`).
pub mod avatar_flag {
    /// `AVATAR_FLAG_FORCED_MOVEMENT`.
    pub const FORCED_MOVEMENT: u32 = 1 << 0;
    /// `AVATAR_FLAG_UNK1`: set at creation, cleared after the first
    /// completed step (`sub_0205CC74`); gates forced-movement checks.
    pub const UNK1: u32 = 1 << 1;
    /// `AVATAR_FLAG_UNK2`: bike gear latch.
    pub const UNK2: u32 = 1 << 2;
    /// `AVATAR_FLAG_LOCK_BIKE_STATE`.
    pub const LOCK_BIKE_STATE: u32 = 1 << 3;
    /// `AVATAR_FLAG_UNK4`.
    pub const UNK4: u32 = 1 << 4;
    /// `AVATAR_FLAG_UNK5`.
    pub const UNK5: u32 = 1 << 5;
    /// `AVATAR_FLAG_UNK6`: a step was issued this frame (`sub_0205D4B4`).
    pub const UNK6: u32 = 1 << 6;
    /// `AVATAR_FLAG_UNK7`.
    pub const UNK7: u32 = 1 << 7;
}

/// The bits `sub_0205DA34` returns (`asm/unk_0205CB48.s:1921`).
pub mod collide {
    /// Bit 0: the way is blocked (terrain, map edge, object, range).
    pub const BLOCKED: u32 = 1 << 0;
    /// Bit 1: another object stands there (`sub_02060BFC`).
    pub const OBJECT: u32 = 1 << 1;
    /// Bit 2: the target is a ledge to hop (`sub_0205DB68`).
    pub const LEDGE: u32 = 1 << 2;
    /// Bit 3: blocked by a warp entrance or door — silences the
    /// wall-hit sound (`sub_0205DBF4`).
    pub const DOOR: u32 = 1 << 3;
    /// Bit 5: the target is water (`sub_0205DCA0`).
    pub const WATER: u32 = 1 << 5;
}

/// `PlayerSaveData` (`include/player_avatar.h:20`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerSaveData {
    /// `hasRunningShoes`: `FALSE` at a new game
    /// (`PlayerSaveData_Init`); B does nothing until it is set.
    pub has_running_shoes: bool,
    /// `runningShoesLock`: B is forced on while set.
    pub running_shoes_lock: bool,
    /// `state`: the saved `PLAYER_STATE_*`.
    pub state: i32,
}

/// What `MoveControl` decided for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOutcome {
    /// `sub_0205CBEC` refused: a command is still running and the
    /// input cannot interrupt it.
    Busy,
    /// No direction: face the current way (command family 0).
    Idle,
    /// Turn in place (family 40, three ticks).
    Turn(Direction),
    /// Walk one tile (family 12, eight ticks).
    Walk(Direction),
    /// Run one tile (family 88, four ticks).
    Run(Direction),
    /// Blocked: walk on the spot (family 28, seventeen ticks).
    Bump(Direction),
    /// Hop a ledge (family 56) — issued, but not yet runnable.
    Jump(Direction),
    /// The avatar's `PLAYER_STATE_*` has no ported control path.
    Unsupported,
}

/// The `FieldSystem` step counters that walking maintains
/// (`include/field_system.h:207-209`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EncounterSteps {
    /// `lastFacingDirection` (0x7A): the facing at the last encounter
    /// check, to spot back-and-forth turning.
    pub last_facing_direction: u16,
    /// `reverseTurnFrameSteps` (0x7C): checks that reversed the
    /// facing — each raises the effective encounter rate.
    pub reverse_turn_frame_steps: u16,
    /// `encounterInhibitSteps` (0x7E): move ends since the map loaded;
    /// no encounter in the first four.
    pub encounter_inhibit_steps: u16,
}

impl EncounterSteps {
    /// `FieldSystem_CheckWildEncounter`'s prologue
    /// (`src/field/field_control.c:471`): count a move end
    /// (`fieldInput->endMovement`, walks *and* turns), saturating.
    pub fn on_end_movement(&mut self) {
        if self.encounter_inhibit_steps < 0xFFFF {
            self.encounter_inhibit_steps += 1;
        }
    }

    /// `FieldSystem_PerformLandOrSurfEncounterCheck`'s gate
    /// (`src/field/encounter_check.c:216`): no encounter while the
    /// inhibit count is 3 or less.
    #[must_use]
    pub fn encounter_allowed(&self) -> bool {
        self.encounter_inhibit_steps > 3
    }

    /// `FieldSystem_UpdateTurnFrameCounter`
    /// (`src/field/encounter_check.c:1397`), run by the encounter check
    /// once the gate passes and the standing tile has a nonzero
    /// encounter rate: a facing opposite to the last check's counts a
    /// reverse turn (saturating), and the facing is latched.
    pub fn update_turn_frame_counter(&mut self, facing: Direction) {
        let reversed = u32::from(self.last_facing_direction) == facing.reverse().index();
        if reversed && self.reverse_turn_frame_steps < 0xFFFF {
            self.reverse_turn_frame_steps += 1;
        }
        self.last_facing_direction = facing.index() as u16;
    }

    /// Map change (`src/field_warp_tasks.c:268`, `src/encounter.c:142`):
    /// both step counters restart; the last facing is kept.
    pub fn reset(&mut self) {
        self.encounter_inhibit_steps = 0;
        self.reverse_turn_frame_steps = 0;
    }
}

/// What one [`PlayerAvatar::run_frame`] produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameReport {
    /// The frame's input digest (computed before the movement).
    pub input: FieldInput,
    /// The movement decision.
    pub outcome: MoveOutcome,
    /// The object tick's edge flags.
    pub tick: TickReport,
}

/// `PlayerAvatar` (`include/player_avatar.h:26`) — owning its
/// `LocalMapObject` (pret keeps a pointer into the object manager).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerAvatar {
    /// `flags`: [`avatar_flag`] bits.
    pub flags: u32,
    /// `transitionFlags`: pending state-transition bits, applied and
    /// cleared by `Field_PlayerAvatar_ApplyTransitionFlags`
    /// (`asm/overlay_01_021F1AFC.s:26`). No transition is ported; the
    /// word is cleared each frame as the ROM does.
    pub transition_flags: u32,
    /// `unk8`: the last movement command issued (`sub_0205DA1C`);
    /// 255 at creation.
    pub last_command: u8,
    /// `unkC`: the walk-cycle toggle flipped by turns and steps.
    pub walk_toggle: bool,
    /// `moveState`.
    pub move_state: AvatarMoveState,
    /// `playerMoveState`.
    pub player_move_state: PlayerMoveState,
    /// `state`: `PLAYER_STATE_*`.
    pub state: PlayerState,
    /// `gender`: 0 male, 1 female.
    pub gender: u32,
    /// `unk24`: the bicycle gear (0–3).
    pub bike_gear: i32,
    /// `unk28`: the horizontal pad component recorded by the last
    /// accepted `MoveControl` (`sub_0205CC4C`); -1 = `None`.
    pub held_horizontal: Option<Direction>,
    /// `unk2C`: the vertical component likewise.
    pub held_vertical: Option<Direction>,
    /// `mapObject`: the player's map object.
    pub object: MapObject,
    /// `playerSaveData`.
    pub save: PlayerSaveData,
    /// `GAME_STAT_STEPS_WALKED` (`sub_0205E048`): steps issued.
    pub steps_walked: u32,
}

impl PlayerAvatar {
    /// `PlayerAvatar_CreateWithParams` (`src/player_avatar.c:144`) with
    /// `PlayerAvatar_Setup` (`:212`) and
    /// `PlayerAvatar_CreateMapObjectWithParams` (`:226`): the avatar at
    /// tile `(x, z)` facing `direction`. `sprite_id` is whatever
    /// `PlayerAvatar_GetSpriteByStateAndGender` picks — movement never
    /// reads it.
    #[must_use]
    pub fn new(
        x: i32,
        z: i32,
        direction: Direction,
        state: PlayerState,
        gender: u32,
        sprite_id: u32,
        save: PlayerSaveData,
    ) -> Self {
        let mut object = MapObject::create(x, z, direction, sprite_id, 1);
        object.id = OBJ_PLAYER;
        object.x_range = -1;
        object.y_range = -1;
        object.set_flags(flag::UNK13 | flag::KEEP);
        object.clear_flags(flag::UNK8 | flag::FACING_LOCKED);
        object.set_flags(flag::UNK29);
        let mut avatar = Self {
            flags: 0,
            transition_flags: 0,
            last_command: 255,
            walk_toggle: false,
            move_state: AvatarMoveState::None,
            player_move_state: PlayerMoveState::None,
            state,
            gender,
            bike_gear: 0,
            held_horizontal: None,
            held_vertical: None,
            object,
            save,
            steps_walked: 0,
        };
        avatar.clear_gear_and_flag2();
        avatar.set_flag(avatar_flag::UNK1, true);
        avatar
    }

    /// `PlayerAvatar_GetFacingDirection`.
    #[must_use]
    pub fn facing(&self) -> Direction {
        self.object.current_facing
    }

    /// `PlayerAvatar_GetXCoord`.
    #[must_use]
    pub fn x(&self) -> i32 {
        self.object.current[0]
    }

    /// `PlayerAvatar_GetZCoord`.
    #[must_use]
    pub fn z(&self) -> i32 {
        self.object.current[2]
    }

    /// `PlayerAvatar_Set*` / `PlayerAvatar_Check*` for one flag bit.
    pub fn set_flag(&mut self, bit: u32, set: bool) {
        if set {
            self.flags |= bit;
        } else {
            self.flags &= !bit;
        }
    }

    /// Whether `bit` is set.
    #[must_use]
    pub fn has_flag(&self, bit: u32) -> bool {
        self.flags & bit != 0
    }

    /// `PlayerAvatar_ClearUnk24ClearFlag2`.
    pub fn clear_gear_and_flag2(&mut self) {
        self.bike_gear = 0;
        self.set_flag(avatar_flag::UNK2, false);
    }

    /// `sub_0205D01C(avatar, -1)`: the terrain-forced movement kind
    /// (ice sliding, `_020FCB88`). Ice is not ported, so this is
    /// always 0 — the "no forced movement" handler.
    #[must_use]
    pub fn forced_movement_kind(&self) -> u32 {
        0
    }

    /// `PlayerAvatar_UpdateMovement` (`asm/unk_0205CB48.s:401`): derive
    /// this frame's `playerMoveState` from the object's held-movement
    /// flags as the previous frame's tick left them. Runs first in
    /// the frame.
    pub fn update_movement(&mut self) {
        let move_state = self.move_state;
        let previous = self.player_move_state;
        self.player_move_state = PlayerMoveState::None;
        let forced = self.forced_movement_kind();
        if forced != 0 && forced != 2 {
            self.player_move_state = PlayerMoveState::Moving;
            return;
        }
        if !self.object.ready_for_movement() {
            match move_state {
                AvatarMoveState::None => {}
                AvatarMoveState::Moving => {
                    if self.object.movement_cmd.is_bump() {
                        return;
                    }
                    self.player_move_state =
                        if previous == PlayerMoveState::None || previous == PlayerMoveState::End {
                            PlayerMoveState::Start
                        } else {
                            PlayerMoveState::Moving
                        };
                }
                AvatarMoveState::Turning => self.player_move_state = PlayerMoveState::Moving,
            }
            return;
        }
        if self.object.is_movement_idle() {
            match move_state {
                AvatarMoveState::None => {}
                AvatarMoveState::Moving | AvatarMoveState::Turning => {
                    if previous == PlayerMoveState::None {
                        return;
                    }
                    self.player_move_state = if previous == PlayerMoveState::End {
                        PlayerMoveState::None
                    } else {
                        PlayerMoveState::End
                    };
                }
            }
        }
    }

    /// `sub_0205DA1C` (`asm/unk_0205CB48.s:1907`): remember and load a
    /// command.
    fn hold(&mut self, command: MovementCmd) {
        self.last_command = command.as_u8();
        self.object
            .set_held_movement(command)
            .expect("table commands are holdable");
    }

    /// `sub_0205CBEC` (`asm/unk_0205CB48.s:100`): may a new command
    /// start now? Yes when the object is idle; during a bump, yes if
    /// the way the pad asks for has become free (or is water while
    /// surfing); otherwise no.
    #[must_use]
    pub fn can_accept(&self, direction: Option<Direction>, terrain: &dyn Collision) -> bool {
        if self.object.ready_for_movement() {
            return true;
        }
        let Some(direction) = direction else {
            return false;
        };
        if !self.object.movement_cmd.is_bump() {
            return false;
        }
        let bits = self.check_collision(direction, terrain);
        bits == 0 || (bits == collide::WATER && self.state == PlayerState::Surfing)
    }

    /// `sub_0205D40C` / `sub_0205D450` (`asm/unk_0205CB48.s:1123,:1169`):
    /// the turn-versus-walk rule. No direction: stand (0). A direction
    /// other than the facing while not already walking: turn first (2).
    /// Otherwise walk (1) — including a direction change while walking,
    /// which turns *and* steps in one command. Returns pret's 0/1/2
    /// and sets `move_state` accordingly.
    pub fn decide(&mut self, direction: Option<Direction>) -> u32 {
        let Some(direction) = direction else {
            self.move_state = AvatarMoveState::None;
            return 0;
        };
        if self.facing() != direction && self.move_state != AvatarMoveState::Moving {
            self.move_state = AvatarMoveState::Turning;
            return 2;
        }
        self.move_state = AvatarMoveState::Moving;
        1
    }

    /// `sub_0205D494`: stand facing the current way.
    fn stand(&mut self) -> MoveOutcome {
        let cmd = MovementCmd::for_direction(family::FACE, self.facing()).expect("face row");
        self.hold(cmd);
        MoveOutcome::Idle
    }

    /// `sub_0205D610`: turn in place (family 40), queue the facing,
    /// reset and flip the walk toggle.
    fn turn(&mut self, direction: Direction) -> MoveOutcome {
        let cmd = MovementCmd::for_direction(family::WALK_ON_SPOT_FASTEST, direction).expect("row");
        self.hold(cmd);
        self.object.set_next_facing_direction(direction);
        self.walk_toggle = false;
        self.walk_toggle = !self.walk_toggle;
        MoveOutcome::Turn(direction)
    }

    /// `sub_0205D4B4` (`asm/unk_0205CB48.s:1221`), the walking-state
    /// branch: a ledge ahead hops it (family 56); anything else blocking
    /// bumps (family 28) and queues the facing; a free tile walks
    /// (family 12) or, with running shoes and B held, runs (family 88),
    /// counting a step and flagging it.
    fn walk(&mut self, direction: Direction, held: Keys, terrain: &dyn Collision) -> MoveOutcome {
        let bits = self.check_collision(direction, terrain);
        let (family, outcome) = if bits & collide::LEDGE != 0 {
            (family::JUMP_2, MoveOutcome::Jump(direction))
        } else if bits != 0 {
            // `!(bits & DOOR)` → SEQ_SE_DP_WALL_HIT (presentation).
            self.object.set_next_facing_direction(direction);
            (family::WALK_ON_SPOT_SLOWER, MoveOutcome::Bump(direction))
        } else {
            let running = self.save.has_running_shoes && held.any(key::B);
            let family = if running { family::RUN } else { family::WALK_NORMAL };
            self.steps_walked = self.steps_walked.wrapping_add(1);
            self.set_flag(avatar_flag::UNK6, true);
            let outcome = if running {
                MoveOutcome::Run(direction)
            } else {
                MoveOutcome::Walk(direction)
            };
            (family, outcome)
        };
        let cmd = MovementCmd::for_direction(family, direction).expect("row");
        self.hold(cmd);
        // Not blocked → ov01_02205990 (grass/footprint effect; presentation).
        outcome
    }

    /// `sub_0205D3A8` (`asm/unk_0205CB48.s:1070`): the walking-state
    /// control — decide, then stand / walk / turn.
    fn walking_control(&mut self, direction: Option<Direction>, held: Keys, terrain: &dyn Collision) -> MoveOutcome {
        match self.decide(direction) {
            0 => self.stand(),
            1 => self.walk(direction.expect("walk needs a direction"), held, terrain),
            _ => self.turn(direction.expect("turn needs a direction")),
        }
    }

    /// `PlayerAvatar_MoveControl(avatar, mapLoadManager, -1, newKeys,
    /// heldKeys, flag)` (`asm/unk_0205CB48.s:18`) for
    /// [`PlayerState::Walking`]. The direction comes from the held pad
    /// (`sub_0205DDD4`); if the object cannot take a command the frame
    /// is [`MoveOutcome::Busy`]. Otherwise the pad components are
    /// recorded (`sub_0205CC4C`), transition flags applied, the
    /// forced-movement path consulted (`sub_0205D004`, no-op without
    /// ice), and the state's control runs (`sub_0205D340`). Footstep
    /// sounds (`sub_0205CC94`) and the special-sprite hooks
    /// (`ov01_021F2F24` / `ov01_021F2EDC`) are presentation and skipped.
    pub fn move_control(&mut self, input: &FieldInput, terrain: &dyn Collision) -> MoveOutcome {
        let held = input.held_keys;
        let direction = direction_from_keys(self, input.new_keys, held);
        if !self.can_accept(direction, terrain) {
            return MoveOutcome::Busy;
        }
        self.held_horizontal = horizontal(held);
        self.held_vertical = vertical(held);
        self.set_flag(avatar_flag::UNK6, false);
        self.transition_flags = 0;
        if self.forced_movement_kind() == 1 {
            // Ice sliding (sub_0205D0A8) is not ported.
            return MoveOutcome::Unsupported;
        }
        let outcome = match self.state {
            PlayerState::Walking => {
                self.decide(direction);
                self.walking_control(direction, held, terrain)
            }
            _ => return MoveOutcome::Unsupported,
        };
        // sub_0205CC74
        if self.has_flag(avatar_flag::UNK6) && self.move_state == AvatarMoveState::Moving {
            self.set_flag(avatar_flag::UNK1, false);
        }
        outcome
    }

    /// `sub_0205DAA8` (`asm/unk_0205CB48.s:1983`): the raw probe of the
    /// tile ahead — bit 0 range (`sub_02060D94`), bit 1 terrain
    /// (`sub_020549F4`: the impassable attribute; elevation cliffs and
    /// map gimmicks are deferred) or edge rules (`sub_02060DEC`), bit 2
    /// another object (`sub_02060BFC`; no other objects are tracked
    /// here).
    #[must_use]
    pub fn collision_probe(&self, direction: Direction, terrain: &dyn Collision) -> u32 {
        let tx = self.x().wrapping_add(direction.delta_x());
        let tz = self.z().wrapping_add(direction.delta_z());
        let mut bits = 0;
        if self.object.out_of_range(tx, tz) {
            bits |= 1;
        }
        if terrain.impassable(tx, tz) {
            bits |= 2;
        }
        if self.object.edge_blocked(direction, terrain.behavior(tx, tz)) {
            bits |= 2;
        }
        bits
    }

    /// `sub_0205DBF4` (`asm/unk_0205CB48.s:2146`): standing on a warp
    /// entrance (the asm's switch falls through: north tests all four,
    /// south three, west two, east one) or facing a door.
    #[must_use]
    pub fn facing_warp_or_door(&self, direction: Direction, terrain: &dyn Collision) -> bool {
        const WARP_EAST: u8 = 98;
        const WARP_WEST: u8 = 99;
        const WARP_NORTH: u8 = 100;
        const WARP_SOUTH: u8 = 101;
        const DOOR: u8 = 105;
        let here = (self.object.current_behavior & 0xFF) as u8;
        let entrances: &[u8] = match direction {
            Direction::North => &[WARP_NORTH, WARP_SOUTH, WARP_WEST, WARP_EAST],
            Direction::South => &[WARP_SOUTH, WARP_WEST, WARP_EAST],
            Direction::West => &[WARP_WEST, WARP_EAST],
            Direction::East => &[WARP_EAST],
        };
        if entrances.contains(&here) {
            return true;
        }
        let tx = self.x().wrapping_add(direction.delta_x());
        let tz = self.z().wrapping_add(direction.delta_z());
        terrain.behavior(tx, tz) == DOOR
    }

    /// `sub_0205DB68` (`asm/unk_0205CB48.s:2076`): the tile ahead is
    /// the ledge that hops in `direction`.
    #[must_use]
    pub fn ledge_ahead(&self, direction: Direction, terrain: &dyn Collision) -> bool {
        const JUMP_EAST: u8 = 56;
        const JUMP_WEST: u8 = 57;
        const JUMP_NORTH: u8 = 58;
        const JUMP_SOUTH: u8 = 59;
        let tx = self.x().wrapping_add(direction.delta_x());
        let tz = self.z().wrapping_add(direction.delta_z());
        let ahead = terrain.behavior(tx, tz);
        ahead
            == match direction {
                Direction::North => JUMP_NORTH,
                Direction::South => JUMP_SOUTH,
                Direction::West => JUMP_WEST,
                Direction::East => JUMP_EAST,
            }
    }

    /// `sub_0205DCA0` (`asm/unk_0205CB48.s:2231`) → `sub_02060E54` (`asm/unk_0205FD20.s:2155`): the
    /// tile ahead is water the avatar cannot walk onto —
    /// `TILE_BEHAVIOR_115` unless the object's bit 28 is set, any
    /// surfable water, or `TILE_BEHAVIOR_34` for the two fishing
    /// sprites (0xB2/0xB3).
    #[must_use]
    pub fn water_ahead(&self, direction: Direction, terrain: &dyn Collision) -> bool {
        let tx = self.x().wrapping_add(direction.delta_x());
        let tz = self.z().wrapping_add(direction.delta_z());
        let ahead = terrain.behavior(tx, tz);
        if ahead == 115 && !self.object.test_flags(1 << 28) {
            return true;
        }
        if terrain.surfable(tx, tz) {
            return true;
        }
        ahead == 34 && (0xB2..=0xB3).contains(&self.object.sprite_id)
    }

    /// `sub_0205DA34` (`asm/unk_0205CB48.s:1921`): the collision
    /// aggregate for a step in `direction` — [`collide`] bits; 0 means
    /// free. The surfing-only probe (`sub_0205DCFC`) is not ported.
    #[must_use]
    pub fn check_collision(&self, direction: Direction, terrain: &dyn Collision) -> u32 {
        let raw = self.collision_probe(direction, terrain);
        let mut bits = 0;
        if raw & 0xA != 0 {
            bits |= collide::BLOCKED;
            if self.facing_warp_or_door(direction, terrain) {
                bits |= collide::DOOR;
            }
        }
        if raw & 4 != 0 {
            bits |= collide::OBJECT;
        }
        if self.ledge_ahead(direction, terrain) {
            bits |= collide::LEDGE;
        }
        if self.water_ahead(direction, terrain) {
            bits |= collide::WATER;
        }
        bits
    }

    /// One field frame of player movement in the ROM's order
    /// (`FieldSystem_Control` then the object SysTask):
    /// [`update_movement`](Self::update_movement) →
    /// [`FieldInput::update`] → [`move_control`](Self::move_control) →
    /// [`MapObject::tick`]. `FieldInput_Process` (events, encounters,
    /// menus) sits between the digest and the control in the ROM and
    /// is the field system's to run from the returned digest; a frame
    /// it consumes should skip `move_control` but still tick the object.
    pub fn run_frame(
        &mut self,
        new_keys: Keys,
        held_keys: Keys,
        terrain: &dyn Collision,
        ctx: FieldInputContext,
        last_touch_menu_input: &mut u16,
    ) -> FrameReport {
        self.update_movement();
        let input = FieldInput::update(self, last_touch_menu_input, new_keys, held_keys, ctx);
        let outcome = self.move_control(&input, terrain);
        let tick = self.object.tick(terrain);
        FrameReport {
            input,
            outcome,
            tick,
        }
    }
}

impl MapObject {
    /// `sub_02060D94` (`asm/unk_0205FD20.s:2055`): the tile lies
    /// outside the object's `xRange` / `yRange` box around its initial
    /// tile (-1 = unbounded on that axis).
    #[must_use]
    pub fn out_of_range(&self, x: i32, z: i32) -> bool {
        let outside = |initial: i32, range: i32, value: i32| {
            range != -1 && (value < initial - range || value > initial + range)
        };
        outside(self.initial[0], self.x_range, x) || outside(self.initial[2], self.y_range, z)
    }

    /// `sub_02060DEC` (`asm/unk_0205FD20.s:2103`): the standing tile's
    /// behaviour forbids leaving in `direction` (`_020FD4CC`) or the
    /// target's forbids entering from it (`_020FD4BC`), or the target
    /// has no behaviour at all. Objects with `flags2` bit 2 skip the
    /// check. The edge tables are `sub_0205B8F4` / `sub_0205B918` /
    /// `sub_0205B93C` / `sub_0205B960` (`src/metatile_behavior.c:438`).
    #[must_use]
    pub fn edge_blocked(&self, direction: Direction, target_behavior: u8) -> bool {
        if self.flags2 & (1 << 2) != 0 {
            return false;
        }
        if target_behavior == BEHAVIOR_NONE {
            return true;
        }
        // sub_0205B8F4: 50, 52, 53, LADDER_NORTH (60), 73
        let b8f4 = |t: u8| matches!(t, 50 | 52 | 53 | 60 | 73);
        // sub_0205B918: 51, 54, 55, LADDER_SOUTH (61), 73
        let b918 = |t: u8| matches!(t, 51 | 54 | 55 | 61 | 73);
        // sub_0205B93C: 49, 53, 55, 74
        let b93c = |t: u8| matches!(t, 49 | 53 | 55 | 74);
        // sub_0205B960: 48, 52, 54, 74
        let b960 = |t: u8| matches!(t, 48 | 52 | 54 | 74);
        let here = (self.current_behavior & 0xFF) as u8;
        let (leave, enter): (fn(u8) -> bool, fn(u8) -> bool) = match direction {
            Direction::North => (b8f4, b918),
            Direction::South => (b918, b8f4),
            Direction::West => (b93c, b960),
            Direction::East => (b960, b93c),
        };
        leave(here) || enter(target_behavior)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::map_object::{ATTR_NONE, NoTerrain, VecFx32};

    /// A flat open floor with an impassable ring at |x|,|z| == 3.
    struct Room;

    impl Collision for Room {
        fn attr(&self, x: i32, z: i32) -> u16 {
            if x.abs() > 3 || z.abs() > 3 {
                ATTR_NONE
            } else if x.abs() == 3 || z.abs() == 3 {
                0x8000
            } else {
                0
            }
        }
    }

    fn avatar() -> PlayerAvatar {
        let mut a = PlayerAvatar::new(0, 0, Direction::South, PlayerState::Walking, 0, 0, PlayerSaveData::default());
        a.object.set_flags(flag::IGNORE_HEIGHTS);
        a
    }

    fn frame(a: &mut PlayerAvatar, held: u16, terrain: &dyn Collision) -> FrameReport {
        a.run_frame(Keys::IDLE, Keys(held), terrain, FieldInputContext::default(), &mut 0)
    }

    #[test]
    fn creation_matches_pret_setup() {
        let a = avatar();
        assert_eq!(a.object.id, OBJ_PLAYER);
        assert_eq!(a.object.x_range, -1);
        assert_eq!(a.last_command, 255);
        assert!(a.has_flag(avatar_flag::UNK1));
        assert!(a.object.test_flags(flag::KEEP | flag::UNK13 | flag::UNK29));
        assert!(!a.object.test_flags(flag::FACING_LOCKED));
        assert_eq!(a.object.position, VecFx32::from_tile(0, 0, 0));
    }

    #[test]
    fn walking_forward_takes_eight_frames_and_reports_phases() {
        let mut a = avatar();
        let room = Room;
        let mut phases = Vec::new();
        for _ in 0..9 {
            let r = frame(&mut a, key::DOWN, &room);
            phases.push(a.player_move_state);
            if phases.len() == 1 {
                assert_eq!(r.outcome, MoveOutcome::Walk(Direction::South));
            }
        }
        assert_eq!(
            phases,
            [
                PlayerMoveState::None,
                PlayerMoveState::Start,
                PlayerMoveState::Moving,
                PlayerMoveState::Moving,
                PlayerMoveState::Moving,
                PlayerMoveState::Moving,
                PlayerMoveState::Moving,
                PlayerMoveState::Moving,
                PlayerMoveState::End,
            ]
        );
        // Frame 8 saw the END and immediately started the next step.
        assert_eq!(a.object.current, [0, 0, 2]);
        assert_eq!(a.steps_walked, 2);
    }

    #[test]
    fn a_new_direction_turns_first_then_walks() {
        let mut a = avatar();
        let room = Room;
        let r = frame(&mut a, key::RIGHT, &room);
        assert_eq!(r.outcome, MoveOutcome::Turn(Direction::East));
        assert_eq!(a.facing(), Direction::East);
        assert_eq!(a.move_state, AvatarMoveState::Turning);
        assert_eq!(frame(&mut a, key::RIGHT, &room).outcome, MoveOutcome::Busy);
        assert_eq!(frame(&mut a, key::RIGHT, &room).outcome, MoveOutcome::Busy);
        let r = frame(&mut a, key::RIGHT, &room);
        assert_eq!(r.outcome, MoveOutcome::Walk(Direction::East));
        assert!(r.input.end_movement, "the turn's END phase");
        assert!(!r.input.movement, "a turn is not a step");
        assert_eq!(a.object.current, [1, 0, 0]);
    }

    #[test]
    fn a_tap_turns_and_then_stands() {
        let mut a = avatar();
        let room = Room;
        assert_eq!(frame(&mut a, key::LEFT, &room).outcome, MoveOutcome::Turn(Direction::West));
        assert_eq!(frame(&mut a, 0, &room).outcome, MoveOutcome::Busy);
        assert_eq!(frame(&mut a, 0, &room).outcome, MoveOutcome::Busy);
        assert_eq!(frame(&mut a, 0, &room).outcome, MoveOutcome::Idle);
        assert_eq!(a.facing(), Direction::West);
        assert_eq!(a.move_state, AvatarMoveState::None);
        assert_eq!(a.object.current, [0, 0, 0]);
    }

    #[test]
    fn changing_direction_mid_walk_does_not_turn() {
        let mut a = avatar();
        let room = Room;
        for _ in 0..8 {
            frame(&mut a, key::DOWN, &room);
        }
        let r = frame(&mut a, key::RIGHT, &room);
        assert_eq!(r.outcome, MoveOutcome::Walk(Direction::East));
        assert_eq!(a.facing(), Direction::East);
        assert_eq!(a.object.current, [1, 0, 1]);
    }

    #[test]
    fn a_blocked_step_bumps_for_seventeen_frames() {
        let mut a = avatar();
        let room = Room;
        a.object.current = [0, 0, 2];
        a.object.previous = [0, 0, 2];
        a.object.position = VecFx32::from_tile(0, 0, 2);
        let r = frame(&mut a, key::DOWN, &room);
        assert_eq!(r.outcome, MoveOutcome::Bump(Direction::South));
        assert_eq!(a.object.movement_cmd, MovementCmd::WalkOnSpotSlowerSouth);
        for _ in 0..16 {
            assert_eq!(frame(&mut a, key::DOWN, &room).outcome, MoveOutcome::Busy);
        }
        assert_eq!(frame(&mut a, key::DOWN, &room).outcome, MoveOutcome::Bump(Direction::South));
        assert_eq!(a.object.current, [0, 0, 2]);
        assert_eq!(a.object.position, VecFx32::from_tile(0, 0, 2));
        assert_eq!(a.steps_walked, 0);
        assert_eq!(a.player_move_state, PlayerMoveState::None, "bumps never report phases");
    }

    #[test]
    fn a_free_direction_interrupts_a_bump() {
        let mut a = avatar();
        let room = Room;
        a.object.current = [0, 0, 2];
        a.object.previous = [0, 0, 2];
        a.object.position = VecFx32::from_tile(0, 0, 2);
        frame(&mut a, key::DOWN, &room);
        frame(&mut a, key::DOWN, &room);
        let r = frame(&mut a, key::LEFT, &room);
        assert_eq!(r.outcome, MoveOutcome::Walk(Direction::West));
        assert_eq!(a.object.current, [-1, 0, 2]);
    }

    #[test]
    fn running_needs_shoes_and_b() {
        let mut a = avatar();
        let room = Room;
        assert_eq!(frame(&mut a, key::DOWN | key::B, &room).outcome, MoveOutcome::Walk(Direction::South));
        let mut a = avatar();
        a.save.has_running_shoes = true;
        assert_eq!(frame(&mut a, key::DOWN | key::B, &room).outcome, MoveOutcome::Run(Direction::South));
        assert_eq!(a.object.movement_cmd, MovementCmd::RunSouth);
        for _ in 0..3 {
            frame(&mut a, key::DOWN | key::B, &room);
        }
        assert_eq!(a.object.position, VecFx32::from_tile(0, 0, 1));
    }

    #[test]
    fn out_of_map_and_edge_rules_block() {
        let mut a = avatar();
        assert_ne!(a.check_collision(Direction::South, &NoTerrain), 0);
        a.object.current_behavior = 73; // blocks north and south exits
        assert_eq!(a.check_collision(Direction::South, &Room) & collide::BLOCKED, collide::BLOCKED);
        assert_eq!(a.check_collision(Direction::East, &Room), 0);
        a.object.current_behavior = 0;
        struct Ledge;
        impl Collision for Ledge {
            fn attr(&self, _x: i32, z: i32) -> u16 {
                if z == 1 { 59 } else { 0 }
            }
        }
        assert_eq!(a.check_collision(Direction::South, &Ledge), collide::LEDGE);
        assert_eq!(a.check_collision(Direction::North, &Ledge), 0);
    }

    #[test]
    fn encounter_step_counters_follow_the_rules() {
        let mut s = EncounterSteps::default();
        for _ in 0..4 {
            assert!(!s.encounter_allowed());
            s.on_end_movement();
        }
        assert!(s.encounter_allowed());
        s.update_turn_frame_counter(Direction::North);
        assert_eq!(s.reverse_turn_frame_steps, 0);
        s.update_turn_frame_counter(Direction::South);
        assert_eq!(s.reverse_turn_frame_steps, 1);
        s.update_turn_frame_counter(Direction::South);
        assert_eq!(s.reverse_turn_frame_steps, 1);
        s.update_turn_frame_counter(Direction::North);
        assert_eq!(s.reverse_turn_frame_steps, 2);
        s.reset();
        assert_eq!(s.encounter_inhibit_steps, 0);
        assert_eq!(s.last_facing_direction, 0);
    }
}
