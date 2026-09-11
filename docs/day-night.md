# Day/night — the clock, the hour buckets, field lighting and prop state

Reference for Phase 5's time-of-day layer: `apricorn-core::rtc`
(`TimeOfDay`, the SDK calendar helpers on `RtcDateTime`, and `RtcClock`,
the advancing hardware clock), `field::lighting` (the area-light
archives and the per-map `AreaLightManager`) and `field::time_state`
(the map-prop animation-slot swap). Nothing here is wired into the field
renderer yet: the module hands the presenter typed light templates and a
"bucket changed" edge; `gfx` decides what to do with them later.

Sources: pret `pokeheartgold` — `src/gf_rtc.c` (C) for the buckets and
the polling cadence, `lib/asm/nitro.s` for the SDK's calendar
converters, `asm/overlay_01_021E90C0.s` for the light manager (no C
exists; the asm is the spec), `src/field/overlay_01_02204004.c` for the
prop swap — and melonDS `src/RTC.cpp` / `src/NDS.cpp` for how the
oracle's clock advances. Every leaf that is a pure function is locked to
the retail ARM9 image with arm-runner
(`crates/apricorn-harness/tests/rtc_hg.rs`); the archives are locked to
SHA-1 goldens over the retail dump
(`crates/apricorn-core/tests/lighting_hg.rs`).

## The hour buckets (`gf_rtc.c`)

`GF_RTC_GetTimeOfDayByHour(hour)` indexes `sTimeOfDayByHour[24]`:

| hours | `TIMEOFDAY`          | value | `IsNighttime` | wild param                | prop slot |
|-------|----------------------|-------|---------------|---------------------------|-----------|
| 00–03 | `RTC_TIMEOFDAY_LATE` | 4     | yes           | `TIMEOFDAY_WILD_NITE` (2) | 3         |
| 04–09 | `RTC_TIMEOFDAY_MORN` | 0     | no            | `TIMEOFDAY_WILD_MORN` (0) | 0         |
| 10–16 | `RTC_TIMEOFDAY_DAY`  | 1     | no            | `TIMEOFDAY_WILD_DAY`  (1) | 1         |
| 17–19 | `RTC_TIMEOFDAY_EVE`  | 2     | no            | `TIMEOFDAY_WILD_DAY`  (1) | 2         |
| 20–23 | `RTC_TIMEOFDAY_NITE` | 3     | yes           | `TIMEOFDAY_WILD_NITE` (2) | 3         |

`GF_RTC_GetTimeOfDay()` runs it over the cached hour; `IsNighttime()` is
`NITE || LATE`; `GF_RTC_GetTimeOfDayWildParamByHour` is the three-way
collapse the encounter tables index (`src/field/encounter_check.c:938`).
The port is `TimeOfDay::by_hour`, `is_night`, `wild_param`; the C enum
values are the discriminants (`index`/`from_index`) because
`sTimeOfDayVisualState` and the encounter tables index by them.

The table and the six functions around it were located in the retail
image by a byte scan for the 24-entry table plus the one literal pool
that names it, then decoded outward along the `bl` chain
(`pins/arm9.tsv`, "Day/night" block: `GF_RTC_GetTimeOfDayByHour`
`0x02014844`, `..WildParamByHour` `0x0201485C`, `GF_RTC_GetTimeOfDay`
`0x0201481C`, `IsNighttime` `0x02014804`, `GF_RTC_GetTimeOfDayWildParam`
`0x02014830`, `GF_RTC_TimeToSec` `0x020147A4`, `sTimeOfDayByHour`
`0x020F6060`, `sRTCWork` `0x021D1048` in `.bss`). `rtc_hg.rs` calls all
24 hours through both hour functions, fills `sRTCWork` (`time` at +0x20,
`frozenTimeState` +0x48, `frozenTime` +0x4C — the offsets
`GF_RTC_TimeToSec` decodes to) and runs the cache readers over every
hour and over photo mode's frozen time (state 3 makes the readers use
`frozenTime`, whose second is never set).

Other consumers, for the record: evolution (`src/pokemon.c:2830`), the
field BGM's night variant (`src/field_bgm.c:121`), the map-preview
picture at map entry (`src/unk_02055BF0.c:203`), the Pokégear map, and
weather 0's time-of-day blend (below).

## The SDK calendar (`lib/asm/nitro.s`)

The daily-event bookkeeping compares day numbers, so `RtcDateTime`
gained the SDK converters, each differential-tested against the pinned
ARM function:

* `day_number` = `RTC_ConvertDateToDay` (`nitro.s:9145`, `0x020DC284`):
  days since 2000-01-01 —
  `(day-1) + daysBeforeMonth[month-1] + (month>=3 && year%4==0) + year*365 + (year+3)/4`
  with `year` the SDK's years-since-2000. `None` where the SDK returns
  `-1`: year outside 0–99, month outside 1–12, day outside 1–31 (no
  month-length check — April 31 is "valid"), week ≥ 7. `year % 4` is
  the whole leap rule because 2000 is a leap year and 2100 is out of
  range; pret's `GF_RTC_TimeDelta` asserts the corollary,
  `MAX_SECONDS = 3155759999` for 2099-12-31 23:59:59.
* `seconds_of_day` = `GF_RTC_TimeToSec` / `RTCi_ConvertTimeToSecond`
  (`nitro.s:9190`, `0x020DC318`): `3600h + 60m + s`.
* `date_time_seconds` = `RTC_ConvertDateTimeToSecond` (`nitro.s:9200`,
  `0x020DC330`): the 64-bit `day_number * 86400 + seconds_of_day`, `-1`
  → `None`.
* `day_of_year` = `GF_RTC_GetDayOfYear` (`gf_rtc.c:107`), ported for
  completeness — the retail image never calls it (its `sGF_DaysPerMonth`
  table at `0x020F6048` has no code reference).
* `week_from_date` = the firmware's `(6 + day_number) % 7` (2000-01-01
  was a Saturday, 0 = Sunday) — how melonDS's `RTC::SetDateTime`
  derives the day-of-week register, ignoring any supplied one.

A date-changed check is `a.day_number() != b.day_number()`; a day count
is their difference.

## The advancing clock (`RtcClock`)

The game never sees the wall clock. On hardware the RTC chip ticks; in
the oracle melonDS emulates it, and the runner pins the start
(`docs/oracle.md`, boot step 3: `NDS::Reset()` then
`RTC::SetDateTime(...)`). `RtcDateTime` stays the pinned snapshot the
runner already hands `Game` (unchanged API); `RtcClock { start }` is the
hardware as the oracle runs it, a pure function of the frame index.

### Seconds per frame

melonDS `RTC::ScheduleTimer` (`src/RTC.cpp:513`) runs the 32768 Hz RTC
crystal off the 33 513 982 Hz system clock (the ARM7 bus clock; the
ARM9 runs at twice it, `ARM9ClockShift`) with a Bresenham remainder:

```text
sysclock   = 33513982 + TimerError
delay      = sysclock >> 15
TimerError = sysclock & 0x7FFF
```

so tick `k` lands at system-clock cycle `floor(k · 33513982 / 32768)`,
and after exactly 32768 ticks the remainder returns to zero: the tick
that fires `CountSecond` (`ClockTimer`, `ClockCount & 0x7FFF == 0`)
lands on cycle `s · 33513982` for second `s`, with no drift.
`NDS::Reset` calls `RTC::Reset` (`NDS.cpp:544`), which zeroes
`ClockCount` and schedules the first tick with `TimerError = 0`; the
runner's `SetDateTime` comes *after* the reset and only writes the
date/time registers, so the tick phase is that of cycle 0.

A frame is `NDS::RunFrame`'s `frametarget = SysTimestamp + 560190`
(`NDS.cpp:935`; `GPU.cpp:33` `LINE_CYCLES = 355·6`, `FRAME_CYCLES =
LINE_CYCLES · 263`). Hence the whole seconds visible at the start of
frame `f`:

```text
elapsed_seconds(f) = floor(f · 560190 / 33513982)
```

`RtcClock::CYCLES_PER_SECOND = 33_513_982`, `CYCLES_PER_FRAME =
560_190`, `elapsed_seconds`, and its inverse `first_frame_of_second(s)
= ceil(s · 33513982 / 560190)`. The clock gains its first second at
frame 60 (59 frames are 33 051 210 cycles, short of one second), the
sixtieth at frame 3590 — 59.826 Hz, not 60, which is why the runner's
`frame-rate 59.8268` and this table agree. `at_frame(f)` is
`start.advanced_by(elapsed_seconds(f))`.

Caveat: `RunFrame` runs the CPU until `SysTimestamp >= frametarget`, so
a frame boundary can overshoot its target by an instruction's worth of
cycles and the next target is measured from the overshoot. The frame
count is exact modulo that CPU-granularity jitter; a second boundary
that lands within a few cycles of a frame boundary may be observed one
frame later in the oracle than in the model.

### Counting

`advanced_by(seconds)` counts the way the chip does — melonDS's
`CountSecond → CountMinute → CountHour → CountDay → CountMonth →
CountYear` (`RTC.cpp:502`): 24-hour mode, month lengths by the `year %
4` rule (`RTC::DaysInMonth`), the day-of-week a plain mod-7 counter,
the two-digit year wrapping 99 → 00 (2099-12-31 rolls into
2000-01-01 — a 36 525-day cycle). `RtcClock::new` first runs
`RtcDateTime::sanitized`, which is `RTC::SetDateTime`'s clamping
(`RTC.cpp:158`): year folded into 2000–2099, an out-of-range month or
day (against the month's length) reset to 1, an out-of-range time field
to 0, and the week recomputed from the date. Unit tests sweep ~5 years
at a prime stride and check `date_time_seconds` advances by exactly the
seconds added.

### What the game sees: the polling cadence

`GF_RTC_UpdateOnFrame` (`gf_rtc.c:45`) re-reads the hardware only when
`++getDateTimeSleep > 10` — every eleventh main-loop iteration — and
`GF_InitRTCWork` does a boot read before the loop (`src/main.c`). The
read is asynchronous (`RTC_GetDateTimeAsync` over PXI to the ARM7's
SPI) but completes well within a frame, so the cache holds the hardware
value of the frame it was requested on. With the engine's frame index
standing in for the main-loop iteration (the same identification
`rng_seed`'s vblank counter makes), `RtcClock::poll_frame(f)` is 0 for
`f < 10`, then the latest of 10, 21, 32, …, and
`observed_at_frame(f) = at_frame(poll_frame(f))` is what
`GF_RTC_CopyDateTime` hands every consumer during frame `f`. A bucket
edge is therefore seen up to ten frames after the hardware crosses it,
always on a poll frame — `time_state`'s day-long tick test asserts
exactly that.

## Field lighting (`field::lighting`)

HGSS's overworld does not tint 2D palettes by time of day. The map is
3D, and what changes with the clock is the set of four hardware lights
and the global material colors the map is drawn with. The only 2D
palette work in `fieldmap.c` is the black fade at map transitions
(`:646`); no field code path from `GF_RTC_GetTimeOfDay`/`IsNighttime`
reaches a palette routine. The one other visual consumer is weather 0's
per-bucket blend (`asm/overlay_01_021FD41C.s:126 ov01_021FD4F4`,
`ov01_02208C5C` slot 0), which steps four values toward a per-bucket
target and writes them through `NNS_G3dMdlSetMdlAlphaAll` — a 3D
overlay model, deferred to the weather workstream.

### The manager

`fieldmap.c:758`: at map creation `AreaDataManager_GetAreaLightArchiveID`
(`asm/overlay_01_021FB878.s:249`) maps the area-data record's
`lightSelector` byte (`field::area::AreaData::light_selector`,
`docs/field-data.md`) to an archive id — type 0 → archive 1, type 1 →
archive 0, type 2 → archive 3, anything else → 0 — and `CheckFlag96A`
promotes 3 to 4; `AreaLightManager_New(modelAttributes, id)` follows.
`fieldmap.c:421` runs `AreaLightManager_UpdateActiveTemplate` every
field frame, immediately before the prop swap. The port is
`archive_for_light_type`, `AreaLightManager::new`/`update`.

Archive ids index `ov01_02206450` (`overlay_01_021E90C0.s:2875`):
`data/area00light.txt`, `data/area01light.txt`, `data/area02light.txt`,
`data/dun20_01light.txt`, `data/dun20_02light.txt` — loose NitroFS
files read with `Sys_AllocAndReadFile` (`:2453`), not NARC members.
`data/arealight.narc` is a leftover: nothing in the image names it
(`lighting_hg.rs` shows its four members are the same text format, two
byte-identical to the shipped area01/area02 tables).

### The text format

The loader `ov01_021EA3E0` (`:2384`) walks the file with
`Ascii_GetDelim(…, '\r')` and `Ascii_StrToL` (`src/ascii_util.c`):

```text
until,                       half-seconds of the day this record lasts until
enable,r,g,b,x,y,z,          light 0   (enable == 1 turns it on)
enable,r,g,b,x,y,z,          light 1
enable,r,g,b,x,y,z,          light 2
enable,r,g,b,x,y,z,          light 3
r,g,b,                       diffuse
r,g,b,                       ambient
r,g,b,                       specular
r,g,b,                       emission
                             separator
… (repeat) …
EOF
```

A first pass counts records until a line whose first three bytes are
`E`,`O`,`F` (`:2465–2473`, checked before each record and again on the
tenth line); the second pass fills one 0x30-byte record per ten lines:
`u32 until` at 0, the light-enable mask byte at 4, `u16 color[4]` at 6,
`s16 vector[4][3]` at 0xE, then `u16` diffuse/ambient/specular/emission
at 0x26/0x28/0x2A/0x2C (`ov01_021EA578` per light line, `ov01_021EA668`
per color line). Colors pack `r | g<<5 | b<<10` (`GXRgb`); vector
components are truncated to 16 bits and clamped to ±4096 (fx16 ±1.0).
`Ascii_StrToL`'s quirks are ported verbatim (a leading non-digit is
ignored, one elsewhere yields −1; a line longer than the 256-byte
buffer is an error).

Retail shapes (`lighting_hg.rs`): the three area tables have 15 records
whose thresholds ascend strictly and end at 43200; `area00` opens with
a zero-length `until 0` record, `area01`/`area02` with a short one
(`until 900`, 00:30); the two dungeon tables are one record with `until
0`. Values are pinned by SHA-1 over a canonical serialization; the
tables themselves are ROM data and are not reproduced here.

### Selection: thresholds, not interpolation

Both selection paths compare `GF_RTC_TimeToSec() / 2` (a signed
`asr #1`, `:2229–2233` and `:2306–2309`) against the `until` fields, so
thresholds are **half-seconds** and a day is 43200. Values step; nothing
interpolates between records.

* `AreaLightManager_New` (`:2237–2270`): the active record is the first
  whose `until` exceeds the half-second (unsigned `bls`), or 0 when none
  does. So record `i` covers `[until[i-1], until[i])` and a leading
  `until 0` record is never the boot pick. Then the record is applied
  (`ov01_021EA398` → `ov01_021EA300`). Port: `initial_index`,
  `template_at`.
* `AreaLightManager_UpdateActiveTemplate` (`:2278`): with more than one
  record, if the half-second is `>= until[active]` or `< until[active-1]`
  (0 for the first record; signed compares), the active index advances
  by one, wrapping to 0, and — when the enable flag at +0x10 is set —
  the new record is applied. One step per frame: a jump of several
  records takes as many frames to catch up, and at midnight the clock
  falls below the last record's window so the manager visits record 0
  (the zero-length one in `area00`) for one frame on its way to record
  1. `lighting_hg.rs` steps every second of a day and checks the
  manager's index equals the direct selection throughout, then the
  midnight wrap.

### Applying a template

`ov01_021EA300` (`:2331`) writes through the `ModelAttributes` setters
(`asm/model_attributes.s:268–340`, `include/field/model_attributes.h`):
for each of the four lights, an enabled one's vector and color
(`NNS_G3dGlbLightVector`/`Color`), a disabled one's zeros; then
`diffuse` (with `setDiffuseColorAsVertexColor = FALSE`, no global
apply), `ambient` (applied), `specular` (with
`enableSpecularReflectShininessTable = FALSE`, no apply), `emission`
(applied). `LightTemplate::apply` does the same into `ModelLighting`,
the lighting subset of `ModelAttributes` in its own field order, which
is what the renderer will consume.

## Prop visual state (`field::time_state`)

`src/field/overlay_01_02204004.c:473`:

```c
sTimeOfDayVisualState[RTC_TIMEOFDAY_COUNT] = { MORN 0, DAY 1, EVE 2, NITE 3, LATE 3 };
```

`FieldSystemUnkSub104_Init` (`:441`) stores `GF_RTC_GetTimeOfDay()` at
map load; `ov01_022047DC` (`:481`, `fieldmap.c:422`) compares the
current *bucket* with the stored one every frame and, on any change,
`MapPropAnimation_RemoveFromRenderObj`s the old bucket's slot and
`AddToRenderObj`s the new bucket's for every registered prop (up to
four props of up to four animations, `ov01_0220476C`). Because the
comparison is on the bucket, the midnight `NITE → LATE` edge fires a
remove/add of the same slot (3 → 3).

`TimeOfDayState::new(bucket)`, `update(now) -> Option<BucketChange>`
(the edge machine; `BucketChange::previous_slot`/`slot`/`slot_changed`),
`tick(&clock, frame)` (update with `observed_at_frame(frame)`'s bucket),
`visual_state()` (`ov01_02204834`).

## Tests

* `rtc.rs` unit tests: every bucket boundary, the SDK formulas at the
  epoch, leap days and `MAX_SECONDS`, `sanitized` vs `SetDateTime`,
  `advanced_by` carries and the 99 → 00 wrap, the frame → second
  boundaries (0/59/60/119/120…, 3590 for a minute) and their inverse
  over a day and a half, the poll cadence.
* `field/lighting.rs` unit tests over an invented three-record fixture:
  fields, clamps, the enable mask, selection edges, the one-step-per-
  update manager and its midnight wrap, single-record tables, EOF on a
  tenth line, malformed texts, `Ascii_StrToL`/`Ascii_GetDelim` quirks.
* `tests/lighting_hg.rs` (ROM-gated): the five archives parse with the
  pinned record counts, ordering and range; SHA-1 goldens; 09:00 vs
  21:00 differ in every stepping table and match in the flat ones; the
  manager tracks the direct selection across a day; the leftover NARC;
  the bedroom's area data names a loadable archive.
* `crates/apricorn-harness/tests/rtc_hg.rs` (ROM-gated, arm-runner):
  both hour functions over all 24 hours, the cache readers over a
  filled `sRTCWork` (including frozen time), the three SDK converters
  over valid, leap and invalid dates. Twelve pins were appended to
  `pins/arm9.tsv` (9 code + 3 data), so the pin-count assertions in
  `src/pins.rs` and `tests/pins_hg.rs` read 103 (91 code).
