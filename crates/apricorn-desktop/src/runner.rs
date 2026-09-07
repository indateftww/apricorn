//! The fixed-timestep pacer — the desktop shell's clock discipline.
//!
//! The DS refreshes its LCDs at ~59.8268 Hz (GBATEK's figure — a tick
//! every 1 / 59.8268 s ≈ 16.7149 ms, slightly slower than 60 Hz). The engine
//! itself never sees the wall clock — ticks are pure functions of the
//! frame index — so this module only decides *when* to emit ticks,
//! never *what* a tick computes.
//!
//! The contract is the classic game-loop accumulator: every poll, the
//! elapsed wall time is added to a carry, and one tick is consumed per
//! elapsed period — clamped to [`MAX_TICKS_PER_POLL`] so a stall
//! (window drag, debugger pause) unwinds over a few frames instead of
//! a "spiral of death" where ticking takes longer than the tick period
//! and the debt grows without bound. When the clamp bites, the debt
//! is discarded: the chain catches up to *now*, not to the paused
//! interval, exactly as an emulator does.

use std::time::{Duration, Instant};

/// One tick's wall-clock period: the DS video refresh, GBATEK's
/// 59.8268 Hz (slightly *under* 60 — a real minute runs ~1.7 frames
/// short), i.e. 1 / 59.8268 s ≈ 16.7149 ms rounded to nanoseconds
/// (the reciprocal is not a const fn).
pub const TICK_PERIOD: Duration = Duration::from_nanos(16_714_917);

/// The catch-up clamp: at most this many ticks per poll, however long
/// the stall was. Four is the plan's spiral-of-death guard.
pub const MAX_TICKS_PER_POLL: u32 = 4;

/// The accumulator: wall-time elapsed but not yet spent on ticks.
#[derive(Debug)]
pub struct Pacer {
    /// The last poll's [`Instant`] — the origin the next poll measures from.
    last: Instant,
    /// Elapsed-but-unticked time. Bounded to one period after a
    /// clamped poll (see [`Pacer::poll`]).
    carry: Duration,
}

impl Pacer {
    /// Starts the clock at `now` with no carried debt.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            last: now,
            carry: Duration::ZERO,
        }
    }

    /// How many ticks `now` has earned since the last poll, clamped to
    /// [`MAX_TICKS_PER_POLL`].
    ///
    /// When the clamp bites, the excess debt is dropped (the carry
    /// keeps at most one full period): the chain catches up to *now*,
    /// not to however long the stall lasted.
    pub fn poll(&mut self, now: Instant) -> u32 {
        let elapsed = now.saturating_duration_since(self.last);
        // A backwards poll (clock skew) is ignored entirely: no debt
        // is paid and the origin does not move.
        self.last = self.last.max(now);
        self.carry += elapsed;

        let mut ticks = 0;
        while ticks < MAX_TICKS_PER_POLL && self.carry >= TICK_PERIOD {
            self.carry -= TICK_PERIOD;
            ticks += 1;
        }
        // The clamp bit (a stall longer than four ticks): the debt is
        // discarded, keeping at most one period so the next poll still
        // ticks immediately if the stall just ended.
        if ticks == MAX_TICKS_PER_POLL {
            self.carry = self.carry.min(TICK_PERIOD);
        }
        ticks
    }

    /// The earliest [`Instant`] the next tick becomes due — the
    /// winit `WaitUntil` deadline, so the event loop sleeps between
    /// frames instead of spinning.
    ///
    /// Only meaningful after a [`Pacer::poll`] that returned no
    /// ticks (the carry is then below one period); otherwise it is
    /// `now` itself — the loop should poll again immediately.
    pub fn next_deadline(&self, now: Instant) -> Instant {
        if self.carry >= TICK_PERIOD {
            now
        } else {
            now + (TICK_PERIOD - self.carry)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One poll per period, exactly one tick each — the steady state.
    #[test]
    fn steady_cadence_emits_one_tick_per_period() {
        let start = Instant::now();
        let mut pacer = Pacer::new(start);
        for i in 1..10 {
            let now = start + TICK_PERIOD * i;
            assert_eq!(pacer.poll(now), 1, "poll {i} earns exactly one tick");
        }
    }

    /// A short poll banks a partial period and pays it out when the
    /// sum crosses the threshold — no tick is lost to rounding.
    #[test]
    fn partial_periods_bank_and_pay_out() {
        let start = Instant::now();
        let mut pacer = Pacer::new(start);
        // Thirds (not halves — `Duration / 2` truncates a nanosecond
        // odd period in two, and the sum would miss the threshold).
        let third = TICK_PERIOD / 3;
        assert_eq!(
            pacer.poll(start + third),
            0,
            "a third of a period: no tick yet"
        );
        assert_eq!(
            pacer.poll(start + third * 2),
            0,
            "two thirds banked: still no tick"
        );
        assert_eq!(
            pacer.poll(start + TICK_PERIOD),
            1,
            "the banked thirds complete the first tick"
        );
        assert_eq!(
            pacer.poll(start + TICK_PERIOD * 2),
            1,
            "a full period later, the next tick"
        );
    }

    /// A long stall pays at most [`MAX_TICKS_PER_POLL`] ticks and
    /// keeps at most one period of debt.
    #[test]
    fn long_stalls_clamp_and_discard_debt() {
        let start = Instant::now();
        let mut pacer = Pacer::new(start);
        // 100 periods stalled: the poll pays four ticks…
        assert_eq!(pacer.poll(start + TICK_PERIOD * 100), MAX_TICKS_PER_POLL);
        // …and the excess debt is discarded, keeping one period: the
        // next poll one period later earns that kept tick plus the
        // new period — two, not 97 more.
        assert_eq!(pacer.poll(start + TICK_PERIOD * 101), 2);
        // And the debt is spent: the steady cadence resumes.
        assert_eq!(pacer.poll(start + TICK_PERIOD * 102), 1);
    }

    /// A poll *before* the last (clock skew) is not owed any ticks and
    /// must not go backwards.
    #[test]
    fn backwards_polls_owe_nothing() {
        let start = Instant::now();
        let mut pacer = Pacer::new(start);
        assert_eq!(pacer.poll(start - TICK_PERIOD), 0);
        assert_eq!(pacer.poll(start), 0);
        assert_eq!(pacer.poll(start + TICK_PERIOD), 1);
    }

    /// The sleep deadline is the *remaining* fraction of the period,
    /// one period out after a fresh poll.
    #[test]
    fn deadlines_sleep_for_the_remaining_period() {
        let start = Instant::now();
        let mut pacer = Pacer::new(start);
        assert_eq!(pacer.next_deadline(start), start + TICK_PERIOD);
        // Half a period banked: only the other half remains.
        assert_eq!(pacer.poll(start + TICK_PERIOD / 2), 0);
        assert_eq!(
            pacer.next_deadline(start + TICK_PERIOD / 2),
            start + TICK_PERIOD
        );
        // The tick paid out: the next is a full period out.
        assert_eq!(pacer.poll(start + TICK_PERIOD), 1);
        assert_eq!(
            pacer.next_deadline(start + TICK_PERIOD),
            start + TICK_PERIOD * 2
        );
    }
}
