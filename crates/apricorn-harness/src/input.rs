//! Input scripts (`.apin`) — what both sides replay.
//!
//! A frame-timed button/stylus script. One state machine
//! ([`InputScript::at`]) is the single source of truth: the oracle
//! compiles it ahead of time to a flat frame/mask blob, the headless
//! engine (Phase 4) will evaluate it per frame, and neither side can
//! interpret input differently.
//!
//! ```text
//! # apricorn input v1
//! rtc 2010-03-01T09:00:00
//! end 600
//! 120 down A
//! 150 up A
//! 200 down UP+B
//! 240 up UP+B
//! 300 touch 128 96
//! 310 lift
//! ```
//!
//! Grammar, line-oriented:
//!
//! * `# …` — full-line comments, skipped.
//! * `rtc <timestamp>` — the case's pinned RTC (optional, at most once);
//!   the oracle runs its clock from this value, the engine sets its
//!   model clock from it.
//! * `end <frames>` — the replay length in frames (required, exactly
//!   once). Every event must fall before it.
//! * `<frame> down <BTN+BTN…>` — press buttons at that frame.
//! * `<frame> up <BTN+BTN…>` — release buttons.
//! * `<frame> touch <x> <y>` — stylus down at `(x, y)` (both 0-255).
//! * `<frame> lift` — stylus up.
//!
//! Buttons are the twelve NDS keys (`A B X Y L R START SELECT UP DOWN
//! LEFT RIGHT`); the mask bit for each is the hardware `REG_KEYXY` bit
//! (`A` = `1<<0` through `Y` = `1<<11`), with **bit set = held** (the
//! hardware register is active-low; each producer inverts at the edge).
//! Events may appear in any file order; within a frame they apply in
//! file order. [`InputScript::at`] is the state after applying every
//! event at or before the given frame.
//!
//! The canonical form ([`std::fmt::Display`], round-trip-tested) is
//! what `input-sha1` in a trace header hashes — comments and spacing
//! never change a case's identity.

use crate::HarnessError;
use sha1::{Digest, Sha1};

/// A button's mask bit, per the hardware `REG_KEYXY` layout.
#[must_use]
pub fn button_mask(name: &str) -> Option<u16> {
    let bit = match name {
        "A" => 0,
        "B" => 1,
        "SELECT" => 2,
        "START" => 3,
        "RIGHT" => 4,
        "LEFT" => 5,
        "UP" => 6,
        "DOWN" => 7,
        "R" => 8,
        "L" => 9,
        "X" => 10,
        "Y" => 11,
        _ => return None,
    };
    Some(1 << bit)
}

/// One scripted input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Press the buttons in the mask.
    Down(u16),
    /// Release the buttons in the mask.
    Up(u16),
    /// Stylus down at (x, y).
    Touch(u16, u16),
    /// Stylus up.
    Lift,
}

impl Action {
    /// The canonical spelling of one event (no frame prefix).
    #[must_use]
    pub fn as_str(self) -> String {
        match self {
            Action::Down(mask) => format!("down {}", spell_mask(mask)),
            Action::Up(mask) => format!("up {}", spell_mask(mask)),
            Action::Touch(x, y) => format!("touch {x} {y}"),
            Action::Lift => "lift".to_string(),
        }
    }
}

/// The buttons held in `mask`, in canonical order, joined with `+`.
fn spell_mask(mask: u16) -> String {
    const NAMES: [&str; 12] = [
        "A", "B", "SELECT", "START", "RIGHT", "LEFT", "UP", "DOWN", "R", "L", "X", "Y",
    ];
    let mut names = Vec::new();
    for (bit, name) in NAMES.iter().enumerate() {
        if mask & (1 << bit) != 0 {
            names.push(*name);
        }
    }
    names.join("+")
}

/// An event: something that happens at a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    /// The frame the event applies at.
    pub frame: u32,
    /// What happens.
    pub action: Action,
}

/// A frame's input state — the output of [`InputScript::at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInput {
    /// Buttons held (bit set = held, [`button_mask`] layout).
    pub mask: u16,
    /// Stylus contact, if down.
    pub touch: Option<(u16, u16)>,
}

impl FrameInput {
    /// No buttons, stylus up — the state before any event.
    pub const IDLE: Self = Self {
        mask: 0,
        touch: None,
    };
}

/// A parsed `.apin` input script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputScript {
    /// The pinned RTC, if the case sets one.
    pub rtc: Option<String>,
    /// The replay length in frames.
    pub end: u32,
    /// The events, sorted by frame (stable: file order within a frame).
    pub events: Vec<InputEvent>,
}

impl InputScript {
    /// Parses `.apin` text.
    ///
    /// # Errors
    /// Returns a [`HarnessError::Syntax`] naming the line of any
    /// malformed event, unknown button, out-of-range coordinate, missing
    /// or duplicated `end`/`rtc`, or event at or past `end`.
    pub fn parse(text: &str) -> Result<Self, HarnessError> {
        let mut rtc: Option<String> = None;
        let mut end: Option<u32> = None;
        let mut events = Vec::new();

        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = |what: String| HarnessError::Syntax {
                line: line_no,
                what,
            };

            let mut fields = line.split_whitespace();
            let first = fields.next().unwrap_or_default();

            // Header directives: rtc, end.
            if let Some(value) = line.strip_prefix("rtc ") {
                if rtc.is_some() {
                    return Err(bad("duplicate rtc".to_string()));
                }
                rtc = Some(value.trim().to_string());
                continue;
            }
            if let Some(value) = line.strip_prefix("end ") {
                if end.is_some() {
                    return Err(bad("duplicate end".to_string()));
                }
                end = Some(
                    value
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| bad("end: not a number".to_string()))?,
                );
                continue;
            }

            // Events: `<frame> <action> …`.
            let frame = first
                .parse::<u32>()
                .map_err(|_| bad(format!("not a frame number or directive: '{first}'")))?;
            let action = match (fields.next(), fields.next(), fields.next()) {
                (Some("down"), Some(buttons), None) => Action::Down(
                    parse_buttons(buttons)
                        .ok_or_else(|| bad(format!("unknown button(s): '{buttons}'")))?,
                ),
                (Some("up"), Some(buttons), None) => Action::Up(
                    parse_buttons(buttons)
                        .ok_or_else(|| bad(format!("unknown button(s): '{buttons}'")))?,
                ),
                (Some("touch"), Some(x), Some(y)) => {
                    let x = x
                        .parse::<u16>()
                        .map_err(|_| bad("touch: x not a number".to_string()))?;
                    let y = y
                        .parse::<u16>()
                        .map_err(|_| bad("touch: y not a number".to_string()))?;
                    if x > 255 || y > 255 {
                        return Err(bad("touch: coordinates must be 0-255".to_string()));
                    }
                    Action::Touch(x, y)
                }
                (Some("lift"), None, None) => Action::Lift,
                _ => {
                    return Err(bad(
                        "expected: <frame> down BTN+… | up BTN+… | touch x y | lift".to_string(),
                    ));
                }
            };
            events.push(InputEvent { frame, action });
        }

        let end = end.ok_or(HarnessError::Syntax {
            line: 1,
            what: "missing end (the replay length in frames)".to_string(),
        })?;
        // Every event must fall inside the replay.
        if let Some(event) = events.iter().find(|e| e.frame >= end) {
            return Err(HarnessError::Syntax {
                line: 1,
                what: format!("event at frame {} is not before end {end}", event.frame),
            });
        }
        // Stable sort by frame: file order is preserved within a frame.
        events.sort_by_key(|e| e.frame);

        Ok(Self { rtc, end, events })
    }

    /// The input state at `frame`: the initial idle state with every
    /// event at or before `frame` applied in order.
    ///
    /// Frames past `end` clamp to the state at `end` (a replay never
    /// steps there; the clamp only keeps lookups total).
    #[must_use]
    pub fn at(&self, frame: u32) -> FrameInput {
        let mut state = FrameInput::IDLE;
        for event in &self.events {
            if event.frame > frame {
                break;
            }
            match event.action {
                Action::Down(mask) => state.mask |= mask,
                Action::Up(mask) => state.mask &= !mask,
                Action::Touch(x, y) => state.touch = Some((x, y)),
                Action::Lift => state.touch = None,
            }
        }
        state
    }

    /// SHA-1 over the canonical form, as lowercase hex — the
    /// `input-sha1` a trace header carries.
    #[must_use]
    pub fn sha1_hex(&self) -> String {
        let mut hasher = Sha1::new();
        hasher.update(self.to_string().as_bytes());
        let digest = hasher.finalize();
        let mut out = String::with_capacity(40);
        for b in digest {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }
}

/// Parses a `BTN+BTN…` group into its mask.
fn parse_buttons(text: &str) -> Option<u16> {
    let mut mask = 0;
    for name in text.split('+') {
        mask |= button_mask(name)?;
    }
    Some(mask)
}

impl std::fmt::Display for InputScript {
    /// The canonical form: `rtc`, `end`, then events in frame order,
    /// each as `<frame> <action>`. Re-parsing the output yields the same
    /// [`InputScript`] (round-trip-tested).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "# apricorn input v1")?;
        if let Some(rtc) = &self.rtc {
            writeln!(f, "rtc {rtc}")?;
        }
        writeln!(f, "end {}", self.end)?;
        for event in &self.events {
            writeln!(f, "{} {}", event.frame, event.action.as_str())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example script from the module docs.
    const SCRIPT: &str = "# apricorn input v1\n\
                          rtc 2010-03-01T09:00:00\n\
                          end 600\n\
                          \n\
                          120 down A\n\
                          150 up A\n\
                          200 down UP+B\n\
                          240 up UP+B\n\
                          300 touch 128 96\n\
                          310 lift\n";

    #[test]
    fn parses_and_evaluates() {
        let script = InputScript::parse(SCRIPT).expect("fixture must parse");
        assert_eq!(script.rtc.as_deref(), Some("2010-03-01T09:00:00"));
        assert_eq!(script.end, 600);
        assert_eq!(script.events.len(), 6);

        // Before any event: idle.
        assert_eq!(script.at(119), FrameInput::IDLE);
        // A is held from 120 until released at 150.
        assert_eq!(script.at(120).mask, button_mask("A").unwrap());
        assert_eq!(script.at(149).mask, button_mask("A").unwrap());
        assert_eq!(script.at(150).mask, 0);
        // UP+B is two bits held together.
        let up_b = button_mask("UP").unwrap() | button_mask("B").unwrap();
        assert_eq!(script.at(200).mask, up_b);
        assert_eq!(script.at(239).mask, up_b);
        assert_eq!(script.at(240).mask, 0);
        // Stylus down at 300, up at 310.
        assert_eq!(script.at(299).touch, None);
        assert_eq!(script.at(300).touch, Some((128, 96)));
        assert_eq!(script.at(309).touch, Some((128, 96)));
        assert_eq!(script.at(310).touch, None);
        // Events hold their state to the end of the replay.
        assert_eq!(script.at(599), FrameInput::IDLE);
    }

    #[test]
    fn mask_bits_match_hardware_layout() {
        // REG_KEYXY bit order (bit set = held here; hardware is active-low).
        assert_eq!(button_mask("A"), Some(1 << 0));
        assert_eq!(button_mask("B"), Some(1 << 1));
        assert_eq!(button_mask("SELECT"), Some(1 << 2));
        assert_eq!(button_mask("START"), Some(1 << 3));
        assert_eq!(button_mask("RIGHT"), Some(1 << 4));
        assert_eq!(button_mask("LEFT"), Some(1 << 5));
        assert_eq!(button_mask("UP"), Some(1 << 6));
        assert_eq!(button_mask("DOWN"), Some(1 << 7));
        assert_eq!(button_mask("R"), Some(1 << 8));
        assert_eq!(button_mask("L"), Some(1 << 9));
        assert_eq!(button_mask("X"), Some(1 << 10));
        assert_eq!(button_mask("Y"), Some(1 << 11));
        assert_eq!(button_mask("Z"), None);
        assert_eq!(parse_buttons("UP+B"), Some((1 << 6) | (1 << 1)));
    }

    #[test]
    fn later_events_win_within_a_frame() {
        // Within one frame, file order applies — here the press wins
        // only because it is listed after the release.
        let text = "end 10\n5 down A\n5 up A\n5 down A\n";
        let script = InputScript::parse(text).expect("fixture must parse");
        assert_eq!(script.at(4).mask, 0);
        assert_eq!(script.at(5).mask, button_mask("A").unwrap());
        assert_eq!(script.at(6).mask, button_mask("A").unwrap());

        // Same fixture written in a different file order is a different
        // script: `up` last releases within the frame.
        let text = "end 10\n5 down A\n5 down A\n5 up A\n";
        let script = InputScript::parse(text).expect("fixture must parse");
        assert_eq!(script.at(5).mask, 0);
    }

    #[test]
    fn round_trips_byte_exact() {
        let script = InputScript::parse(SCRIPT).expect("fixture must parse");
        let text = script.to_string();
        assert_eq!(
            InputScript::parse(&text).expect("writer output must parse"),
            script
        );
        // Reverse direction: canonical text is a fixed point.
        let reparsed = InputScript::parse(&text).expect("fixture must parse");
        assert_eq!(reparsed.to_string(), text);

        // Comments and spacing never change the case's identity: the
        // canonical form (and thus input-sha1) is the semantic identity.
        let respaced = "# pad\n\n\nrtc 2010-03-01T09:00:00\nend 600\n120  down   A\n150 up A\n200 down UP+B\n240 up UP+B\n300 touch 128 96\n310 lift\n";
        let other = InputScript::parse(respaced).expect("respaced must parse");
        assert_eq!(other.sha1_hex(), script.sha1_hex());
        assert_eq!(other.sha1_hex().len(), 40);
    }

    #[test]
    fn rejects_broken_scripts() {
        // Missing end.
        let err = InputScript::parse("120 down A\n").expect_err("must fail");
        assert_eq!(
            err,
            HarnessError::Syntax {
                line: 1,
                what: "missing end (the replay length in frames)".to_string()
            }
        );
        // Unknown button.
        let err = InputScript::parse("end 10\n5 down Q\n").expect_err("must fail");
        assert_eq!(
            err,
            HarnessError::Syntax {
                line: 2,
                what: "unknown button(s): 'Q'".to_string()
            }
        );
        // Out-of-range stylus.
        assert!(InputScript::parse("end 10\n5 touch 300 96\n").is_err());
        assert!(InputScript::parse("end 10\n5 touch 128\n").is_err());
        // Event at or past end.
        assert!(InputScript::parse("end 10\n10 down A\n").is_err());
        assert!(InputScript::parse("end 10\n11 down A\n").is_err());
        // Duplicated directives.
        assert!(InputScript::parse("end 10\nend 20\n").is_err());
        assert!(InputScript::parse("rtc x\nrtc y\nend 10\n").is_err());
        // Not a frame.
        assert!(InputScript::parse("end 10\nfive down A\n").is_err());
        // Unknown action.
        assert!(InputScript::parse("end 10\n5 press A\n").is_err());
    }

    #[test]
    fn empty_script_is_idle_everywhere() {
        let script = InputScript::parse("end 3\n").expect("empty must parse");
        assert_eq!(script.events.len(), 0);
        assert_eq!(script.at(0), FrameInput::IDLE);
        assert_eq!(script.at(2), FrameInput::IDLE);
        // Past end the state clamps to the last event's aftermath.
        assert_eq!(script.at(999), FrameInput::IDLE);
    }
}
