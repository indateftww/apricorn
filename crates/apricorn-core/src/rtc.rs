//! The real-time clock — pinned at boot, advancing with the frame
//! index, and the RNG seed derived from it.
//!
//! pret's RTC stack (`src/gf_rtc.c` over the NitroSDK's `RTCDate`/
//! `RTCTime`) polls the hardware clock asynchronously and refreshes
//! its copy every eleventh frame (`GF_RTC_UpdateOnFrame`), so
//! `GF_RTC_CopyDateTime` returns whatever the hardware said most
//! recently. The engine can't read hardware: [`RtcDateTime`] is the
//! *pinned* clock the runner hands the game — the snapshot that models
//! the hardware reading at boot — and [`RtcClock`] is the hardware
//! itself as the oracle emulates it: the pinned start plus exactly the
//! seconds the emulated cycles have counted by a given frame. Both are
//! pure functions of the frame index, the determinism the harness
//! needs (the same pinned clock replays the same flow) and the desktop
//! wants (no wall-clock ever feeds state).
//!
//! Consumers of the fields: Oak's intro greeting picks its message by
//! `hour * 100 + minute` (`OakSpeech_GetTimeOfDayIntroMsg`,
//! `src/oaks_speech.c`); the field's day/night look, wild-encounter
//! slot and evolution checks read the hour bucket
//! ([`TimeOfDay`], `GF_RTC_GetTimeOfDayByHour`); the daily-event
//! bookkeeping compares day numbers ([`RtcDateTime::day_number`],
//! the SDK's `RTC_ConvertDateToDay`); and the boot RNG seed
//! [`RtcDateTime::rng_seed`] is [`RngSeedFromRTC()`](gf_rtc.h)
//! verbatim, wrapping 32-bit arithmetic over the date/time fields plus
//! the vblank counter, run at pret's `InitializeMainRNG` points
//! (`src/main.c`, and again at each `ov36` overlay init).
//!
//! `docs/day-night.md` walks the whole model with its evidence.

/// pret `TIMEOFDAY` (`include/gf_rtc.h`): the five hour buckets of
/// `GF_RTC_GetTimeOfDayByHour`'s `sTimeOfDayByHour[24]`
/// (`src/gf_rtc.c`) — hours 0–3 `LATE`, 4–9 `MORN`, 10–16 `DAY`,
/// 17–19 `EVE`, 20–23 `NITE`. The discriminants are the C enum's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum TimeOfDay {
    /// `RTC_TIMEOFDAY_MORN` — 04:00 to 09:59.
    Morn = 0,
    /// `RTC_TIMEOFDAY_DAY` — 10:00 to 16:59.
    Day = 1,
    /// `RTC_TIMEOFDAY_EVE` — 17:00 to 19:59.
    Eve = 2,
    /// `RTC_TIMEOFDAY_NITE` — 20:00 to 23:59.
    Nite = 3,
    /// `RTC_TIMEOFDAY_LATE` — 00:00 to 03:59.
    Late = 4,
}

/// pret `TimeOfDayWildParam` (`include/gf_rtc.h`): the three-way
/// collapse wild encounter tables index by
/// (`GF_RTC_GetTimeOfDayWildParamByHour`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum WildTimeOfDay {
    /// `TIMEOFDAY_WILD_MORN` — the `MORN` bucket.
    Morn = 0,
    /// `TIMEOFDAY_WILD_DAY` — the `DAY` and `EVE` buckets.
    Day = 1,
    /// `TIMEOFDAY_WILD_NITE` — the `NITE` and `LATE` buckets.
    Nite = 2,
}

impl TimeOfDay {
    /// `RTC_TIMEOFDAY_COUNT`.
    pub const COUNT: usize = 5;

    /// `sTimeOfDayByHour[24]` (`src/gf_rtc.c`, `GF_RTC_GetTimeOfDayByHour`).
    const BY_HOUR: [TimeOfDay; 24] = [
        Self::Late,
        Self::Late,
        Self::Late,
        Self::Late,
        Self::Morn,
        Self::Morn,
        Self::Morn,
        Self::Morn,
        Self::Morn,
        Self::Morn,
        Self::Day,
        Self::Day,
        Self::Day,
        Self::Day,
        Self::Day,
        Self::Day,
        Self::Day,
        Self::Eve,
        Self::Eve,
        Self::Eve,
        Self::Nite,
        Self::Nite,
        Self::Nite,
        Self::Nite,
    ];

    /// `GF_RTC_GetTimeOfDayByHour(hour)` — the bucket of an hour.
    ///
    /// # Panics
    /// When `hour` is not 0–23, where the C `GF_ASSERT`s.
    #[must_use]
    pub fn by_hour(hour: u32) -> Self {
        Self::BY_HOUR[hour as usize]
    }

    /// `IsNighttime()`: `NITE` or `LATE`.
    #[must_use]
    pub fn is_night(self) -> bool {
        matches!(self, Self::Nite | Self::Late)
    }

    /// `GF_RTC_GetTimeOfDayWildParamByHour`'s `switch`: `MORN` stays,
    /// `DAY` and `EVE` become the day slot, everything else night.
    #[must_use]
    pub fn wild_param(self) -> WildTimeOfDay {
        match self {
            Self::Morn => WildTimeOfDay::Morn,
            Self::Day | Self::Eve => WildTimeOfDay::Day,
            Self::Nite | Self::Late => WildTimeOfDay::Nite,
        }
    }

    /// The C enum value, for table lookups (`sTimeOfDayVisualState`
    /// and friends).
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    /// The bucket with C enum value `index`, if any.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        [Self::Morn, Self::Day, Self::Eve, Self::Nite, Self::Late]
            .get(index)
            .copied()
    }
}

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

    /// `GF_RTC_GetTimeOfDay()` — the bucket of this clock's hour.
    #[must_use]
    pub fn time_of_day(&self) -> TimeOfDay {
        TimeOfDay::by_hour(self.hour)
    }

    /// `IsNighttime()`.
    #[must_use]
    pub fn is_night(&self) -> bool {
        self.time_of_day().is_night()
    }

    /// `GF_RTC_GetTimeOfDayWildParam()`.
    #[must_use]
    pub fn wild_param(&self) -> WildTimeOfDay {
        self.time_of_day().wild_param()
    }

    /// `GF_RTC_TimeToSec()` — `60 * minute + 3600 * hour + second`, the
    /// seconds since midnight (also the SDK's `RTCi_ConvertTimeToSecond`,
    /// `lib/asm/nitro.s:9190`, which the field lighting halves).
    #[must_use]
    pub fn seconds_of_day(&self) -> u32 {
        60 * self.minute + 3600 * self.hour + self.second
    }

    /// The SDK's `RTCDate::year` — years since 2000 — when the calendar
    /// year lies in the RTC's 2000–2099 range.
    fn sdk_year(&self) -> Option<u32> {
        self.year.checked_sub(2000).filter(|y| *y < 100)
    }

    /// `sGF_DaysPerMonth`-style cumulative days before each month of a
    /// common year (`src/gf_rtc.c`; the SDK's `RTC_ConvertDateToDay`
    /// carries the same table at `0x02110F88`).
    const DAYS_BEFORE_MONTH: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

    /// The SDK's `RTC_ConvertDateToDay` (`lib/asm/nitro.s:9145`):
    /// days since 2000-01-01, or `None` where the SDK returns `-1` —
    /// a year outside 2000–2099, a month outside 1–12, a day outside
    /// 1–31 (the SDK checks no month length), or a week of 7 or more.
    ///
    /// ```text
    /// (day - 1) + DAYS_BEFORE_MONTH[month - 1]
    ///   + (month >= 3 && year % 4 == 0 ? 1 : 0)
    ///   + year * 365 + (year + 3) / 4        // year = years since 2000
    /// ```
    ///
    /// `year % 4` is the whole leap rule because 2000 is a leap year
    /// and 2100 lies outside the range; pret's `GF_RTC_TimeDelta`
    /// asserts the corollary — 2099-12-31 23:59:59 is second
    /// 3155759999, day 36524.
    #[must_use]
    pub fn day_number(&self) -> Option<u32> {
        let year = self.sdk_year()?;
        if !(1..=12).contains(&self.month) || !(1..=31).contains(&self.day) || self.week >= 7 {
            return None;
        }
        let mut days = self.day - 1 + Self::DAYS_BEFORE_MONTH[(self.month - 1) as usize];
        if self.month >= 3 && year % 4 == 0 {
            days += 1;
        }
        Some(year * 365 + days + (year + 3) / 4)
    }

    /// The SDK's `RTC_ConvertDateTimeToSecond` (`lib/asm/nitro.s:9200`):
    /// `day_number * 86400 + seconds_of_day`, the 64-bit count
    /// `GF_RTC_DateTimeToSec` returns; `None` where the SDK returns
    /// `-1` (an invalid date).
    #[must_use]
    pub fn date_time_seconds(&self) -> Option<i64> {
        Some(i64::from(self.day_number()?) * 86_400 + i64::from(self.seconds_of_day()))
    }

    /// `GF_RTC_GetDayOfYear` (`src/gf_rtc.c`): the 1-based day of the
    /// year, leap day included from March on. The C's `IsLeapYear`
    /// runs on the SDK's years-since-2000 (so 2000 is leap through
    /// the `% 400` arm); ported over the same value. Not called by
    /// the retail game (no reference in the image), kept for the
    /// port's completeness.
    #[must_use]
    pub fn day_of_year(&self) -> u32 {
        let year = self.year.wrapping_sub(2000);
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let month = self.month.clamp(1, 12);
        let mut days = self.day + Self::DAYS_BEFORE_MONTH[(month - 1) as usize];
        if month >= 3 && leap {
            days += 1;
        }
        days
    }

    /// The day-of-week the DS firmware (and melonDS's `RTC::SetDateTime`)
    /// derives from the date: `(6 + day_number) % 7`, 0 = Sunday, since
    /// 2000-01-01 was a Saturday. `None` for an invalid date.
    #[must_use]
    pub fn week_from_date(&self) -> Option<u32> {
        Some((6 + self.day_number()?) % 7)
    }

    /// The clock as melonDS's `RTC::SetDateTime` (`src/RTC.cpp`)
    /// stores a pinned start: the year folded into 2000–2099, an
    /// out-of-range month or day (against that year's month length)
    /// reset to 1, an out-of-range hour/minute/second to 0, and the
    /// week *recomputed* from the date (the emulator ignores any
    /// supplied day-of-week).
    #[must_use]
    pub fn sanitized(&self) -> Self {
        let year = 2000 + self.year % 100;
        let month = if (1..=12).contains(&self.month) {
            self.month
        } else {
            1
        };
        let day = if (1..=days_in_month(year, month)).contains(&self.day) {
            self.day
        } else {
            1
        };
        let mut clean = Self {
            year,
            month,
            day,
            week: 0,
            hour: if self.hour < 24 { self.hour } else { 0 },
            minute: if self.minute < 60 { self.minute } else { 0 },
            second: if self.second < 60 { self.second } else { 0 },
        };
        clean.week = clean
            .week_from_date()
            .expect("a sanitized date is always valid");
        clean
    }

    /// The clock `seconds` later, counted the way the DS RTC counts —
    /// melonDS's `RTC::CountSecond` → `CountMinute` → `CountHour` →
    /// `CountDay` → `CountMonth` → `CountYear` chain: 24-hour mode,
    /// the day-of-week a plain mod-7 counter, month lengths by the
    /// `year % 4` leap rule, and the two-digit year wrapping from 99
    /// back to 00 (2099-12-31 rolls into 2000-01-01, a 36525-day
    /// cycle). `None` when the start is not a valid date
    /// ([`Self::day_number`]); [`Self::sanitized`] guarantees one.
    #[must_use]
    pub fn advanced_by(&self, seconds: u64) -> Option<Self> {
        const DAY: u64 = 86_400;
        const CYCLE_DAYS: u64 = 36_525;
        let start_day = u64::from(self.day_number()?);
        let total = u64::from(self.seconds_of_day()) + seconds;
        let crossed = total / DAY;
        let seconds_of_day = total % DAY;
        let day = (start_day + crossed) % CYCLE_DAYS;
        // Years since 2000: 1461-day leap blocks, then within a block
        // the first year is the leap one.
        let (mut year, mut rest) = ((day / 1461) * 4, day % 1461);
        if rest >= 366 {
            year += 1 + (rest - 366) / 365;
            rest = (rest - 366) % 365;
        }
        let year = 2000 + year as u32;
        let mut month = 1;
        loop {
            let length = u64::from(days_in_month(year, month));
            if rest < length || month == 12 {
                break;
            }
            rest -= length;
            month += 1;
        }
        Some(Self {
            year,
            month,
            day: rest as u32 + 1,
            week: ((u64::from(self.week) + crossed) % 7) as u32,
            hour: (seconds_of_day / 3600) as u32,
            minute: (seconds_of_day % 3600 / 60) as u32,
            second: (seconds_of_day % 60) as u32,
        })
    }
}

/// The length of `month` in `year` under the RTC's `year % 4` leap
/// rule (melonDS `RTC::DaysInMonth`; exact for 2000–2099).
fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// The hardware clock as the oracle emulates it: a pinned start that
/// then advances with the emulated cycles, one second per
/// [`Self::CYCLES_PER_SECOND`] system-clock cycles.
///
/// melonDS (`src/RTC.cpp`, `RTC::ScheduleTimer`/`ClockTimer`) runs the
/// RTC crystal at 32768 Hz off the 33513982 Hz system clock with a
/// Bresenham remainder (`delay = (33513982 + err) >> 15`, `err =
/// (33513982 + err) & 0x7FFF`), so tick `k` lands at cycle
/// `floor(k * 33513982 / 32768)` and the second counter increments
/// every 32768 ticks — cycle `s * 33513982` exactly for second `s`.
/// The counter starts at reset (`NDS::Reset` zeroes the timestamps
/// and calls `RTC::Reset`), and the oracle pins the date *after* the
/// reset (`RTC::SetDateTime`, `docs/oracle.md`), which leaves the
/// tick phase untouched. A frame is 263 scanlines of 355 × 6 cycles
/// (`src/GPU.cpp` `LINE_CYCLES`/`FRAME_CYCLES`, `NDS::RunFrame`'s
/// `frametarget`) — [`Self::CYCLES_PER_FRAME`] — so the second count
/// visible at the start of frame `f` is
/// `floor(f * 560190 / 33513982)`: the clock gains its first second
/// at frame 60 (59 × 560190 = 33051210 < 33513982 ≤ 60 × 560190).
///
/// ```
/// use apricorn_core::rtc::{RtcClock, RtcDateTime};
///
/// let clock = RtcClock::new(RtcDateTime::new(2010, 3, 1, 0, 9, 0, 0));
/// assert_eq!(clock.at_frame(59).second, 0);
/// assert_eq!(clock.at_frame(60).second, 1);
/// // 2010-03-01 was a Monday; the pinned week is recomputed.
/// assert_eq!(clock.start().week, 1);
/// ```
///
/// The game does not see the hardware every frame:
/// `GF_RTC_UpdateOnFrame` (`src/gf_rtc.c`) re-reads it when its sleep
/// counter passes 10, i.e. on main-loop iterations 10, 21, 32, …
/// after the boot read in `GF_InitRTCWork` (`src/main.c:55`, before
/// the loop). [`Self::observed_at_frame`] models that cadence with
/// the engine's frame index standing in for the main-loop iteration,
/// the same identification `RtcDateTime::rng_seed`'s vblank counter
/// already makes; the asynchronous read itself (`RTC_GetDateTimeAsync`
/// over PXI to the ARM7's SPI) completes well within the frame, so the
/// polled value is the hardware's at that frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcClock {
    start: RtcDateTime,
}

impl RtcClock {
    /// System-clock (ARM7 bus) cycles per RTC second — melonDS's `33513982`
    /// (`RTC::ScheduleTimer`), 32768 ticks of `33513982 / 32768`.
    pub const CYCLES_PER_SECOND: u64 = 33_513_982;

    /// System-clock cycles per frame — 263 lines × 355 × 6
    /// (`GPU.cpp` `FRAME_CYCLES`, `NDS::RunFrame` `frametarget`).
    pub const CYCLES_PER_FRAME: u64 = 560_190;

    /// The first main-loop iteration whose `GF_RTC_UpdateOnFrame`
    /// re-reads the hardware (`++sleep > 10` on the eleventh call).
    pub const FIRST_POLL_FRAME: u32 = 10;

    /// Frames between hardware re-reads after the first.
    pub const POLL_PERIOD: u32 = 11;

    /// A clock pinned at `start` — sanitized the way the oracle's
    /// `RTC::SetDateTime` stores it ([`RtcDateTime::sanitized`]).
    #[must_use]
    pub fn new(start: RtcDateTime) -> Self {
        Self {
            start: start.sanitized(),
        }
    }

    /// The (sanitized) pinned start — the value at frame 0.
    #[must_use]
    pub fn start(&self) -> RtcDateTime {
        self.start
    }

    /// Whole seconds the hardware has counted by the start of frame
    /// `frame`: `floor(frame * 560190 / 33513982)`.
    #[must_use]
    pub const fn elapsed_seconds(frame: u32) -> u64 {
        frame as u64 * Self::CYCLES_PER_FRAME / Self::CYCLES_PER_SECOND
    }

    /// The first frame whose start sees `seconds` counted:
    /// `ceil(seconds * 33513982 / 560190)`. Inverse of
    /// [`Self::elapsed_seconds`] in the sense that
    /// `elapsed_seconds(first_frame_of_second(s)) == s` and the
    /// frame before sees `s - 1`.
    #[must_use]
    pub const fn first_frame_of_second(seconds: u64) -> u64 {
        (seconds * Self::CYCLES_PER_SECOND).div_ceil(Self::CYCLES_PER_FRAME)
    }

    /// The hardware clock at the start of frame `frame`.
    #[must_use]
    pub fn at_frame(&self, frame: u32) -> RtcDateTime {
        self.start
            .advanced_by(Self::elapsed_seconds(frame))
            .expect("the sanitized start is a valid date")
    }

    /// The frame whose hardware read the game's `sRTCWork` cache
    /// holds during `frame`: 0 (the boot read) until the first poll,
    /// then the latest of 10, 21, 32, ….
    #[must_use]
    pub const fn poll_frame(frame: u32) -> u32 {
        if frame < Self::FIRST_POLL_FRAME {
            0
        } else {
            frame - (frame - Self::FIRST_POLL_FRAME) % Self::POLL_PERIOD
        }
    }

    /// What `GF_RTC_CopyDateTime` hands the game during `frame` — the
    /// hardware at [`Self::poll_frame`].
    #[must_use]
    pub fn observed_at_frame(&self, frame: u32) -> RtcDateTime {
        self.at_frame(Self::poll_frame(frame))
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

    // ===== Time-of-day buckets (gf_rtc.c) ===============================

    #[test]
    fn buckets_follow_the_hour_table_at_every_boundary() {
        use TimeOfDay::*;
        let expect = |hour: u32| match hour {
            0..=3 => Late,
            4..=9 => Morn,
            10..=16 => Day,
            17..=19 => Eve,
            _ => Nite,
        };
        for hour in 0..24 {
            assert_eq!(TimeOfDay::by_hour(hour), expect(hour), "hour {hour}");
            let clock = RtcDateTime::new(2010, 3, 14, 0, hour, 59, 59);
            assert_eq!(clock.time_of_day(), expect(hour));
        }
        // The edges, spelled out: 03:59 → 04:00, 09:59 → 10:00,
        // 16:59 → 17:00, 19:59 → 20:00, 23:59 → 00:00.
        assert_eq!(TimeOfDay::by_hour(3), Late);
        assert_eq!(TimeOfDay::by_hour(4), Morn);
        assert_eq!(TimeOfDay::by_hour(9), Morn);
        assert_eq!(TimeOfDay::by_hour(10), Day);
        assert_eq!(TimeOfDay::by_hour(16), Day);
        assert_eq!(TimeOfDay::by_hour(17), Eve);
        assert_eq!(TimeOfDay::by_hour(19), Eve);
        assert_eq!(TimeOfDay::by_hour(20), Nite);
        assert_eq!(TimeOfDay::by_hour(23), Nite);
        assert_eq!(TimeOfDay::by_hour(0), Late);
        // The C enum values survive the round trip.
        for index in 0..TimeOfDay::COUNT {
            assert_eq!(TimeOfDay::from_index(index).map(TimeOfDay::index), Some(index));
        }
        assert_eq!(TimeOfDay::from_index(5), None);
    }

    #[test]
    fn night_and_wild_param_collapse_the_buckets() {
        use TimeOfDay::*;
        assert!(Nite.is_night() && Late.is_night());
        assert!(!Morn.is_night() && !Day.is_night() && !Eve.is_night());
        assert_eq!(Morn.wild_param(), WildTimeOfDay::Morn);
        assert_eq!(Day.wild_param(), WildTimeOfDay::Day);
        assert_eq!(Eve.wild_param(), WildTimeOfDay::Day);
        assert_eq!(Nite.wild_param(), WildTimeOfDay::Nite);
        assert_eq!(Late.wild_param(), WildTimeOfDay::Nite);
        let evening = RtcDateTime::new(2010, 3, 14, 0, 18, 30, 0);
        assert!(!evening.is_night());
        assert_eq!(evening.wild_param(), WildTimeOfDay::Day);
        assert!(RtcDateTime::new(2010, 3, 14, 0, 2, 0, 0).is_night());
    }

    #[test]
    fn time_to_sec_is_seconds_since_midnight() {
        assert_eq!(RtcDateTime::new(2010, 3, 14, 0, 0, 0, 0).seconds_of_day(), 0);
        assert_eq!(RtcDateTime::new(2010, 3, 14, 0, 4, 0, 0).seconds_of_day(), 14_400);
        assert_eq!(
            RtcDateTime::new(2010, 3, 14, 0, 23, 59, 59).seconds_of_day(),
            86_399
        );
    }

    // ===== SDK calendar conversions =====================================

    #[test]
    fn day_number_matches_the_sdk_formula() {
        let day = |y, m, d| RtcDateTime::new(y, m, d, 0, 0, 0, 0).day_number();
        assert_eq!(day(2000, 1, 1), Some(0));
        assert_eq!(day(2000, 2, 29), Some(59));
        assert_eq!(day(2000, 3, 1), Some(60));
        assert_eq!(day(2001, 1, 1), Some(366));
        assert_eq!(day(2004, 2, 29), Some(1520));
        // 10 years (3 leap days) + Jan + Feb + 13.
        assert_eq!(day(2010, 3, 14), Some(3725));
        assert_eq!(day(2099, 12, 31), Some(36_524));
        // The SDK's -1 cases.
        assert_eq!(day(1999, 12, 31), None);
        assert_eq!(day(2100, 1, 1), None);
        assert_eq!(day(2010, 0, 1), None);
        assert_eq!(day(2010, 13, 1), None);
        assert_eq!(day(2010, 1, 0), None);
        assert_eq!(day(2010, 1, 32), None);
        assert_eq!(RtcDateTime::new(2010, 1, 1, 7, 0, 0, 0).day_number(), None);
        // The SDK checks no month length: April 31 is "valid" and lands
        // on May 1's number.
        assert_eq!(day(2010, 4, 31), day(2010, 5, 1));
    }

    #[test]
    fn date_time_seconds_hits_prets_max_seconds() {
        // gf_rtc.c's GF_RTC_TimeDelta asserts this exact value for
        // {99, 12, 31, SUNDAY} {23, 59, 59}.
        let last = RtcDateTime::new(2099, 12, 31, 0, 23, 59, 59);
        assert_eq!(last.date_time_seconds(), Some(3_155_759_999));
        let noon = RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0);
        assert_eq!(noon.date_time_seconds(), Some(3725 * 86_400 + 43_200));
        assert_eq!(RtcDateTime::new(2010, 13, 1, 0, 0, 0, 0).date_time_seconds(), None);
    }

    #[test]
    fn day_of_year_counts_the_leap_day_from_march() {
        assert_eq!(RtcDateTime::new(2010, 3, 14, 0, 0, 0, 0).day_of_year(), 73);
        assert_eq!(RtcDateTime::new(2000, 2, 29, 0, 0, 0, 0).day_of_year(), 60);
        assert_eq!(RtcDateTime::new(2000, 3, 1, 0, 0, 0, 0).day_of_year(), 61);
        assert_eq!(RtcDateTime::new(2001, 3, 1, 0, 0, 0, 0).day_of_year(), 60);
        assert_eq!(RtcDateTime::new(2001, 12, 31, 0, 0, 0, 0).day_of_year(), 365);
    }

    #[test]
    fn week_from_date_uses_the_firmware_epoch() {
        // 2000-01-01 was a Saturday (6); 2010-03-14 a Sunday (0);
        // 2010-03-01 a Monday (1); 2010-01-01 a Friday (5).
        let week = |y, m, d| RtcDateTime::new(y, m, d, 0, 0, 0, 0).week_from_date();
        assert_eq!(week(2000, 1, 1), Some(6));
        assert_eq!(week(2010, 3, 14), Some(0));
        assert_eq!(week(2010, 3, 1), Some(1));
        assert_eq!(week(2010, 1, 1), Some(5));
        assert_eq!(week(2099, 12, 31), Some(4));
        assert_eq!(week(1999, 1, 1), None);
    }

    #[test]
    fn sanitized_clamps_like_set_date_time() {
        // Every field out of range: month and day to 1, time to 0, the
        // week recomputed (2010-01-01, Friday).
        let messy = RtcDateTime::new(2010, 13, 40, 9, 25, 61, 61);
        assert_eq!(messy.sanitized(), RtcDateTime::new(2010, 1, 1, 5, 0, 0, 0));
        // A day past the month's length resets to 1 (2010 is common).
        let feb = RtcDateTime::new(2010, 2, 29, 0, 8, 0, 0).sanitized();
        assert_eq!((feb.month, feb.day), (2, 1));
        // ... but Feb 29 in a leap year stands.
        let leap = RtcDateTime::new(2012, 2, 29, 0, 8, 0, 0).sanitized();
        assert_eq!((leap.month, leap.day), (2, 29));
        // The year folds into 2000–2099 (melonDS's `year %= 100`), and
        // a supplied week is ignored in favor of the date's.
        let folded = RtcDateTime::new(1910, 3, 14, 3, 12, 0, 0).sanitized();
        assert_eq!(folded, RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0));
        // A clean clock is a fixed point.
        let clean = RtcDateTime::new(2010, 3, 1, 1, 9, 0, 0);
        assert_eq!(clean.sanitized(), clean);
    }

    // ===== Advancing =====================================================

    #[test]
    fn advancing_counts_like_the_hardware_chain() {
        let noon = RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0);
        assert_eq!(noon.advanced_by(0), Some(noon));
        assert_eq!(noon.advanced_by(1), Some(RtcDateTime::new(2010, 3, 14, 0, 12, 0, 1)));
        assert_eq!(noon.advanced_by(3600), Some(RtcDateTime::new(2010, 3, 14, 0, 13, 0, 0)));
        assert_eq!(
            noon.advanced_by(86_400),
            Some(RtcDateTime::new(2010, 3, 15, 1, 12, 0, 0))
        );
        // Month, year and leap-day carries.
        let step = |y, m, d, w| RtcDateTime::new(y, m, d, w, 23, 59, 59).advanced_by(1);
        assert_eq!(step(2010, 3, 31, 3), Some(RtcDateTime::new(2010, 4, 1, 4, 0, 0, 0)));
        assert_eq!(step(2010, 12, 31, 5), Some(RtcDateTime::new(2011, 1, 1, 6, 0, 0, 0)));
        assert_eq!(step(2012, 2, 28, 2), Some(RtcDateTime::new(2012, 2, 29, 3, 0, 0, 0)));
        assert_eq!(step(2011, 2, 28, 1), Some(RtcDateTime::new(2011, 3, 1, 2, 0, 0, 0)));
        // The two-digit year wraps 99 → 00; the week is a plain mod-7
        // counter, so it does not snap to 2000-01-01's Saturday.
        assert_eq!(step(2099, 12, 31, 4), Some(RtcDateTime::new(2000, 1, 1, 5, 0, 0, 0)));
        // An invalid start cannot advance.
        assert_eq!(RtcDateTime::new(2010, 2, 30, 0, 0, 0, 0).advanced_by(1).map(|c| c.month), Some(3));
        assert_eq!(RtcDateTime::new(2010, 13, 1, 0, 0, 0, 0).advanced_by(1), None);
    }

    #[test]
    fn advancing_agrees_with_the_second_count_over_years() {
        let start = RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0);
        let base = start.date_time_seconds().expect("valid");
        // Prime-stride sweep across ~5 years so every month length and
        // both leap days (2012, 2016) get crossed.
        for k in 0..20_000u64 {
            let t = k * 7919;
            let later = start.advanced_by(t).expect("valid");
            assert_eq!(later.date_time_seconds(), Some(base + t as i64), "t = {t}");
            assert_eq!(later.week, ((0 + (43_200 + t) / 86_400) % 7) as u32, "t = {t}");
            assert_eq!(later.sanitized(), later, "t = {t}");
        }
    }

    // ===== Frames to seconds =============================================

    #[test]
    fn frame_to_second_boundaries_follow_the_cycle_counts() {
        assert_eq!(RtcClock::elapsed_seconds(0), 0);
        assert_eq!(RtcClock::elapsed_seconds(59), 0);
        assert_eq!(RtcClock::elapsed_seconds(60), 1);
        assert_eq!(RtcClock::elapsed_seconds(119), 1);
        assert_eq!(RtcClock::elapsed_seconds(120), 2);
        assert_eq!(RtcClock::elapsed_seconds(179), 2);
        assert_eq!(RtcClock::elapsed_seconds(180), 3);
        assert_eq!(RtcClock::first_frame_of_second(0), 0);
        assert_eq!(RtcClock::first_frame_of_second(1), 60);
        assert_eq!(RtcClock::first_frame_of_second(2), 120);
        assert_eq!(RtcClock::first_frame_of_second(3), 180);
        // One minute is 3590 frames, not 3600: 59.826 Hz, not 60.
        assert_eq!(RtcClock::first_frame_of_second(60), 3590);
        // The two are inverses at every boundary in a day and a half.
        for s in 0..130_000u64 {
            let f = RtcClock::first_frame_of_second(s);
            assert_eq!(RtcClock::elapsed_seconds(f as u32), s, "second {s}");
            if f > 0 {
                assert_eq!(RtcClock::elapsed_seconds(f as u32 - 1), s - 1, "second {s}");
            }
        }
    }

    #[test]
    fn clock_at_frame_advances_from_the_sanitized_start() {
        let clock = RtcClock::new(RtcDateTime::new(2010, 3, 14, 9, 23, 59, 59));
        // The supplied week (9) is replaced by the date's Sunday.
        assert_eq!(clock.start(), RtcDateTime::new(2010, 3, 14, 0, 23, 59, 59));
        assert_eq!(clock.at_frame(59), clock.start());
        assert_eq!(clock.at_frame(60), RtcDateTime::new(2010, 3, 15, 1, 0, 0, 0));
        // From 23:59:59, 04:00:00 is four hours and one second away.
        let morn = RtcClock::first_frame_of_second(4 * 3600 + 1) as u32;
        assert_eq!(clock.at_frame(morn - 1).time_of_day(), TimeOfDay::Late);
        assert_eq!(clock.at_frame(morn).time_of_day(), TimeOfDay::Morn);
    }

    #[test]
    fn game_polls_the_hardware_every_eleventh_frame() {
        for frame in 0..10 {
            assert_eq!(RtcClock::poll_frame(frame), 0, "frame {frame}");
        }
        assert_eq!(RtcClock::poll_frame(10), 10);
        assert_eq!(RtcClock::poll_frame(20), 10);
        assert_eq!(RtcClock::poll_frame(21), 21);
        assert_eq!(RtcClock::poll_frame(31), 21);
        assert_eq!(RtcClock::poll_frame(32), 32);
        let clock = RtcClock::new(RtcDateTime::new(2010, 3, 1, 0, 9, 0, 0));
        // The hardware ticks at frame 60; the cache last read at 54 and
        // catches up at the next poll, 65.
        assert_eq!(clock.at_frame(60).second, 1);
        assert_eq!(clock.observed_at_frame(60).second, 0);
        assert_eq!(clock.observed_at_frame(64).second, 0);
        assert_eq!(clock.observed_at_frame(65).second, 1);
    }
}
