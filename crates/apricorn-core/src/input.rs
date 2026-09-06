//! Input — the button and stylus state one tick consumes.
//!
//! The button layout is the hardware `REG_KEYXY` register's, the same
//! twelve bits the harness's input scripts (`apricorn-harness::input`)
//! define — cited, not shared, so the headless core keeps zero
//! dependencies on the harness. Convention throughout the engine:
//! **bit set = held** (the register itself is active-low; every
//! producer — harness script, desktop keyboard — inverts at its edge).

/// The button mask bits, `REG_KEYXY` layout.
pub mod key {
    /// A.
    pub const A: u16 = 1 << 0;
    /// B.
    pub const B: u16 = 1 << 1;
    /// SELECT.
    pub const SELECT: u16 = 1 << 2;
    /// START.
    pub const START: u16 = 1 << 3;
    /// RIGHT.
    pub const RIGHT: u16 = 1 << 4;
    /// LEFT.
    pub const LEFT: u16 = 1 << 5;
    /// UP.
    pub const UP: u16 = 1 << 6;
    /// DOWN.
    pub const DOWN: u16 = 1 << 7;
    /// R shoulder.
    pub const R: u16 = 1 << 8;
    /// L shoulder.
    pub const L: u16 = 1 << 9;
    /// X.
    pub const X: u16 = 1 << 10;
    /// Y.
    pub const Y: u16 = 1 << 11;
}

/// The buttons held — one bit per `REG_KEYXY` key, bit set = held.
///
/// Constructed by whatever fronts the engine: the harness from a
/// `.apin` script, the desktop shell from the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Keys(pub u16);

impl Keys {
    /// No buttons held.
    pub const IDLE: Self = Self(0);

    /// Whether every button in `mask` (a `key::*` bit set) is held.
    #[must_use]
    pub fn down(self, mask: u16) -> bool {
        self.0 & mask == mask
    }

    /// Whether any button in `mask` is held.
    #[must_use]
    pub fn any(self, mask: u16) -> bool {
        self.0 & mask != 0
    }

    /// The buttons newly held this tick versus `prev`.
    #[must_use]
    pub fn pressed(self, prev: Self) -> Self {
        Self(self.0 & !prev.0)
    }
}

/// Stylus contact on the touch LCD — raw coordinates, 0–255 per axis
/// (the resolution the harness's `touch` events carry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Touch {
    /// X coordinate, 0–255 (left = 0).
    pub x: u16,
    /// Y coordinate, 0–255 (top = 0).
    pub y: u16,
}

/// One tick's complete input: the buttons and the stylus, if down.
///
/// The pair that `App::tick` consumes for a [`crate::Frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Input {
    /// Buttons held this tick.
    pub keys: Keys,
    /// Stylus contact, `None` when up.
    pub touch: Option<Touch>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_bits_match_the_register_layout() {
        // REG_KEYXY: A=0 through Y=11, one bit each in order.
        assert_eq!(key::A, 1 << 0);
        assert_eq!(key::B, 1 << 1);
        assert_eq!(key::SELECT, 1 << 2);
        assert_eq!(key::START, 1 << 3);
        assert_eq!(key::RIGHT, 1 << 4);
        assert_eq!(key::LEFT, 1 << 5);
        assert_eq!(key::UP, 1 << 6);
        assert_eq!(key::DOWN, 1 << 7);
        assert_eq!(key::R, 1 << 8);
        assert_eq!(key::L, 1 << 9);
        assert_eq!(key::X, 1 << 10);
        assert_eq!(key::Y, 1 << 11);
    }

    #[test]
    fn keys_queries_hold_state() {
        let keys = Keys(key::A | key::UP | key::START);
        assert!(keys.down(key::A));
        assert!(keys.down(key::A | key::UP));
        assert!(!keys.down(key::A | key::B));
        assert!(keys.any(key::B | key::START));
        assert!(!keys.any(key::B | key::Y));
        assert_eq!(Keys::IDLE, Keys(0));
    }

    #[test]
    fn keys_detect_press_edges() {
        // The press edge: newly held versus the previous tick.
        let prev = Keys(key::A);
        let now = Keys(key::A | key::START);
        let pressed = now.pressed(prev);
        assert!(pressed.down(key::START));
        assert!(!pressed.any(key::A));
        assert_eq!(now.pressed(now), Keys::IDLE);
    }

    #[test]
    fn input_carries_keys_and_touch() {
        let idle = Input::default();
        assert_eq!(idle.keys, Keys::IDLE);
        assert_eq!(idle.touch, None);
        let touched = Input {
            keys: Keys(key::B),
            touch: Some(Touch { x: 128, y: 96 }),
        };
        assert!(touched.keys.down(key::B));
        assert_eq!(touched.touch, Some(Touch { x: 128, y: 96 }));
    }
}
