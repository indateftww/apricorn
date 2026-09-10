//! The engine-side trace producer — the real `apricorn-core` game
//! replayed through the same corpus machinery as the oracle (PLAN.md
//! Phase 2's exit promise, redeemed in Phase 4).
//!
//! [`EngineRun`] opens the ROM, pins the RTC from the script, boots
//! [`Game`] (`NitroMain`'s order: card, clock, seed, intro — pret
//! `src/main.c:50-90`), ticks frames `0..frames` with the script's
//! input, and after every tick hashes each watched region of
//! `regions.conf` — by **name**, through the engine-side region table
//! below — into a [`Trace`] whose gate fields (`rom-sha1`,
//! `input-sha1`, `regions-sha1`, `frames`, `frame-rate`, `rtc`) are
//! computed exactly as the oracle computes them, so `apricorn-diff`
//! (and `apricorn-replay --engine`) compare the two producers with no
//! format change.
//!
//! # The region table
//!
//! The oracle hashes memory at pinned addresses; the engine has no
//! memory image, so a region resolves by its pin *name* to the engine
//! state that models that static, serialized as the ROM lays it out
//! (`src/math_util.c:9-11` declares the three):
//!
//! | region (`pins/arm9.tsv`) | engine source | bytes |
//! |---|---|---|
//! | `sLCRNG_State` | [`Game::lcrng`] → `seed()` | 4 — one LE `u32` |
//! | `sMTRNG_State` | [`Game::mtrng`] → `state_words()` | 2496 — the 624 LE `u32` words, no cursor |
//! | `sMTRNG_Cycles` | [`Game::mtrng`] → `cycles()` | 4 — one LE `i32` (`.data`, initializer 625) |
//!
//! A `regions.conf` naming anything else — or giving a known region a
//! size other than its layout's — is refused before the ROM is opened
//! ([`check_regions`]), with the region named. The table grows as the
//! engine grows (the party, the player position, flags, …).
//!
//! # Input
//!
//! `.apin` state → [`Input`] is bit-for-bit ([`to_input`]): the
//! script's mask uses the `REG_KEYXY` bit layout with **bit set =
//! held**, and so does [`Keys`] — the twelve names map onto
//! `key::A` … `key::Y` in the same order (a test locks both tables
//! together). A `touch x y` becomes `Touch { x, y }`, `lift` becomes
//! `None`. Frames past the script's `end` (a `frames` override) get
//! idle input, as the oracle's input blob does.
//!
//! # Sampling
//!
//! The oracle hashes after `NDS::RunFrame()`; the engine hashes after
//! [`Game::tick`] — every region whose `frame % sample == 0`, regions
//! in file order — so both emit the same records in the same order.
//! Frame indices are each producer's own axis: the engine's frame 0
//! is the intro's first tick, the oracle's frame 0 is power-on (crt0
//! still decompressing the ARM9 image; the retail `InitializeMainRNG`
//! lands at VBlank 185 on `corpus/boot-idle`). `docs/engine-runner.md`
//! carries the measured boot timeline and what it means for parity.

use std::path::Path;
use std::sync::{Arc, Mutex};

use apricorn_core::Frame;
use apricorn_core::app::game::Game;
use apricorn_core::assets::AssetStore;
use apricorn_core::frame::LogicalFrame;
use apricorn_core::input::{Input, Keys, Touch};
use apricorn_core::rng::{Lcrng, Mt19937};
use apricorn_core::rtc::RtcDateTime;

use crate::HarnessError;
use crate::input::{FrameInput, InputScript};
use crate::regions::RegionSet;
use crate::trace::{Trace, TraceHeader, TraceRecord};

/// The `producer` header this side writes.
pub const PRODUCER: &str = "engine-apricorn-core";

/// The `frame-rate` header — the oracle's `FRAME_RATE` string byte
/// for byte (informational only; equivalence is frame-indexed).
pub const FRAME_RATE: &str = "59.8268";

/// The RTC a script without an `rtc` line replays under — the
/// oracle's documented default (`docs/oracle.md`), so both producers'
/// headers agree on a case that pins no clock.
pub const DEFAULT_RTC: &str = "2010-03-01T09:00:00";

/// The regions the engine can serve, each with the byte size its
/// serialization has (the ROM static's `sizeof`).
pub const REGIONS: [(&str, u32); 3] = [
    ("sLCRNG_State", 4),
    ("sMTRNG_State", 2496),
    ("sMTRNG_Cycles", 4),
];

/// `sLCRNG_State` as the ROM holds it: one little-endian `u32`.
#[must_use]
pub fn lcrng_bytes(rng: &Lcrng) -> [u8; 4] {
    rng.seed().to_le_bytes()
}

/// `sMTRNG_State` as the ROM holds it: the 624 words, each a
/// little-endian `u32`, in array order — 2496 bytes, cursor excluded.
#[must_use]
pub fn mtrng_state_bytes(rng: &Mt19937) -> Vec<u8> {
    let mut out = Vec::with_capacity(624 * 4);
    for word in rng.state_words() {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out
}

/// `sMTRNG_Cycles` as the ROM holds it: one little-endian `int`.
#[must_use]
pub fn mtrng_cycles_bytes(rng: &Mt19937) -> [u8; 4] {
    rng.cycles().to_le_bytes()
}

/// The bytes a named region hashes at this tick, or `None` for a
/// name the table does not know (see [`REGIONS`]).
#[must_use]
pub fn region_bytes(game: &Game, name: &str) -> Option<Vec<u8>> {
    match name {
        "sLCRNG_State" => Some(lcrng_bytes(game.lcrng()).to_vec()),
        "sMTRNG_State" => Some(mtrng_state_bytes(game.mtrng())),
        "sMTRNG_Cycles" => Some(mtrng_cycles_bytes(game.mtrng()).to_vec()),
        _ => None,
    }
}

/// Checks that every region of a `regions.conf` has an engine-side
/// source of the configured size — run before the ROM is touched, so
/// a misnamed region fails fast and by name.
///
/// # Errors
/// Returns a [`HarnessError::Gate`] naming the first region the
/// engine cannot serve, or whose size differs from the layout's.
pub fn check_regions(regions: &RegionSet) -> Result<(), HarnessError> {
    for region in regions.regions() {
        let Some(&(_, size)) = REGIONS.iter().find(|(name, _)| *name == region.name) else {
            let known: Vec<&str> = REGIONS.iter().map(|(name, _)| *name).collect();
            return Err(HarnessError::Gate {
                what: format!(
                    "region '{}' has no engine-side source (the engine serves: {})",
                    region.name,
                    known.join(", ")
                ),
            });
        };
        if region.size != size {
            return Err(HarnessError::Gate {
                what: format!(
                    "region {}: regions.conf size {} != the engine layout's {}",
                    region.name, region.size, size
                ),
            });
        }
    }
    Ok(())
}

/// One `.apin` frame state as the engine's [`Input`]: the button
/// mask verbatim (both sides use the `REG_KEYXY` layout, bit set =
/// held), the stylus as a [`Touch`] when down.
#[must_use]
pub fn to_input(input: FrameInput) -> Input {
    Input {
        keys: Keys(input.mask),
        touch: input.touch.map(|(x, y)| Touch { x, y }),
    }
}

/// `RTCWeek` for a Gregorian date — `RTC_WEEK_SUNDAY` = 0 through
/// `RTC_WEEK_SATURDAY` = 6 (`lib/include/nitro/rtc/ARM9/api.h`) —
/// by Sakamoto's method.
#[must_use]
pub fn day_of_week(year: u32, month: u32, day: u32) -> u32 {
    const OFFSETS: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    (y + y / 4 - y / 100 + y / 400 + OFFSETS[(month - 1) as usize] + day) % 7
}

/// Parses a script's `rtc` value — `YYYY-MM-DDTHH:MM:SS`, the form
/// the oracle's `--rtc` takes — into the pinned clock the game boots
/// under. The day of week is derived (the SDK's `RTCDate::week`).
///
/// # Errors
/// Returns a [`HarnessError::Gate`] quoting the text when it is not
/// a well-formed, in-range timestamp.
pub fn parse_rtc(text: &str) -> Result<RtcDateTime, HarnessError> {
    let bad = || HarnessError::Gate {
        what: format!("rtc: expected YYYY-MM-DDTHH:MM:SS, got '{text}'"),
    };
    let (date, time) = text.split_once('T').ok_or_else(bad)?;
    let fields = |part: &str, sep: char| -> Result<[u32; 3], HarnessError> {
        let parsed: Vec<u32> = part
            .split(sep)
            .map(|f| f.parse::<u32>().map_err(|_| bad()))
            .collect::<Result<_, _>>()?;
        <[u32; 3]>::try_from(parsed).map_err(|_| bad())
    };
    let [year, month, day] = fields(date, '-')?;
    let [hour, minute, second] = fields(time, ':')?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
        || year < 1
    {
        return Err(bad());
    }
    Ok(RtcDateTime::new(
        year,
        month,
        day,
        day_of_week(year, month, day),
        hour,
        minute,
        second,
    ))
}

/// One engine replay: the inputs the oracle's [`crate::oracle::OracleRun`]
/// takes, minus probes (the engine has no addresses to call) and plus
/// an optional card backup.
#[derive(Debug, Clone)]
pub struct EngineRun<'a> {
    /// The ROM dump to open (its whole-file SHA-1 is the `rom-sha1`).
    pub rom: &'a Path,
    /// The watched regions — every name must be in [`REGIONS`].
    pub regions: &'a RegionSet,
    /// The input script: the pinned RTC, the length, the events.
    pub script: &'a InputScript,
    /// The card backup handed to [`Game::new`]; `None` is a blank card.
    pub save: Option<&'a [u8]>,
    /// Frames to run; `None` is the script's `end`. Frames past `end`
    /// get idle input.
    pub frames: Option<u32>,
    /// The `producer` header; `None` is [`PRODUCER`].
    pub producer: Option<&'a str>,
}

/// What an observer sees after each tick — the frame the tick
/// produced and the game it left behind (for PNG dumps, state
/// prints, and any per-frame check a caller wants).
pub struct EngineTick<'a> {
    /// The tick's frame index.
    pub index: u32,
    /// The logical frame [`Game::tick`] returned for this index (on
    /// an advancing tick, the finishing state's last frame).
    pub frame: &'a LogicalFrame,
    /// The game after the tick.
    pub game: &'a Game,
    /// The asset store the game runs on (what a rasterizer needs).
    pub store: &'a Mutex<AssetStore>,
}

/// A finished engine replay: the trace, plus the game and its store
/// for whatever the caller wants to read off the end state.
pub struct EngineOutput {
    /// The trace, header gates included.
    pub trace: Trace,
    /// The game after the last tick.
    pub game: Game,
    /// The asset store the game ran on.
    pub store: Arc<Mutex<AssetStore>>,
    /// The pinned clock the game booted under.
    pub rtc: RtcDateTime,
}

impl EngineRun<'_> {
    /// The frames this run replays: the override, else the script's
    /// `end`.
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.frames.unwrap_or(self.script.end)
    }

    /// The `rtc` header text: the script's, else [`DEFAULT_RTC`].
    #[must_use]
    pub fn rtc_text(&self) -> &str {
        self.script.rtc.as_deref().unwrap_or(DEFAULT_RTC)
    }

    /// Replays the run and returns the trace with the end state.
    ///
    /// # Errors
    /// See [`Self::run_with`].
    pub fn run(&self) -> Result<EngineOutput, HarnessError> {
        self.run_with(|_| {})
    }

    /// Replays the run, calling `observe` after every tick, and
    /// returns the trace with the end state.
    ///
    /// The regions are validated and the RTC parsed before the ROM is
    /// read; the ROM is hashed whole (the oracle's `rom-sha1`), then
    /// opened as the game's asset store.
    ///
    /// # Errors
    /// Returns a [`HarnessError::Gate`] for a region the engine cannot
    /// serve ([`check_regions`]), an unparsable `rtc` ([`parse_rtc`]),
    /// an unreadable or unopenable ROM, or a card blob the game has
    /// no flow for ([`apricorn_core::app::game::GameError`]).
    ///
    /// # Panics
    /// The game panics when a pinned ROM member fails to load — the
    /// engine's own convention (a broken member is a broken table).
    pub fn run_with(
        &self,
        mut observe: impl FnMut(&EngineTick<'_>),
    ) -> Result<EngineOutput, HarnessError> {
        let gate = |what: String| HarnessError::Gate { what };
        check_regions(self.regions)?;
        let rtc = parse_rtc(self.rtc_text())?;

        let rom_bytes = std::fs::read(self.rom)
            .map_err(|e| gate(format!("cannot read {}: {e}", self.rom.display())))?;
        let rom_sha1 = Trace::hash_bytes(&rom_bytes);
        drop(rom_bytes);
        let store = AssetStore::open(self.rom)
            .map_err(|e| gate(format!("cannot open {}: {e}", self.rom.display())))?;
        let store = Arc::new(Mutex::new(store));
        let mut game = Game::new(Arc::clone(&store), self.save, rtc)
            .map_err(|e| gate(format!("cannot boot the game: {e}")))?;

        let frames = self.frames();
        let mut records = Vec::new();
        for index in 0..frames {
            // The script's state for its frames, idle past its end —
            // the oracle's input blob does exactly this.
            let input = if index < self.script.end {
                to_input(self.script.at(index))
            } else {
                Input::default()
            };
            let frame = game.tick(Frame { index }, input).clone();
            observe(&EngineTick {
                index,
                frame: &frame,
                game: &game,
                store: &store,
            });
            for region in self.regions.regions() {
                if index % region.sample == 0 {
                    let bytes = region_bytes(&game, &region.name)
                        .expect("check_regions admitted every region name");
                    records.push(TraceRecord::Sample {
                        frame: index,
                        region: region.name.clone(),
                        hash: Trace::hash_region(&bytes),
                    });
                }
            }
        }

        // The passthrough gates hash the canonical texts — the same
        // bytes `InputScript::sha1_hex` / `RegionSet::sha1_hex` hash,
        // which is what the oracle is handed.
        let header = TraceHeader {
            producer: self.producer.unwrap_or(PRODUCER).to_string(),
            rom_sha1,
            input_sha1: Trace::hash_bytes(self.script.to_string().as_bytes()),
            regions_sha1: Trace::hash_bytes(self.regions.canonical().as_bytes()),
            frames,
            frame_rate: FRAME_RATE.to_string(),
            rtc: Some(self.rtc_text().to_string()),
        };
        Ok(EngineOutput {
            trace: Trace { header, records },
            game,
            store,
            rtc,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::button_mask;
    use apricorn_core::input::key;

    #[test]
    fn apin_buttons_are_the_core_key_bits() {
        // The two tables are cited, not shared (the core keeps zero
        // dependencies): lock them together here, name by name.
        let pairs = [
            ("A", key::A),
            ("B", key::B),
            ("SELECT", key::SELECT),
            ("START", key::START),
            ("RIGHT", key::RIGHT),
            ("LEFT", key::LEFT),
            ("UP", key::UP),
            ("DOWN", key::DOWN),
            ("R", key::R),
            ("L", key::L),
            ("X", key::X),
            ("Y", key::Y),
        ];
        for (name, bit) in pairs {
            assert_eq!(button_mask(name), Some(bit), "button {name}");
        }
    }

    #[test]
    fn to_input_is_bit_for_bit() {
        let held = FrameInput {
            mask: button_mask("A").unwrap() | button_mask("UP").unwrap(),
            touch: Some((128, 96)),
        };
        let input = to_input(held);
        assert!(input.keys.down(key::A | key::UP));
        assert!(!input.keys.any(key::B));
        assert_eq!(input.touch, Some(Touch { x: 128, y: 96 }));
        assert_eq!(to_input(FrameInput::IDLE), Input::default());
    }

    #[test]
    fn rtc_parses_the_oracle_form() {
        // The boot-idle pin: a Monday.
        let rtc = parse_rtc("2010-03-01T09:00:00").expect("well-formed");
        assert_eq!(
            (rtc.year, rtc.month, rtc.day, rtc.hour, rtc.minute, rtc.second),
            (2010, 3, 1, 9, 0, 0)
        );
        assert_eq!(rtc.week, 1, "2010-03-01 was a Monday (RTC_WEEK_MONDAY)");
        // The desktop's pin: HG's US release date, a Sunday.
        assert_eq!(day_of_week(2010, 3, 14), 0);
        assert_eq!(day_of_week(2000, 1, 1), 6, "a Saturday");
        assert_eq!(day_of_week(2024, 2, 29), 4, "a Thursday (leap day)");

        for bad in [
            "2010-03-01 09:00:00",
            "2010-3-1T9",
            "2010-13-01T09:00:00",
            "2010-03-32T09:00:00",
            "2010-03-01T24:00:00",
            "2010-03-01T09:60:00",
            "2010-03-01T09:00:60",
            "x",
            "",
        ] {
            assert!(parse_rtc(bad).is_err(), "must refuse '{bad}'");
        }
    }

    #[test]
    fn region_table_serializes_the_rom_layout() {
        let lc = Lcrng::new(0x0309_000A);
        assert_eq!(lcrng_bytes(&lc), [0x0A, 0x00, 0x09, 0x03]);

        let mt = Mt19937::uninitialized();
        let bytes = mtrng_state_bytes(&mt);
        assert_eq!(bytes.len(), 2496);
        assert!(bytes.iter().all(|&b| b == 0));
        assert_eq!(mtrng_cycles_bytes(&mt), 625i32.to_le_bytes());

        let seeded = Mt19937::new(0x1234);
        let bytes = mtrng_state_bytes(&seeded);
        assert_eq!(&bytes[..4], &0x1234u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &seeded.state_words()[1].to_le_bytes());
        assert_eq!(mtrng_cycles_bytes(&seeded), 624i32.to_le_bytes());
    }

    #[test]
    fn check_regions_refuses_unknown_names_and_wrong_sizes() {
        let ok = RegionSet::parse(
            "sLCRNG_State hard 0x021D15A8 4 1\nsMTRNG_State hard 0x021D15AC 2496 30\nsMTRNG_Cycles hard 0x0210F6CC 4 1\n",
        )
        .unwrap();
        check_regions(&ok).expect("the pinned RNG statics are served");

        let unknown = RegionSet::parse("party hard 0x02000000 4 1\n").unwrap();
        let err = check_regions(&unknown).expect_err("unknown region");
        assert!(err.to_string().contains("'party'"), "{err}");
        assert!(err.to_string().contains("sLCRNG_State"), "{err}");

        let wrong = RegionSet::parse("sLCRNG_State hard 0x021D15A8 8 1\n").unwrap();
        let err = check_regions(&wrong).expect_err("wrong size");
        assert!(err.to_string().contains("size 8 != the engine layout's 4"), "{err}");

        // An empty config is fine: a trace with no records.
        check_regions(&RegionSet::parse("").unwrap()).unwrap();
    }

    #[test]
    fn a_bad_region_fails_before_the_rom_is_touched() {
        let regions = RegionSet::parse("bogus hard 0x02000000 4 1\n").unwrap();
        let script = InputScript::parse("end 3\n").unwrap();
        let run = EngineRun {
            rom: Path::new("this-rom-does-not-exist.nds"),
            regions: &regions,
            script: &script,
            save: None,
            frames: None,
            producer: None,
        };
        let Err(err) = run.run() else {
            panic!("must refuse a region the engine cannot serve")
        };
        assert!(err.to_string().contains("'bogus'"), "{err}");
        assert_eq!(run.frames(), 3);
        assert_eq!(run.rtc_text(), DEFAULT_RTC);
    }
}
