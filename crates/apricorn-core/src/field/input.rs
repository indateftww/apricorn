//! The per-frame field input digest — port of `FieldInput_Update`
//! (`src/field/field_control.c:121`) and the key-to-direction helpers
//! it calls (`sub_0205DD9C` / `sub_0205DDB8` / `sub_0205DDD4`,
//! `asm/unk_0205CB48.s:2348-2440`).
//!
//! `FieldInput_Update` runs once per frame, right after
//! `PlayerAvatar_UpdateMovement` and before `FieldInput_Process` and
//! `PlayerAvatar_MoveControl` (`src/field_system.c:207`,
//! `FieldSystem_Control`). It turns the raw pad state into the flags
//! the field event dispatcher reads, and records the direction the
//! pad currently asks for.

use super::avatar::{AvatarMoveState, PlayerAvatar, PlayerMoveState};
use super::map_object::Direction;
use crate::input::{Keys, key};

/// `PAD_PLUS_KEY_MASK`: the four directions.
pub const PLUS_KEY_MASK: u16 = key::UP | key::DOWN | key::LEFT | key::RIGHT;

/// `sub_0205DD9C` (`asm/unk_0205CB48.s:2356`): the horizontal component
/// of a pad word — LEFT wins over RIGHT.
#[must_use]
pub fn horizontal(held: Keys) -> Option<Direction> {
    if held.any(key::LEFT) {
        Some(Direction::West)
    } else if held.any(key::RIGHT) {
        Some(Direction::East)
    } else {
        None
    }
}

/// `sub_0205DDB8` (`asm/unk_0205CB48.s:2377`): the vertical component —
/// UP wins over DOWN.
#[must_use]
pub fn vertical(held: Keys) -> Option<Direction> {
    if held.any(key::UP) {
        Some(Direction::North)
    } else if held.any(key::DOWN) {
        Some(Direction::South)
    } else {
        None
    }
}

/// `sub_0205DDD4` (`asm/unk_0205CB48.s:2398`; `sub_0205DD94` is a
/// thunk to it): the direction the held pad asks for. One axis held
/// gives that axis. Both held resolves against what the avatar
/// remembers from the previous `MoveControl`
/// ([`PlayerAvatar::held_horizontal`] / [`PlayerAvatar::held_vertical`],
/// the components recorded then) and its `nextFacing`:
///
/// * the same diagonal as last time keeps the current `nextFacing`;
/// * a vertical that was already held makes the horizontal the new
///   (winning) axis; otherwise the vertical is the new one.
///
/// `new_keys` is accepted for signature parity; the asm never reads it.
#[must_use]
pub fn direction_from_keys(avatar: &PlayerAvatar, _new_keys: Keys, held: Keys) -> Option<Direction> {
    let x = horizontal(held);
    let y = vertical(held);
    let Some(x) = x else { return y };
    let Some(y) = y else { return Some(x) };
    let next = avatar.object.next_facing;
    if Some(x) == avatar.held_horizontal && Some(y) == avatar.held_vertical {
        return Some(next);
    }
    if Some(y) == avatar.held_vertical {
        Some(x)
    } else {
        Some(y)
    }
}

/// What `FieldInput_Update` reads off the `FieldSystem` besides the
/// avatar — facts other systems own, passed in so the digest stays a
/// pure function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldInputContext {
    /// `FieldSystem_ShouldDrawStartMenuIcon(fieldSystem, START_MENU_ICON_BAG)`:
    /// the start menu (and Y-item shortcut) is available.
    pub bag_icon_shown: bool,
    /// `ov01_021E690C(fieldSystem)`: a registered item may be used now.
    pub registered_item_usable: bool,
    /// `ov01_021F6B00(fieldSystem) == 4`: X opens the menu directly
    /// (the touch menu is in its button-driven state).
    pub x_menu_direct: bool,
}

/// `FieldInput` (`include/field_system.h:250`): one frame's digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldInput {
    /// `interact`: A was pressed while standing.
    pub interact: bool,
    /// `endMovement`: the avatar's move ended this frame.
    pub end_movement: bool,
    /// `menu`: open the start menu.
    pub menu: bool,
    /// `registeredItem`: 0 none, 1 = Y slot, 2 = the second slot.
    pub registered_item: u8,
    /// `sign`: a direction is held while standing (signpost check).
    pub sign: bool,
    /// `mapTransition`: a direction is held while standing.
    pub map_transition: bool,
    /// `movement`: a walking step completed this frame.
    pub movement: bool,
    /// `unk0_8`: the avatar was standing this frame.
    pub standing: bool,
    /// `unk0_9`: the touch menu asked for the button menu.
    pub touch_menu: bool,
    /// `playerDir`: the direction the pad asks for (`DIR_NONE` = `None`).
    pub player_dir: Option<Direction>,
    /// `transitionDir`: the facing, if the pad holds that same way.
    pub transition_dir: Option<Direction>,
    /// `newKeys`: buttons newly pressed this frame.
    pub new_keys: Keys,
    /// `heldKeys`: buttons held (B forced on while running shoes are locked).
    pub held_keys: Keys,
}

impl FieldInput {
    /// `FieldInput_Update(fieldInput, fieldSystem, newKeys, heldKeys)`
    /// (`src/field/field_control.c:121`). `last_touch_menu_input` is
    /// `fieldSystem->lastTouchMenuInput`, the touch-screen shortcut
    /// latch the routine both reads and clears.
    #[must_use]
    pub fn update(
        avatar: &PlayerAvatar,
        last_touch_menu_input: &mut u16,
        new_keys: Keys,
        mut held_keys: Keys,
        ctx: FieldInputContext,
    ) -> Self {
        let mut input = Self::default();
        if avatar.save.running_shoes_lock {
            held_keys.0 |= key::B;
        }
        let move_state = avatar.player_move_state;
        let avatar_move_state = avatar.move_state;
        let facing = avatar.facing();
        input.new_keys = new_keys;
        input.held_keys = held_keys;

        if move_state == PlayerMoveState::End || move_state == PlayerMoveState::None {
            if (ctx.bag_icon_shown && new_keys.any(key::Y)) || *last_touch_menu_input == 9 {
                if ctx.registered_item_usable {
                    input.registered_item = 1;
                    *last_touch_menu_input = 0;
                }
            } else if *last_touch_menu_input == 10 {
                if ctx.registered_item_usable {
                    input.registered_item = 2;
                    *last_touch_menu_input = 0;
                }
            } else if *last_touch_menu_input == 11 {
                input.touch_menu = true;
                *last_touch_menu_input = 0;
            } else if new_keys.any(key::X) || *last_touch_menu_input != 0 {
                if ctx.x_menu_direct {
                    // MenuInputStateMgr_SetState(BUTTONS) — a UI latch
                    // owned by the menu; not modelled here.
                    input.touch_menu = true;
                    input.menu = true;
                } else if ctx.bag_icon_shown {
                    input.menu = true;
                    if *last_touch_menu_input == 1 {
                        *last_touch_menu_input = 0;
                    }
                }
            } else if new_keys.any(key::A) {
                input.interact = true;
            }
            if held_keys.any(PLUS_KEY_MASK) {
                input.sign = true;
                input.map_transition = true;
            }
            input.standing = true;
        } else {
            *last_touch_menu_input = 0;
        }

        if move_state == PlayerMoveState::End && avatar_move_state == AvatarMoveState::Moving {
            input.movement = true;
        }
        if move_state == PlayerMoveState::End {
            input.end_movement = true;
        }

        let facing_key = match facing {
            Direction::North => key::UP,
            Direction::South => key::DOWN,
            Direction::West => key::LEFT,
            Direction::East => key::RIGHT,
        };
        input.transition_dir = held_keys.any(facing_key).then_some(facing);
        input.player_dir = direction_from_keys(avatar, new_keys, held_keys);
        input
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::avatar::{PlayerSaveData, PlayerState};

    fn avatar() -> PlayerAvatar {
        PlayerAvatar::new(6, 6, Direction::South, PlayerState::Walking, 0, 0, PlayerSaveData::default())
    }

    #[test]
    fn single_axis_keys_map_to_directions() {
        let a = avatar();
        assert_eq!(direction_from_keys(&a, Keys::IDLE, Keys(key::UP)), Some(Direction::North));
        assert_eq!(direction_from_keys(&a, Keys::IDLE, Keys(key::DOWN)), Some(Direction::South));
        assert_eq!(direction_from_keys(&a, Keys::IDLE, Keys(key::LEFT)), Some(Direction::West));
        assert_eq!(direction_from_keys(&a, Keys::IDLE, Keys(key::RIGHT)), Some(Direction::East));
        assert_eq!(direction_from_keys(&a, Keys::IDLE, Keys::IDLE), None);
        // Within an axis the first-tested key wins.
        assert_eq!(horizontal(Keys(key::LEFT | key::RIGHT)), Some(Direction::West));
        assert_eq!(vertical(Keys(key::UP | key::DOWN)), Some(Direction::North));
    }

    #[test]
    fn diagonals_resolve_against_the_remembered_components() {
        let mut a = avatar();
        // Nothing remembered: the vertical is "new" and wins.
        assert_eq!(
            direction_from_keys(&a, Keys::IDLE, Keys(key::UP | key::RIGHT)),
            Some(Direction::North)
        );
        // The vertical was already held last frame: the horizontal is new.
        a.held_vertical = Some(Direction::North);
        assert_eq!(
            direction_from_keys(&a, Keys::IDLE, Keys(key::UP | key::RIGHT)),
            Some(Direction::East)
        );
        // Same diagonal as last frame: keep going the way we face next.
        a.held_horizontal = Some(Direction::East);
        a.object.set_next_facing_direction(Direction::North);
        assert_eq!(
            direction_from_keys(&a, Keys::IDLE, Keys(key::UP | key::RIGHT)),
            Some(Direction::North)
        );
    }

    #[test]
    fn standing_frames_digest_buttons() {
        let a = avatar();
        let mut latch = 0;
        let ctx = FieldInputContext {
            bag_icon_shown: true,
            ..Default::default()
        };
        let input = FieldInput::update(&a, &mut latch, Keys(key::A), Keys(key::A), ctx);
        assert!(input.interact);
        assert!(input.standing);
        assert!(!input.menu);
        let input = FieldInput::update(&a, &mut latch, Keys(key::X), Keys(key::X), ctx);
        assert!(input.menu);
        assert!(!input.interact);
        let input = FieldInput::update(&a, &mut latch, Keys::IDLE, Keys(key::DOWN), ctx);
        assert!(input.sign && input.map_transition);
        assert_eq!(input.transition_dir, Some(Direction::South));
        assert_eq!(input.player_dir, Some(Direction::South));
        let input = FieldInput::update(&a, &mut latch, Keys::IDLE, Keys(key::UP), ctx);
        assert_eq!(input.transition_dir, None);
    }

    #[test]
    fn running_shoes_lock_forces_b() {
        let mut a = avatar();
        a.save.running_shoes_lock = true;
        let input = FieldInput::update(&a, &mut 0, Keys::IDLE, Keys::IDLE, FieldInputContext::default());
        assert!(input.held_keys.down(key::B));
    }

    #[test]
    fn movement_flags_follow_the_player_move_state() {
        let mut a = avatar();
        a.player_move_state = PlayerMoveState::End;
        a.move_state = AvatarMoveState::Moving;
        let input = FieldInput::update(&a, &mut 0, Keys::IDLE, Keys::IDLE, FieldInputContext::default());
        assert!(input.movement && input.end_movement && input.standing);
        a.move_state = AvatarMoveState::Turning;
        let input = FieldInput::update(&a, &mut 0, Keys::IDLE, Keys::IDLE, FieldInputContext::default());
        assert!(!input.movement && input.end_movement);
        a.player_move_state = PlayerMoveState::Moving;
        let mut latch = 5;
        let input = FieldInput::update(&a, &mut latch, Keys(key::A), Keys(key::A), FieldInputContext::default());
        assert!(!input.interact && !input.standing);
        assert_eq!(latch, 0, "a moving frame clears the touch latch");
    }
}
