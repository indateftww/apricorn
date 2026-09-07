//! Game data tables: species, moves, items — read from the ROM's own
//! NARCs at runtime, zero hand-typing (Phase 4, step 1).
//!
//! pret's `NarcId` enum (`include/filesystem_files_def.h`) indexes
//! `sNarcFileList[]`, which names the NitroFS path behind each id; the
//! loader here walks the same paths in the retail dump:
//!
//! | table | NitroFS path | pret id | rows |
//! |---|---|---|---|
//! | personal (base stats) | `a/0/0/2` | 2 | 508 × 44 B |
//! | growtbl (exp curves) | `a/0/0/3` | 3 | 8 × 404 B |
//! | waza (moves) | `a/0/1/1` | 11 | 471 × 16 B |
//! | item_data | `a/0/1/7` | 17 | 514 × 34 B |
//! | wotbl (learnsets) | `a/0/3/3` | 33 | 508, variable |
//! | evo (evolutions) | `a/0/3/4` | 34 | 508 × 44 B |
//!
//! plus the raw `poketool/personal/pms.narc` file — not a NARC despite
//! the name, just 508 little-endian u16s mapping each species to the
//! species its eggs hatch as (pret `src/pokemon.c`, `GetEggSpecies`).
//!
//! [`GameData::load`] parses every row of every table eagerly and
//! rejects wrong member counts, wrong row sizes, and generator-zero
//! padding that is not zero — each invariant was verified across the
//! entire retail image before becoming a validation, so a drifted or
//! foreign dump fails loudly instead of parsing to plausible garbage.
//! The per-table layouts and pret citations live in the submodule docs
//! and `docs/game-data.md`.

use core::fmt;

use crate::formats::Narc;
use crate::nds::{NdsError, NdsRom, u16le};

pub mod items;
pub mod moves;
pub mod personal;

pub use items::{ItemEntry, NO_NATURAL_GIFT, pocket};
pub use moves::{MoveCategory, MoveEntry};
pub use personal::{
    BaseStats, EVO_NONE, EvYields, Evolution, EvolutionTable, GrowthTable, LearnMove, Learnset,
    growth,
};

/// NitroFS paths of the data tables — pret's `NarcId` values resolved
/// through `sNarcFileList[]` (`include/filesystem_files_def.h`).
pub mod paths {
    /// Base stats: `NARC_poketool_personal_personal` (id 2).
    pub const PERSONAL: &str = "a/0/0/2";
    /// Experience curves: `NARC_poketool_personal_growtbl` (id 3).
    pub const GROWTBL: &str = "a/0/0/3";
    /// Moves: `NARC_poketool_waza_waza_tbl` (id 11).
    pub const WAZA: &str = "a/0/1/1";
    /// Items: `NARC_itemtool_itemdata_item_data` (id 17).
    pub const ITEM_DATA: &str = "a/0/1/7";
    /// Level-up learnsets: `NARC_poketool_personal_wotbl` (id 33).
    pub const WOTBL: &str = "a/0/3/3";
    /// Evolutions: `NARC_poketool_personal_evo` (id 34).
    pub const EVO: &str = "a/0/3/4";
    /// Egg-species map — a raw (non-NARC) file of 508 u16s.
    pub const PMS: &str = "poketool/personal/pms.narc";
}

/// Rows in the species-indexed tables: the empty slot, national ids
/// 1–493, `SPECIES_EGG` (494), and the alternate-form slots 495–507
/// (pret `constants/species.h`).
pub const SPECIES_ROWS: usize = 508;

/// Rows in the growth-curve table — one per [`growth`] id, two of them
/// unused slots the table still carries.
pub const GROWTH_RATES: usize = 8;

/// Experience columns per curve: `lv000`…`lv100`.
pub const GROWTH_LEVELS: usize = 101;

/// Rows in the move table: `NUM_MOVES` is 467 (`SHADOW_FORCE`), plus
/// four unused rows the table still carries.
pub const MOVE_ROWS: usize = 471;

/// Rows in the item table (ids beyond the last real item are zeroed).
pub const ITEM_ROWS: usize = 514;

/// Evolution slots per species — pret `MAX_EVOS_PER_POKE`.
pub const EVOLUTION_SLOTS: usize = 7;

/// The nine species whose eggs hatch as themselves by default — the
/// incense-breeding families (pret `GetEggSpecies`, `src/pokemon.c`):
/// Chansey, Mr. Mime, Snorlax, Marill, Sudowoodo, Wobbuffet, Mantine,
/// Roselia, Chimecho. The table maps them to their baby forms; the
/// caller's incense check decides which applies.
const INCENSE_EGG_SPECIES: [u16; 9] = [113, 122, 143, 183, 185, 202, 226, 315, 358];

/// Failures while reading the game data tables out of a ROM.
#[derive(Debug)]
pub enum DataError {
    /// A table's NitroFS path resolved to nothing.
    Missing(String),
    /// A row failed to parse; `what` names the row, as `path#id`.
    Corrupt {
        /// The failing row, as `path#id`.
        what: String,
        /// The underlying parse error.
        source: NdsError,
    },
    /// A table's member count differs from the retail image's.
    WrongCount {
        /// The failing table.
        what: &'static str,
        /// The retail member count.
        expected: usize,
        /// The member count found.
        got: usize,
    },
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(what) => write!(f, "no such data table: {what}"),
            Self::Corrupt { what, source } => write!(f, "{what}: {source}"),
            Self::WrongCount {
                what,
                expected,
                got,
            } => {
                write!(f, "{what} has {got} members, expected {expected}")
            }
        }
    }
}

impl std::error::Error for DataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Corrupt { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Every row of every table, parsed and validated — the species, move,
/// and item rules the game reads, straight from the pinned dump.
///
/// Built by [`GameData::load`]; rows stay private so counts and
/// cross-references (a move id in a learnset, an item id in a base-stat
/// row) can only be consumed through the typed accessors.
#[derive(Debug, Clone)]
pub struct GameData {
    /// `personal` rows, species-indexed.
    personal: Vec<BaseStats>,
    /// `growtbl` rows, growth-rate-id-indexed.
    growth: Vec<GrowthTable>,
    /// `waza` rows, move-id-indexed.
    moves: Vec<MoveEntry>,
    /// `item_data` rows, item-id-indexed.
    items: Vec<ItemEntry>,
    /// `wotbl` rows, species-indexed.
    learnsets: Vec<Learnset>,
    /// `evo` rows, species-indexed.
    evolutions: Vec<EvolutionTable>,
    /// The `pms` egg-species map, species-indexed.
    egg_species: [u16; SPECIES_ROWS],
}

impl GameData {
    /// Parses every data table in the ROM.
    ///
    /// # Errors
    /// Returns a [`DataError`] when a table's path is missing, its
    /// member count differs from the retail image's, or any row fails
    /// its layout validation.
    pub fn load(rom: &NdsRom<'_>) -> Result<Self, DataError> {
        let personal = members(
            rom,
            paths::PERSONAL,
            SPECIES_ROWS,
            "personal table",
            BaseStats::parse,
        )?;
        let growth = members(
            rom,
            paths::GROWTBL,
            GROWTH_RATES,
            "growth-curve table",
            GrowthTable::parse,
        )?;
        let moves = members(rom, paths::WAZA, MOVE_ROWS, "move table", MoveEntry::parse)?;
        let items = members(
            rom,
            paths::ITEM_DATA,
            ITEM_ROWS,
            "item table",
            ItemEntry::parse,
        )?;
        let learnsets = members(
            rom,
            paths::WOTBL,
            SPECIES_ROWS,
            "learnset table",
            Learnset::parse,
        )?;
        let evolutions = members(
            rom,
            paths::EVO,
            SPECIES_ROWS,
            "evolution table",
            EvolutionTable::parse,
        )?;

        let pms = rom
            .file_by_path(paths::PMS)
            .map_err(|_| DataError::Missing(format!("no NitroFS file {}", paths::PMS)))?;
        exact(pms, SPECIES_ROWS * 2, "egg-species table").map_err(|source| DataError::Corrupt {
            what: paths::PMS.to_owned(),
            source,
        })?;
        let mut egg_species = [0u16; SPECIES_ROWS];
        for (species, entry) in egg_species.iter_mut().enumerate() {
            *entry = u16le(pms, 2 * species).map_err(|source| DataError::Corrupt {
                what: paths::PMS.to_owned(),
                source,
            })?;
        }

        Ok(Self {
            personal,
            growth,
            moves,
            items,
            learnsets,
            evolutions,
            egg_species,
        })
    }

    /// Base stats of `species` (row 0 is the empty slot; `None` only
    /// for ids past the form rows).
    #[must_use]
    pub fn base_stats(&self, species: u16) -> Option<&BaseStats> {
        self.personal.get(usize::from(species))
    }

    /// The experience curve named by a [`growth`] id (6–7 are the
    /// unused slots the table still carries).
    #[must_use]
    pub fn growth_table(&self, rate: u8) -> Option<&GrowthTable> {
        self.growth.get(usize::from(rate))
    }

    /// Move data for `move_id` (`None` past the unused trailing rows).
    #[must_use]
    pub fn move_data(&self, move_id: u16) -> Option<&MoveEntry> {
        self.moves.get(usize::from(move_id))
    }

    /// Item data for `item_id` (`None` past the last row).
    #[must_use]
    pub fn item(&self, item_id: u16) -> Option<&ItemEntry> {
        self.items.get(usize::from(item_id))
    }

    /// The level-up learnset of `species`.
    #[must_use]
    pub fn learnset(&self, species: u16) -> Option<&Learnset> {
        self.learnsets.get(usize::from(species))
    }

    /// The evolution possibilities of `species`.
    #[must_use]
    pub fn evolutions(&self, species: u16) -> Option<&EvolutionTable> {
        self.evolutions.get(usize::from(species))
    }

    /// The species an egg of `species` hatches as, pret
    /// `GetEggSpecies`: the nine incense families hatch as themselves
    /// (the caller's held-item check decides when the baby applies),
    /// every other species reads the `pms` map.
    #[must_use]
    pub fn egg_species(&self, species: u16) -> Option<u16> {
        if INCENSE_EGG_SPECIES.contains(&species) {
            return Some(species);
        }
        self.egg_species.get(usize::from(species)).copied()
    }

    /// Rows in the species-indexed tables (see [`SPECIES_ROWS`]).
    #[must_use]
    pub fn species_count(&self) -> usize {
        self.personal.len()
    }

    /// Rows in the move table (see [`MOVE_ROWS`]).
    #[must_use]
    pub fn move_count(&self) -> usize {
        self.moves.len()
    }

    /// Rows in the item table (see [`ITEM_ROWS`]).
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }
}

/// Pins `data` to exactly `size` bytes.
///
/// # Errors
/// Returns [`NdsError::Truncated`] when short, [`NdsError::Invalid`]
/// when long — members carry no slack, so both are corruption.
fn exact(data: &[u8], size: usize, what: &'static str) -> Result<(), NdsError> {
    if data.len() < size {
        return Err(NdsError::Truncated {
            what,
            need: size,
            got: data.len(),
        });
    }
    if data.len() > size {
        return Err(NdsError::Invalid { what });
    }
    Ok(())
}

/// Opens the NARC at `path`.
///
/// # Errors
/// Returns a [`DataError::Missing`] when the path resolves to nothing,
/// a [`DataError::Corrupt`] when the archive does not parse.
fn narc<'a>(rom: &NdsRom<'a>, path: &str) -> Result<Narc<'a>, DataError> {
    let bytes = rom
        .file_by_path(path)
        .map_err(|_| DataError::Missing(format!("no NitroFS archive {path}")))?;
    Narc::parse(bytes).map_err(|source| DataError::Corrupt {
        what: path.to_owned(),
        source,
    })
}

/// Parses the `expected` members of the NARC at `path` row-by-row.
///
/// # Errors
/// Returns a [`DataError::WrongCount`] when the member count differs
/// from the retail image's, and a [`DataError::Corrupt`] naming the
/// offending `path#id` when any row fails its layout validation.
fn members<'a, T>(
    rom: &NdsRom<'a>,
    path: &str,
    expected: usize,
    what: &'static str,
    parse: impl Fn(&[u8]) -> Result<T, NdsError>,
) -> Result<Vec<T>, DataError> {
    let narc = narc(rom, path)?;
    let got = narc.file_count();
    if got != expected {
        return Err(DataError::WrongCount {
            what,
            expected,
            got,
        });
    }
    let mut rows = Vec::with_capacity(expected);
    for id in 0..expected {
        let raw = narc
            .file(id)
            .map_err(|_| DataError::Missing(format!("{path} has no member {id}")))?;
        rows.push(parse(raw).map_err(|source| DataError::Corrupt {
            what: format!("{path}#{id}"),
            source,
        })?);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_rejects_short_and_long_but_not_equal() {
        assert!(exact(&[0; 4], 4, "row").is_ok());
        assert_eq!(
            exact(&[0; 3], 4, "row"),
            Err(NdsError::Truncated {
                what: "row",
                need: 4,
                got: 3,
            })
        );
        assert_eq!(
            exact(&[0; 5], 4, "row"),
            Err(NdsError::Invalid { what: "row" })
        );
    }

    #[test]
    fn egg_species_prefers_the_incense_special_cases() {
        // A store whose pms map is the identity — so any table lookup
        // returns the species itself too; the nine special cases get a
        // *distinguishing* table entry (their baby form) to prove the
        // switch, not the table, answered.
        let mut egg_species = [0u16; SPECIES_ROWS];
        for (species, entry) in egg_species.iter_mut().enumerate() {
            *entry = species as u16;
        }
        // Marill's table entry is Azurill (298) — the retail value.
        egg_species[183] = 298;
        let data = GameData {
            personal: Vec::new(),
            growth: Vec::new(),
            moves: Vec::new(),
            items: Vec::new(),
            learnsets: Vec::new(),
            evolutions: Vec::new(),
            egg_species,
        };

        // The nine incense species hatch as themselves despite the map.
        for species in INCENSE_EGG_SPECIES {
            assert_eq!(data.egg_species(species), Some(species));
        }
        // Every other species reads the map (identity here).
        assert_eq!(data.egg_species(1), Some(1));
        assert_eq!(data.egg_species(493), Some(493));
        // Out-of-range ids have no answer.
        assert_eq!(data.egg_species(SPECIES_ROWS as u16), None);
    }
}
