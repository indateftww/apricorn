//! Map-prop time-of-day visual state — the bucket-driven animation
//! slot swap of `src/field/overlay_01_02204004.c`: `sTimeOfDayVisualState`
//! (`:473`) folds the five [`TimeOfDay`] buckets into four slots,
//! `ov01_02204744` (`FieldSystemUnkSub104_Init`) stores `GF_RTC_GetTimeOfDay()` (`:441`),
//! and the per-frame `ov01_022047DC` (`:481`,
//! `FieldSystemUnkSub104_SwitchTimeOfDay`, called from
//! `fieldmap.c:422` right after the area light update) compares the
//! *bucket* — not the slot — with the stored one: on any change it
//! `MapPropAnimation_RemoveFromRenderObj`s the old bucket's slot and
//! `AddToRenderObj`s the new bucket's for every registered prop (up to
//! four props of up to four animations each, `ov01_0220476C`).
//! `ov01_02204834` (`:496`) reports the current slot.
//!
//! Because the comparison is on the bucket, the midnight `NITE →
//! LATE` edge fires a swap whose two slots are the same (3 → 3): a
//! remove-and-add of one animation, visually a no-op. The state
//! machine here reports that edge like any other and lets the caller
//! ask whether the slot actually moved.
use crate::rtc::{RtcClock, TimeOfDay};

/// The number of animation slots a time-of-day prop carries.
pub const VISUAL_STATES: usize = 4;

/// `sTimeOfDayVisualState[RTC_TIMEOFDAY_COUNT]`: `MORN` → 0, `DAY` →
/// 1, `EVE` → 2, `NITE` and `LATE` → 3.
#[must_use]
pub fn visual_state(time_of_day: TimeOfDay) -> u8 {
    match time_of_day {
        TimeOfDay::Morn => 0,
        TimeOfDay::Day => 1,
        TimeOfDay::Eve => 2,
        TimeOfDay::Nite | TimeOfDay::Late => 3,
    }
}

/// One bucket edge: what `ov01_022047DC` removed and added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BucketChange {
    /// The bucket the state held before the edge.
    pub previous: TimeOfDay,
    /// The bucket it holds now.
    pub current: TimeOfDay,
}

impl BucketChange {
    /// The slot removed from every registered prop.
    #[must_use]
    pub fn previous_slot(&self) -> u8 {
        visual_state(self.previous)
    }

    /// The slot added to every registered prop.
    #[must_use]
    pub fn slot(&self) -> u8 {
        visual_state(self.current)
    }

    /// Whether the props actually show something new — false on the
    /// midnight `NITE → LATE` edge.
    #[must_use]
    pub fn slot_changed(&self) -> bool {
        self.previous_slot() != self.slot()
    }
}

/// The stored `timeOfDay` of pret's `FieldSystemUnkSub104` and its
/// per-frame comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeOfDayState {
    current: TimeOfDay,
}

impl TimeOfDayState {
    /// The state as the field creates it: the bucket in force at map
    /// load (`unk104->timeOfDay = GF_RTC_GetTimeOfDay()`).
    #[must_use]
    pub fn new(initial: TimeOfDay) -> Self {
        Self { current: initial }
    }

    /// The bucket the props currently show.
    #[must_use]
    pub fn current(&self) -> TimeOfDay {
        self.current
    }

    /// `ov01_02204834`: the slot the props currently show.
    #[must_use]
    pub fn visual_state(&self) -> u8 {
        visual_state(self.current)
    }

    /// `ov01_022047DC`, once per field frame with the game's current
    /// bucket: `Some` (and the stored bucket replaced) exactly when it
    /// differs from the stored one.
    pub fn update(&mut self, now: TimeOfDay) -> Option<BucketChange> {
        if now == self.current {
            return None;
        }
        let change = BucketChange {
            previous: self.current,
            current: now,
        };
        self.current = now;
        Some(change)
    }

    /// [`Self::update`] with the bucket the game observes on `frame`
    /// — the clock's cached hardware read
    /// ([`RtcClock::observed_at_frame`]), as `GF_RTC_GetTimeOfDay`
    /// reads it.
    pub fn tick(&mut self, clock: &RtcClock, frame: u32) -> Option<BucketChange> {
        self.update(clock.observed_at_frame(frame).time_of_day())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtc::RtcDateTime;

    #[test]
    fn visual_state_table_folds_night_and_late() {
        assert_eq!(visual_state(TimeOfDay::Morn), 0);
        assert_eq!(visual_state(TimeOfDay::Day), 1);
        assert_eq!(visual_state(TimeOfDay::Eve), 2);
        assert_eq!(visual_state(TimeOfDay::Nite), 3);
        assert_eq!(visual_state(TimeOfDay::Late), 3);
        for index in 0..TimeOfDay::COUNT {
            let bucket = TimeOfDay::from_index(index).expect("bucket");
            assert!(usize::from(visual_state(bucket)) < VISUAL_STATES);
        }
    }

    #[test]
    fn update_reports_bucket_edges_only() {
        let mut state = TimeOfDayState::new(TimeOfDay::Late);
        assert_eq!(state.visual_state(), 3);
        assert_eq!(state.update(TimeOfDay::Late), None);
        let dawn = state.update(TimeOfDay::Morn).expect("edge");
        assert_eq!(
            dawn,
            BucketChange {
                previous: TimeOfDay::Late,
                current: TimeOfDay::Morn
            }
        );
        assert_eq!((dawn.previous_slot(), dawn.slot()), (3, 0));
        assert!(dawn.slot_changed());
        assert_eq!(state.current(), TimeOfDay::Morn);
        assert_eq!(state.update(TimeOfDay::Morn), None);
        // A jump straight to night is one edge, slot 0 → 3.
        let dusk = state.update(TimeOfDay::Nite).expect("edge");
        assert_eq!((dusk.previous_slot(), dusk.slot()), (0, 3));
        // Midnight: the bucket changes, the slot does not.
        let midnight = state.update(TimeOfDay::Late).expect("edge");
        assert!(!midnight.slot_changed());
        assert_eq!((midnight.previous_slot(), midnight.slot()), (3, 3));
    }

    #[test]
    fn ticking_a_day_of_frames_yields_the_five_edges_at_poll_frames() {
        // Start one minute before dawn so the first edge is near.
        let clock = RtcClock::new(RtcDateTime::new(2010, 3, 14, 0, 3, 59, 0));
        let mut state = TimeOfDayState::new(clock.observed_at_frame(0).time_of_day());
        assert_eq!(state.current(), TimeOfDay::Late);
        // The hardware turns 04:00:00 at this frame...
        let dawn_hw = RtcClock::first_frame_of_second(60) as u32;
        // ...and the game notices at the first poll frame at or after it.
        let mut dawn_seen = dawn_hw;
        while RtcClock::poll_frame(dawn_seen) != dawn_seen {
            dawn_seen += 1;
        }
        let day_frames = RtcClock::first_frame_of_second(86_400 + 60) as u32;
        let mut edges = Vec::new();
        for frame in 1..=day_frames {
            if let Some(change) = state.tick(&clock, frame) {
                edges.push((frame, change));
            }
        }
        let buckets: Vec<(TimeOfDay, TimeOfDay)> =
            edges.iter().map(|(_, c)| (c.previous, c.current)).collect();
        use TimeOfDay::*;
        assert_eq!(
            buckets,
            [(Late, Morn), (Morn, Day), (Day, Eve), (Eve, Nite), (Nite, Late)]
        );
        assert_eq!(edges[0].0, dawn_seen);
        assert!(dawn_seen >= dawn_hw && dawn_seen < dawn_hw + RtcClock::POLL_PERIOD);
        // Every edge lands on a poll frame, and only the last is a
        // slot no-op.
        for (frame, change) in &edges {
            assert_eq!(RtcClock::poll_frame(*frame), *frame);
            assert_eq!(change.slot_changed(), change.current != Late);
        }
    }
}
