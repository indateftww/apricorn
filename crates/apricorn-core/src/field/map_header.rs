//! Map headers — pret `include/map_header.h:13` (`MapHeader`, 24 bytes,
//! GCC little-endian bitfields packed LSB-first) and
//! `src/data/map_headers.h:15` (`sMapHeaders`, 540 records in ARM9
//! `.rodata`), plus the NitroFS name table
//! `fielddata/maptable/mapname.bin` (540 × 16 NUL-padded names).
//!
//! The table's RAM address is not in pret (no symbol files); it was
//! found by encoding map 64's record from pret's values and scanning the
//! decompressed ARM9 image for the 24 bytes — one hit — then decoding
//! neighbours 60/63/65 against pret's table. The harness pins the table
//! as `sMapHeaders` (`crates/apricorn-harness/pins/arm9.tsv`).

use crate::assets::{AssetStore, AssetsError};
use crate::nds::{NdsError, u16le, u32le};

/// Records in `sMapHeaders` (`MAP_NEW_BARK_RIVAL_HOUSE_2F` is 384; the
/// table runs to `NUM_MAPS`).
pub const MAP_HEADER_COUNT: usize = 540;
/// `sizeof(MapHeader)`.
pub const MAP_HEADER_SIZE: usize = 24;
/// RAM address of `sMapHeaders` in the retail (US) ARM9 image — see the
/// module notes for how it was found.
pub const MAP_HEADERS_ADDRESS: u32 = 0x020F_6BE0;
/// NitroFS path of the map name table (`MAP_HEADER_COUNT` ×
/// [`MAP_NAME_SIZE`] NUL-padded ASCII names such as `T20R0202`).
pub const MAP_NAMES_PATH: &str = "fielddata/maptable/mapname.bin";
/// Bytes per name in [`MAP_NAMES_PATH`].
pub const MAP_NAME_SIZE: usize = 16;
/// `ENCDATA_NA` (`include/encounter_tables_narc.h:6`): the wild-encounter
/// bank of a map without encounters (`MapHeader_HasWildEncounters`).
pub const NO_WILD_ENCOUNTERS: u8 = 255;

/// `MapType` (`include/map_header.h:44`), the 4-bit `mapType` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum MapType {
    /// `MAP_TYPE_INVALID`.
    #[default]
    Invalid = 0,
    /// `MAP_TYPE_CITY_TOWN`.
    CityTown = 1,
    /// `MAP_TYPE_ROUTE`.
    Route = 2,
    /// `MAP_TYPE_CAVE`.
    Cave = 3,
    /// `MAP_TYPE_INTERIOR`.
    Interior = 4,
    /// `MAP_TYPE_POKEMON_CENTER` (unused in HG/SS).
    PokemonCenter = 5,
    /// `MAP_TYPE_UNDERGROUND` (Sinnoh leftover).
    Underground = 6,
}

impl MapType {
    fn from_raw(raw: u8) -> Result<Self, NdsError> {
        Ok(match raw {
            0 => Self::Invalid,
            1 => Self::CityTown,
            2 => Self::Route,
            3 => Self::Cave,
            4 => Self::Interior,
            5 => Self::PokemonCenter,
            6 => Self::Underground,
            _ => {
                return Err(NdsError::Invalid {
                    what: "map header mapType",
                });
            }
        })
    }
}

/// `MAP_FOLLOWMODE_*` (`include/map_header.h:9`), the 2-bit `followMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum FollowMode {
    /// `MAP_FOLLOWMODE_PREVENT`.
    #[default]
    Prevent = 0,
    /// `MAP_FOLLOWMODE_HEIGHT_RESTRICT`.
    HeightRestrict = 1,
    /// `MAP_FOLLOWMODE_ALLOW`.
    Allow = 2,
}

impl FollowMode {
    fn from_raw(raw: u8) -> Result<Self, NdsError> {
        Ok(match raw {
            0 => Self::Prevent,
            1 => Self::HeightRestrict,
            2 => Self::Allow,
            _ => {
                return Err(NdsError::Invalid {
                    what: "map header followMode",
                });
            }
        })
    }
}

/// One decoded `MapHeader` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MapHeader {
    /// `wildEncounterBank` — `a/0/3/7` member, [`NO_WILD_ENCOUNTERS`] for none.
    pub wild_encounter_bank: u8,
    /// `areaDataBank` — `a/0/4/2` member (see [`super::area`]).
    pub area_data_bank: u8,
    /// `moveModelBank` (4 bits) — `fielddata/mm_list` member.
    pub move_model_bank: u8,
    /// `worldMapX` (6 bits).
    pub world_map_x: u8,
    /// `worldMapY` (6 bits).
    pub world_map_y: u8,
    /// `matrixId` — `a/0/4/1` member (see [`super::matrix`]).
    pub matrix_id: u16,
    /// `scriptsBank` — `a/0/1/2` member holding the map's scripts.
    pub scripts_bank: u16,
    /// `scriptHeaderBank` — `a/0/1/2` member (see [`super::script_header`]).
    pub script_header_bank: u16,
    /// `msgBank` — `a/0/2/7` message bank.
    pub msg_bank: u16,
    /// `dayMusicId` — SDAT sequence.
    pub day_music_id: u16,
    /// `nightMusicId` — SDAT sequence.
    pub night_music_id: u16,
    /// `eventsBank` — `a/0/3/2` member (see [`super::events`]).
    pub events_bank: u16,
    /// `mapsec` (8 bits) — map section / location name.
    pub mapsec: u8,
    /// `areaIcon` (4 bits).
    pub area_icon: u8,
    /// `momCallIntroParam` (4 bits).
    pub mom_call_intro_param: u8,
    /// `regionNo` (1 bit): 0 Johto, 1 Kanto.
    pub region_no: u8,
    /// `weather` (7 bits).
    pub weather: u8,
    /// `mapType` (4 bits).
    pub map_type: MapType,
    /// `cameraType` (6 bits) — index into the ov01 camera presets.
    pub camera_type: u8,
    /// `followMode` (2 bits).
    pub follow_mode: FollowMode,
    /// `battleBg` (5 bits) — `BattleBg` (`include/constants/battle.h`).
    pub battle_bg: u8,
    /// `bikeAllowed`.
    pub bike_allowed: bool,
    /// `runningAllowed_Unused`.
    pub running_allowed_unused: bool,
    /// `escapeRopeAllowed`.
    pub escape_rope_allowed: bool,
    /// `flyAllowed`.
    pub fly_allowed: bool,
    /// `outgoingCalls`.
    pub outgoing_calls: bool,
    /// `incomingCalls`.
    pub incoming_calls: bool,
    /// `radioSignal`.
    pub radio_signal: bool,
}

impl MapHeader {
    /// Decodes one 24-byte record.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the record is short or a `mapType` /
    /// `followMode` value has no enumerator.
    pub fn parse(record: &[u8]) -> Result<Self, NdsError> {
        if record.len() < MAP_HEADER_SIZE {
            return Err(NdsError::Truncated {
                what: "map header",
                need: MAP_HEADER_SIZE,
                got: record.len(),
            });
        }
        let w2 = u16le(record, 2)?;
        let w18 = u16le(record, 18)?;
        let w20 = u32le(record, 20)?;
        Ok(Self {
            wild_encounter_bank: record[0],
            area_data_bank: record[1],
            move_model_bank: (w2 & 15) as u8,
            world_map_x: ((w2 >> 4) & 63) as u8,
            world_map_y: ((w2 >> 10) & 63) as u8,
            matrix_id: u16le(record, 4)?,
            scripts_bank: u16le(record, 6)?,
            script_header_bank: u16le(record, 8)?,
            msg_bank: u16le(record, 10)?,
            day_music_id: u16le(record, 12)?,
            night_music_id: u16le(record, 14)?,
            events_bank: u16le(record, 16)?,
            mapsec: (w18 & 255) as u8,
            area_icon: ((w18 >> 8) & 15) as u8,
            mom_call_intro_param: ((w18 >> 12) & 15) as u8,
            region_no: (w20 & 1) as u8,
            weather: ((w20 >> 1) & 127) as u8,
            map_type: MapType::from_raw(((w20 >> 8) & 15) as u8)?,
            camera_type: ((w20 >> 12) & 63) as u8,
            follow_mode: FollowMode::from_raw(((w20 >> 18) & 3) as u8)?,
            battle_bg: ((w20 >> 20) & 31) as u8,
            bike_allowed: w20 & (1 << 25) != 0,
            running_allowed_unused: w20 & (1 << 26) != 0,
            escape_rope_allowed: w20 & (1 << 27) != 0,
            fly_allowed: w20 & (1 << 28) != 0,
            outgoing_calls: w20 & (1 << 29) != 0,
            incoming_calls: w20 & (1 << 30) != 0,
            radio_signal: w20 & (1 << 31) != 0,
        })
    }

    /// Re-encodes the record exactly as the compiler laid it out — the
    /// form the ARM9 scan matched.
    #[must_use]
    pub fn encode(&self) -> [u8; MAP_HEADER_SIZE] {
        let mut r = [0u8; MAP_HEADER_SIZE];
        r[0] = self.wild_encounter_bank;
        r[1] = self.area_data_bank;
        let w2 = u16::from(self.move_model_bank & 15)
            | (u16::from(self.world_map_x & 63) << 4)
            | (u16::from(self.world_map_y & 63) << 10);
        r[2..4].copy_from_slice(&w2.to_le_bytes());
        for (i, v) in [
            self.matrix_id,
            self.scripts_bank,
            self.script_header_bank,
            self.msg_bank,
            self.day_music_id,
            self.night_music_id,
            self.events_bank,
        ]
        .iter()
        .enumerate()
        {
            r[4 + i * 2..6 + i * 2].copy_from_slice(&v.to_le_bytes());
        }
        let w18 = u16::from(self.mapsec)
            | (u16::from(self.area_icon & 15) << 8)
            | (u16::from(self.mom_call_intro_param & 15) << 12);
        r[18..20].copy_from_slice(&w18.to_le_bytes());
        let w20 = u32::from(self.region_no & 1)
            | (u32::from(self.weather & 127) << 1)
            | (u32::from(self.map_type as u8) << 8)
            | (u32::from(self.camera_type & 63) << 12)
            | (u32::from(self.follow_mode as u8) << 18)
            | (u32::from(self.battle_bg & 31) << 20)
            | (u32::from(self.bike_allowed) << 25)
            | (u32::from(self.running_allowed_unused) << 26)
            | (u32::from(self.escape_rope_allowed) << 27)
            | (u32::from(self.fly_allowed) << 28)
            | (u32::from(self.outgoing_calls) << 29)
            | (u32::from(self.incoming_calls) << 30)
            | (u32::from(self.radio_signal) << 31);
        r[20..24].copy_from_slice(&w20.to_le_bytes());
        r
    }

    /// `MapHeader_HasWildEncounters`.
    #[must_use]
    pub fn has_wild_encounters(&self) -> bool {
        self.wild_encounter_bank != NO_WILD_ENCOUNTERS
    }
}

/// RAM address of map `id`'s record.
#[must_use]
pub fn record_address(id: u16) -> u32 {
    MAP_HEADERS_ADDRESS + u32::from(id) * MAP_HEADER_SIZE as u32
}

/// The whole `sMapHeaders` table plus the map name table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapHeaders {
    headers: Vec<MapHeader>,
    names: Vec<String>,
}

impl MapHeaders {
    /// Reads the table from the store's ARM9 image and the names from
    /// NitroFS.
    ///
    /// # Errors
    /// Returns an [`AssetsError`] when the image or name file is missing
    /// or a record fails [`MapHeader::parse`].
    pub fn load(store: &AssetStore) -> Result<Self, AssetsError> {
        let image = store.arm9_image()?;
        let start = (MAP_HEADERS_ADDRESS - store.arm9_base()) as usize;
        let table = image
            .get(start..start + MAP_HEADER_COUNT * MAP_HEADER_SIZE)
            .ok_or_else(|| AssetsError::Missing("sMapHeaders past the ARM9 image".into()))?;
        let names = store.nitrofs_file(MAP_NAMES_PATH)?;
        Self::parse(table, names).map_err(|source| AssetsError::Corrupt {
            what: "sMapHeaders".into(),
            source,
        })
    }

    /// Decodes a raw table (`MAP_HEADER_COUNT` records) and name file.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when either input is short or a record is
    /// malformed.
    pub fn parse(table: &[u8], names: &[u8]) -> Result<Self, NdsError> {
        if table.len() < MAP_HEADER_COUNT * MAP_HEADER_SIZE {
            return Err(NdsError::Truncated {
                what: "map header table",
                need: MAP_HEADER_COUNT * MAP_HEADER_SIZE,
                got: table.len(),
            });
        }
        if names.len() < MAP_HEADER_COUNT * MAP_NAME_SIZE {
            return Err(NdsError::Truncated {
                what: "map name table",
                need: MAP_HEADER_COUNT * MAP_NAME_SIZE,
                got: names.len(),
            });
        }
        let headers = table
            .chunks_exact(MAP_HEADER_SIZE)
            .take(MAP_HEADER_COUNT)
            .map(MapHeader::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let names = names
            .chunks_exact(MAP_NAME_SIZE)
            .take(MAP_HEADER_COUNT)
            .map(|n| {
                let n = n.split(|&b| b == 0).next().unwrap_or_default();
                std::str::from_utf8(n)
                    .map(str::to_owned)
                    .map_err(|_| NdsError::Invalid {
                        what: "map name",
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { headers, names })
    }

    /// Map `id`'s header, if `id < MAP_HEADER_COUNT`.
    #[must_use]
    pub fn get(&self, id: u16) -> Option<&MapHeader> {
        self.headers.get(usize::from(id))
    }

    /// Map `id`'s internal name (`T20R0202` for the player's bedroom).
    #[must_use]
    pub fn name(&self, id: u16) -> Option<&str> {
        self.names.get(usize::from(id)).map(String::as_str)
    }

    /// Number of records (always [`MAP_HEADER_COUNT`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.headers.len()
    }

    /// Whether the table is empty (never, for a loaded table).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }

    /// All headers in id order.
    pub fn iter(&self) -> impl Iterator<Item = &MapHeader> {
        self.headers.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Map 64 (`MAP_NEW_BARK_PLAYER_HOUSE_2F`) as `src/data/map_headers.h`
    /// spells it, with the constants resolved.
    pub(crate) fn bedroom() -> MapHeader {
        MapHeader {
            wild_encounter_bank: NO_WILD_ENCOUNTERS,
            area_data_bank: 25,
            move_model_bank: 15,
            world_map_x: 21,
            world_map_y: 12,
            matrix_id: 72,
            scripts_bank: 846,
            script_header_bank: 619,
            msg_bank: 546,
            day_music_id: 1018,
            night_music_id: 1018,
            events_bank: 61,
            mapsec: 126,
            area_icon: 9,
            mom_call_intro_param: 0,
            region_no: 0,
            weather: 0,
            map_type: MapType::Interior,
            camera_type: 4,
            follow_mode: FollowMode::HeightRestrict,
            battle_bg: 6,
            bike_allowed: false,
            running_allowed_unused: true,
            escape_rope_allowed: false,
            fly_allowed: false,
            outgoing_calls: true,
            incoming_calls: true,
            radio_signal: true,
        }
    }

    #[test]
    fn bedroom_record_roundtrips_through_the_bitfield_packing() {
        let h = bedroom();
        let bytes = h.encode();
        assert_eq!(&bytes[..4], &[0xFF, 0x19, 0x5F, 0x31]);
        assert_eq!(&bytes[20..], &[0x00, 0x44, 0x64, 0xE4]);
        assert_eq!(MapHeader::parse(&bytes).unwrap(), h);
        assert!(!h.has_wild_encounters());
    }

    #[test]
    fn out_of_range_enumerators_are_rejected() {
        let mut bytes = bedroom().encode();
        bytes[21] = 0x0F; // mapType 15
        assert!(MapHeader::parse(&bytes).is_err());
        let mut bytes = bedroom().encode();
        bytes[22] |= 0x0C; // followMode 3
        assert!(MapHeader::parse(&bytes).is_err());
        assert!(MapHeader::parse(&bytes[..10]).is_err());
    }

    #[test]
    fn record_addresses_step_by_24() {
        assert_eq!(record_address(0), MAP_HEADERS_ADDRESS);
        assert_eq!(record_address(64), 0x020F_71E0);
    }

    #[test]
    fn table_parse_needs_all_records_and_names() {
        let table = vec![0u8; MAP_HEADER_COUNT * MAP_HEADER_SIZE];
        let names = vec![0u8; MAP_HEADER_COUNT * MAP_NAME_SIZE];
        let t = MapHeaders::parse(&table, &names).unwrap();
        assert_eq!(t.len(), MAP_HEADER_COUNT);
        assert_eq!(t.name(0), Some(""));
        assert!(t.get(540).is_none());
        assert!(MapHeaders::parse(&table[..100], &names).is_err());
    }
}
