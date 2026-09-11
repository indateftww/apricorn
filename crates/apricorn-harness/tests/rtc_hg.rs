//! Phase 5 day/night: the engine's clock helpers (`apricorn_core::rtc`)
//! locked to the original functions through arm-runner.
//!
//! `gf_rtc.c`'s leaves are tiny pure functions over an hour or over
//! the `sRTCWork` cache, and the SDK's calendar converters are pure
//! over an `RTCDate`/`RTCTime` pair — exactly the shape arm-runner
//! proves. Every hour goes through `GF_RTC_GetTimeOfDayByHour` and
//! `GF_RTC_GetTimeOfDayWildParamByHour`; the cache readers
//! (`GF_RTC_GetTimeOfDay`, `IsNighttime`, `GF_RTC_TimeToSec`, the wild
//! param) run over an `sRTCWork` this test fills in — including the
//! photo-mode frozen time — and the SDK's `RTC_ConvertDateToDay` /
//! `RTCi_ConvertTimeToSecond` / `RTC_ConvertDateTimeToSecond` get the
//! valid, leap and invalid dates the Rust port's unit tests pin.
//!
//! Runs only when `hg_usa.nds` sits at the repo root (same policy as
//! `rng_hg.rs`; CI has no ROM and skips silently). Every load
//! re-verifies the pin table, so a wrong dump fails before any call.

use apricorn_core::rtc::{RtcDateTime, TimeOfDay};
use apricorn_harness::arm::call::CallResult;
use apricorn_harness::arm::retail::RetailArm9;
use apricorn_harness::pins::{PinMode, PinTable};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Scratch above the image and below the stack: an `RTCDate` (16
/// bytes) then an `RTCTime` (12 bytes) for the SDK converters.
const DATE_ARG: u32 = 0x0220_0000;
const TIME_ARG: u32 = 0x0220_0010;

/// pret's `sRTCWork` (`.bss`, pinned): `time` at +0x20,
/// `frozenTimeState` at +0x48, `frozenTime` at +0x4C — the offsets
/// `GF_RTC_TimeToSec` decodes to (`0x020147A4`), and
/// `getDateTimeSuccess` at +0 which `GF_RTC_CopyTime` asserts.
const WORK: u32 = 0x021D_1048;
const WORK_SUCCESS: u32 = WORK;
const WORK_TIME: u32 = WORK + 0x20;
const WORK_FROZEN_STATE: u32 = WORK + 0x48;
const WORK_FROZEN_TIME: u32 = WORK + 0x4C;

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

/// Calls the pinned function `name` with `args`.
fn call(arm9: &mut RetailArm9, name: &str, args: &[u32]) -> CallResult {
    let table = PinTable::arm9();
    let pin = table.get(name).unwrap_or_else(|| panic!("pin {name}"));
    assert_ne!(pin.mode, PinMode::Data, "{name} is a data pin");
    let entry = pin.address | u32::from(pin.mode == PinMode::Thumb);
    let cpu = arm9.cpu();
    cpu.prepare_call(entry, args);
    cpu.run_default()
        .unwrap_or_else(|e| panic!("{name} faulted: {e}"))
}

fn write_words(arm9: &mut RetailArm9, at: u32, words: &[u32]) {
    let mem = arm9.cpu().mem_mut();
    for (i, &w) in words.iter().enumerate() {
        mem.write32(at + 4 * i as u32, w).expect("scratch write");
    }
}

/// An `RTCDate` as the SDK stores it: year since 2000, month, day, week.
fn write_date(arm9: &mut RetailArm9, at: u32, date: &RtcDateTime) {
    let year = date.year.wrapping_sub(2000);
    write_words(arm9, at, &[year, date.month, date.day, date.week]);
}

/// An `RTCTime`: hour, minute, second.
fn write_time(arm9: &mut RetailArm9, at: u32, time: &RtcDateTime) {
    write_words(arm9, at, &[time.hour, time.minute, time.second]);
}

fn i64_of(result: CallResult) -> i64 {
    ((u64::from(result.r1) << 32) | u64::from(result.r0)) as i64
}

/// The dates the SDK differential sweeps: the epoch, leap days, the
/// desktop's and the oracle's pinned days, the last day of the range,
/// then the invalid shapes the SDK rejects.
fn dates() -> Vec<RtcDateTime> {
    vec![
        RtcDateTime::new(2000, 1, 1, 6, 0, 0, 0),
        RtcDateTime::new(2000, 2, 29, 2, 23, 59, 59),
        RtcDateTime::new(2000, 3, 1, 3, 12, 0, 0),
        RtcDateTime::new(2004, 2, 29, 0, 1, 2, 3),
        RtcDateTime::new(2010, 3, 1, 1, 9, 0, 0),
        RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0),
        RtcDateTime::new(2010, 4, 31, 6, 0, 0, 0), // the SDK checks no month length
        RtcDateTime::new(2099, 12, 31, 4, 23, 59, 59),
        RtcDateTime::new(2100, 1, 1, 5, 0, 0, 0),
        RtcDateTime::new(1999, 12, 31, 5, 0, 0, 0),
        RtcDateTime::new(2010, 0, 1, 0, 0, 0, 0),
        RtcDateTime::new(2010, 13, 1, 0, 0, 0, 0),
        RtcDateTime::new(2010, 1, 0, 0, 0, 0, 0),
        RtcDateTime::new(2010, 1, 32, 0, 0, 0, 0),
        RtcDateTime::new(2010, 1, 1, 7, 0, 0, 0),
    ]
}

// ===== The hour buckets ================================================

#[test]
fn time_of_day_by_hour_matches_all_24_hours() {
    let Some(mut arm9) = load() else { return };
    for hour in 0..24 {
        let bucket = TimeOfDay::by_hour(hour);
        assert_eq!(
            call(&mut arm9, "GF_RTC_GetTimeOfDayByHour", &[hour]).r0,
            bucket.index() as u32,
            "hour {hour}"
        );
        assert_eq!(
            call(&mut arm9, "GF_RTC_GetTimeOfDayWildParamByHour", &[hour]).r0,
            bucket.wild_param() as u32,
            "hour {hour} wild param"
        );
    }
}

#[test]
fn cache_readers_match_over_a_filled_rtc_work() {
    let Some(mut arm9) = load() else { return };
    // GF_RTC_CopyTime asserts getDateTimeSuccess; the boot read sets it.
    write_words(&mut arm9, WORK_SUCCESS, &[1]);
    for hour in 0..24 {
        let clock = RtcDateTime::new(2010, 3, 14, 0, hour, 30, 7);
        write_time(&mut arm9, WORK_TIME, &clock);
        assert_eq!(
            call(&mut arm9, "GF_RTC_GetTimeOfDay", &[]).r0,
            clock.time_of_day().index() as u32,
            "hour {hour}"
        );
        assert_eq!(
            call(&mut arm9, "IsNighttime", &[]).r0,
            u32::from(clock.is_night()),
            "hour {hour} night"
        );
        assert_eq!(
            call(&mut arm9, "GF_RTC_GetTimeOfDayWildParam", &[]).r0,
            clock.wild_param() as u32,
            "hour {hour} wild"
        );
        assert_eq!(
            call(&mut arm9, "GF_RTC_TimeToSec", &[]).r0,
            clock.seconds_of_day(),
            "hour {hour} seconds"
        );
    }
    // Photo mode freezes the time the readers see (state 3 → frozenTime,
    // whose hour and minute GF_RTC_SetAndFreezeTime sets; the second
    // stays whatever the struct held — zero).
    let frozen = RtcDateTime::new(2010, 3, 14, 0, 5, 45, 0);
    write_time(&mut arm9, WORK_TIME, &RtcDateTime::new(2010, 3, 14, 0, 23, 0, 0));
    write_words(&mut arm9, WORK_FROZEN_STATE, &[3]);
    write_words(&mut arm9, WORK_FROZEN_TIME, &[frozen.hour, frozen.minute]);
    assert_eq!(call(&mut arm9, "GF_RTC_TimeToSec", &[]).r0, frozen.seconds_of_day());
    assert_eq!(
        call(&mut arm9, "GF_RTC_GetTimeOfDay", &[]).r0,
        TimeOfDay::Morn.index() as u32
    );
    assert_eq!(call(&mut arm9, "IsNighttime", &[]).r0, 0);
    // Unfreeze: the real (23:00) time returns.
    write_words(&mut arm9, WORK_FROZEN_STATE, &[0]);
    assert_eq!(call(&mut arm9, "IsNighttime", &[]).r0, 1);
    assert_eq!(call(&mut arm9, "GF_RTC_TimeToSec", &[]).r0, 23 * 3600);
}

// ===== The SDK calendar ================================================

#[test]
fn date_to_day_matches_the_sdk() {
    let Some(mut arm9) = load() else { return };
    for date in dates() {
        write_date(&mut arm9, DATE_ARG, &date);
        let got = call(&mut arm9, "RTC_ConvertDateToDay", &[DATE_ARG]).r0;
        let want = date.day_number().map_or(u32::MAX, |d| d);
        assert_eq!(got, want, "{date:?}");
    }
}

#[test]
fn time_to_second_matches_the_sdk() {
    let Some(mut arm9) = load() else { return };
    for (hour, minute, second) in [(0, 0, 0), (0, 0, 1), (4, 0, 0), (9, 59, 59), (23, 59, 59)] {
        let time = RtcDateTime::new(2010, 3, 14, 0, hour, minute, second);
        write_time(&mut arm9, TIME_ARG, &time);
        assert_eq!(
            call(&mut arm9, "RTCi_ConvertTimeToSecond", &[TIME_ARG]).r0,
            time.seconds_of_day(),
            "{hour}:{minute}:{second}"
        );
    }
}

#[test]
fn date_time_to_second_matches_the_sdk() {
    let Some(mut arm9) = load() else { return };
    for date in dates() {
        write_date(&mut arm9, DATE_ARG, &date);
        write_time(&mut arm9, TIME_ARG, &date);
        let got = i64_of(call(&mut arm9, "RTC_ConvertDateTimeToSecond", &[DATE_ARG, TIME_ARG]));
        assert_eq!(got, date.date_time_seconds().unwrap_or(-1), "{date:?}");
    }
    // pret's MAX_SECONDS, asserted by GF_RTC_TimeDelta.
    let last = RtcDateTime::new(2099, 12, 31, 0, 23, 59, 59);
    write_date(&mut arm9, DATE_ARG, &last);
    write_time(&mut arm9, TIME_ARG, &last);
    assert_eq!(
        i64_of(call(&mut arm9, "RTC_ConvertDateTimeToSecond", &[DATE_ARG, TIME_ARG])),
        3_155_759_999
    );
}
