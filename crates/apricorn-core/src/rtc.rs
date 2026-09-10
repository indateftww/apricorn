//! The real-time clock — pinned, and the RNG seed derived from it.
//!
//! pret's RTC stack (`src/gf_rtc.c` over the NitroSDK's `RTCDate`/
//! `RTCTime`) polls the hardware clock asynchronously and refreshes
//! its copy roughly every eleventh frame (`GF_RTC_UpdateOnFrame`), so
//! `GF_RTC_CopyDateTime` returns whatever the hardware said most
//! recently. The engine can't read hardware: [`RtcDateTime`] is the
//! *pinned* clock the runner hands the game — a frozen snapshot that
//! models the hardware reading at boot and never advances, the
//! determinism the harness needs (the same pinned clock replays the
//! same flow) and the desktop wants (no wall-clock ever feeds state).
//!
//! One consumer of the fields is time-of-day behavior — Oak's intro
//! greeting picks its message by `hour * 100 + minute`
//! (`OakSpeech_GetTimeOfDayIntroMsg`, `src/oaks_speech.c`). The other
//! is the boot RNG seed: [`RtcDateTime::rng_seed`] is
//! [`RngSeedFromRTC()`](gf_rtc.h) verbatim, wrapping 32-bit arithmetic
//! over the date/time fields plus the vblank counter, and the
//! game-state machine runs it at pret's `InitializeMainRNG` points
//! (`src/main.c`, and again at each `ov36` overlay init).

/// The pinned date and time — the NitroSDK's `RTCDate` and `RTCTime`
/// field-for-field (`lib/include/nitro/rtc/ARM9/api.h`): every field
/// is the SDK's `u32`, `week` the SDK's `RTCWeek` day-of-week number.
///
/// ```
/// use apricorn_core::rtc::RtcDateTime;
///
/// // The frozen clock the desktop pins: HG's US release date, noon.
/// let rtc = RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0);
/// assert_eq!(rtc.rng_seed(0), 10 + 0x2A00_0000 + 12 * 0x1_0000);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcDateTime {
    /// Calendar year, e.g. `2010`; SDK reads use years since 2000.
    pub year: u32,
    /// `RTCDate::month` — 1–12.
    pub month: u32,
    /// `RTCDate::day` — 1–31.
    pub day: u32,
    /// `RTCDate::week` — the SDK's day-of-week number.
    pub week: u32,
    /// `RTCTime::hour` — 0–23.
    pub hour: u32,
    /// `RTCTime::minute` — 0–59.
    pub minute: u32,
    /// `RTCTime::second` — 0–59.
    pub second: u32,
}

impl RtcDateTime {
    /// The pinned clock over the SDK's fields, in `RTCDate` then
    /// `RTCTime` order.
    #[must_use]
    pub fn new(
        year: u32,
        month: u32,
        day: u32,
        week: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Self {
        Self {
            year,
            month,
            day,
            week,
            hour,
            minute,
            second,
        }
    }

    /// `RngSeedFromRTC()` (`include/gf_rtc.h`) — the boot RNG seed,
    /// transcribed statement-for-statement in the original's
    /// left-to-right shape (the C's `month * 0x100 * day * 0x10000`
    /// term is not folded, though wrapping `u32` multiplication makes
    /// the two orders equal):
    ///
    /// ```c
    /// date.year + date.month * 0x100 * date.day * 0x10000
    ///         + time.hour * 0x10000
    ///         + (time.minute + time.second) * 0x1000000
    ///         + gSystem.vblankCounter
    /// ```
    ///
    /// `vblank_counter` is the vblanks since boot — pret increments
    /// it once per main-loop frame and calls `InitializeMainRNG`
    /// before the loop (counter 0), so the engine passes the tick's
    /// [`Frame`](crate::Frame) index. The macro's `week` field is not
    /// read.
    #[must_use]
    pub fn rng_seed(&self, vblank_counter: u32) -> u32 {
        (self.year % 100)
            .wrapping_add(
                self.month
                    .wrapping_mul(0x100)
                    .wrapping_mul(self.day)
                    .wrapping_mul(0x1_0000),
            )
            .wrapping_add(self.hour.wrapping_mul(0x1_0000))
            .wrapping_add(
                self.minute
                    .wrapping_add(self.second)
                    .wrapping_mul(0x100_0000),
            )
            .wrapping_add(vblank_counter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_matches_the_macro_term_by_term() {
        // Hand-evaluated from the C, term by term:
        //   SDK year 10                     = 0x00A
        // + month * 0x100 * day * 0x10000   = 3 * 0x100 * 14 * 0x10000
        //   = 10752 * 0x10000               = 0x2A00_0000
        // + hour * 0x10000                  = 0xC_0000
        // + (minute + second) * 0x1000000   = 0
        // + vblankCounter                   = 0
        let rtc = RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0);
        let month_term = 3u32
            .wrapping_mul(0x100)
            .wrapping_mul(14)
            .wrapping_mul(0x1_0000);
        assert_eq!(month_term, 0x2A00_0000);
        let expected = 0x2A0C_000A_u32;
        assert_eq!(rtc.rng_seed(0), expected);

        // The vblank counter lands last, plain wrapping addition.
        assert_eq!(rtc.rng_seed(1234), expected + 1234);
    }

    #[test]
    fn seed_wraps_like_the_c_u32_arithmetic() {
        // The month term is the one that can exceed 2^32: a
        // late-December date makes 12 * 0x100 * 31 * 0x10000 =
        // 0x1_7400_0000, which wraps to 0x7400_0000 (the minute term
        // tops out at (59 + 59) * 0x1000000 = 0x7600_0000, in range).
        let rtc = RtcDateTime::new(2000, 12, 31, 0, 23, 59, 59);
        // SDK year 0 + month 0x7400_0000 + hour 0x17_0000
        // + minute 0x7600_0000
        assert_eq!(rtc.rng_seed(0), 0xEA17_0000);
        // The vblank counter wraps with the whole sum, not saturating.
        assert_eq!(rtc.rng_seed(u32::MAX), 0xEA16_FFFF);
    }

    #[test]
    fn week_is_not_in_the_seed() {
        // RngSeedFromRTC never reads date.week; two clocks differing
        // only in the day-of-week seed identically.
        let monday = RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0);
        let tuesday = RtcDateTime::new(2010, 3, 14, 1, 12, 0, 0);
        assert_eq!(monday.rng_seed(7), tuesday.rng_seed(7));
    }
}
