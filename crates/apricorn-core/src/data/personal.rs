//! Species-level game data: base stats, growth curves, learnsets,
//! evolutions, and the egg-species map.
//!
//! Every record here is a fixed-layout binary row straight out of a NARC
//! in the ROM (see [`super`] for the archive paths and pret citations).
//! The layouts mirror pret's structs exactly — `BASE_STATS`
//! (`include/pokemon_types_def.h`), the growth-curve rows
//! (`files/poketool/personal/growtbl.txt`), the level-up learnset rows
//! (`NARC_poketool_personal_wotbl`), and the evolution rows
//! (`files/poketool/personal/evo.json.txt`) — so the ROM remains the only
//! data source; nothing is hand-typed.
//!
//! Field semantics pret has not named (bit padding, unknown bytes) are
//! validated against the retail data — zero where the generator writes
//! zero — and rejected otherwise, so a drifted dump fails loudly instead
//! of parsing to plausible-looking garbage.

use crate::nds::{NdsError, u16le, u32le};

use super::exact;

/// Experience growth-curve ids — pret `constants/pokemon.h` `GROWTH_*`.
///
/// Each id names one [`GrowthTable`] member of `a/0/0/3`; 6 and 7 are
/// unused slots the table still carries.
pub mod growth {
    /// Medium-fast: n³ (level 100 → 1,000,000).
    pub const MEDIUM_FAST: u8 = 0;
    /// Erratic (level 100 → 600,000).
    pub const ERRATIC: u8 = 1;
    /// Fluctuating (level 100 → 1,640,000).
    pub const FLUCTUATING: u8 = 2;
    /// Medium-slow (level 100 → 1,059,860).
    pub const MEDIUM_SLOW: u8 = 3;
    /// Fast: 4n³/5 (level 100 → 800,000).
    pub const FAST: u8 = 4;
    /// Slow: 5n³/4 (level 100 → 1,250,000).
    pub const SLOW: u8 = 5;
    /// Unused curve slot 6.
    pub const UNUSED_6: u8 = 6;
    /// Unused curve slot 7.
    pub const UNUSED_7: u8 = 7;
}

/// One species row of `personal` (`a/0/0/2`): pret's 0x2C-byte
/// `BASE_STATS`, byte-for-byte.
///
/// Index 0 is the empty slot, 1–493 the national species, and 494–507
/// the alternate-form rows (Deoxys, Wormadam, Giratina, Shaymin, Rotom —
/// pret `constants/species.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseStats {
    /// Base HP.
    pub hp: u8,
    /// Base Attack.
    pub atk: u8,
    /// Base Defense.
    pub def: u8,
    /// Base Speed.
    pub speed: u8,
    /// Base Special Attack.
    pub spatk: u8,
    /// Base Special Defense.
    pub spdef: u8,
    /// Type pair, pret `TYPE_*` ids (0 = Normal … 17 = Dark; the pair is
    /// `[t, t]` for single-typed species).
    pub types: [u8; 2],
    /// Catch rate.
    pub catch_rate: u8,
    /// Experience yield on defeat.
    pub exp_yield: u8,
    /// Effort-value yields (each 0–3).
    pub ev_yields: EvYields,
    /// Wild held item 1 (50% chance when item 2 differs).
    pub item_1: u16,
    /// Wild held item 2 (5% chance when item 1 differs).
    pub item_2: u16,
    /// Gender ratio code — 0 = genderless, 255 = always male; otherwise
    /// `♀` chance is `(code - 1) / 254`. Interpreted by battle code
    /// (Phase 6), kept raw here.
    pub gender_ratio: u8,
    /// Egg cycles until hatching.
    pub egg_cycles: u8,
    /// Base friendship.
    pub friendship: u8,
    /// Growth curve id, a [`growth`] constant.
    pub growth_rate: u8,
    /// Egg-group pair, 0 = none.
    pub egg_groups: [u8; 2],
    /// Ability pair (ability 2 is 0 for single-ability species).
    pub abilities: [u8; 2],
    /// Great Marsh flee rate (a Sinnoh leftover; zero everywhere in HGSS).
    pub great_marsh_rate: u8,
    /// Pokédex color, low 7 bits.
    pub color: u8,
    /// Whether the Pokédex picture is flipped.
    pub flip: bool,
    /// The 128 TM/HM compatibility bits: bit `tmhm` of the little-endian
    /// words (bit 0 of the first word is TM01), as read by
    /// [`BaseStats::tmhm_bit`].
    pub tmhm: [u32; 4],
}

impl BaseStats {
    /// Bytes in one row.
    pub const SIZE: usize = 0x2C;

    /// Parses one `personal` member.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the member is not exactly
    /// [`SIZE`](Self::SIZE) bytes or the generator-zero padding is not
    /// zero (a drifted dump).
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        exact(data, Self::SIZE, "personal entry")?;
        let ev_yields = EvYields {
            hp: data[0x0A] & 3,
            atk: (data[0x0A] >> 2) & 3,
            def: (data[0x0A] >> 4) & 3,
            speed: (data[0x0A] >> 6) & 3,
            spatk: data[0x0B] & 3,
            spdef: (data[0x0B] >> 2) & 3,
        };
        if data[0x0B] >> 4 != 0 {
            return Err(NdsError::Invalid {
                what: "personal EV-yield bit padding is not zero",
            });
        }
        if data[0x1A] != 0 || data[0x1B] != 0 {
            return Err(NdsError::Invalid {
                what: "personal entry's padding at 0x1A is not zero",
            });
        }
        Ok(Self {
            hp: data[0x00],
            atk: data[0x01],
            def: data[0x02],
            speed: data[0x03],
            spatk: data[0x04],
            spdef: data[0x05],
            types: [data[0x06], data[0x07]],
            catch_rate: data[0x08],
            exp_yield: data[0x09],
            ev_yields,
            item_1: u16le(data, 0x0C)?,
            item_2: u16le(data, 0x0E)?,
            gender_ratio: data[0x10],
            egg_cycles: data[0x11],
            friendship: data[0x12],
            growth_rate: data[0x13],
            egg_groups: [data[0x14], data[0x15]],
            abilities: [data[0x16], data[0x17]],
            great_marsh_rate: data[0x18],
            color: data[0x19] & 0x7F,
            flip: data[0x19] >> 7 != 0,
            tmhm: [
                u32le(data, 0x1C)?,
                u32le(data, 0x20)?,
                u32le(data, 0x24)?,
                u32le(data, 0x28)?,
            ],
        })
    }

    /// Whether the species can learn TM/HM number `tmhm` — bit
    /// `tmhm % 32` of word `tmhm / 32`, exactly as pret's
    /// `GetTMHMCompatBySpeciesAndForm` (`src/pokemon.c`) reads it.
    /// TM01 is 0 … HM08 is 99; the top 28 bits are unused.
    #[must_use]
    pub fn tmhm_bit(&self, tmhm: u8) -> bool {
        self.tmhm[usize::from(tmhm / 32)] >> (tmhm % 32) & 1 != 0
    }
}

/// A species' effort-value yields, packed two bits each across bytes
/// 0x0A–0x0B of [`BaseStats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvYields {
    /// HP EVs gained on defeat.
    pub hp: u8,
    /// Attack EVs gained on defeat.
    pub atk: u8,
    /// Defense EVs gained on defeat.
    pub def: u8,
    /// Speed EVs gained on defeat.
    pub speed: u8,
    /// Special Attack EVs gained on defeat.
    pub spatk: u8,
    /// Special Defense EVs gained on defeat.
    pub spdef: u8,
}

/// One experience growth curve: one `growtbl` (`a/0/0/3`) member, a
/// `lv000`…`lv100` row of u32s (pret `files/poketool/personal/
/// growtbl.txt`). Member id = growth-rate id (a [`growth`] constant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrowthTable([u32; super::GROWTH_LEVELS]);

impl GrowthTable {
    /// Parses one growth-curve member.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the member is not exactly
    /// 101 × 4 bytes.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        exact(data, super::GROWTH_LEVELS * 4, "growth curve")?;
        let mut table = [0u32; super::GROWTH_LEVELS];
        for (level, exp) in table.iter_mut().enumerate() {
            *exp = u32le(data, 4 * level)?;
        }
        Ok(Self(table))
    }

    /// The total experience needed to *reach* `level` (0–100); index 0
    /// is the lv000 column and is always zero.
    #[must_use]
    pub fn exp(&self, level: u8) -> u32 {
        self.0[usize::from(level)]
    }
}

/// One `(level, move)` pair of a level-up learnset — the `wotbl`
/// (`a/0/3/3`) entry encoding `(level << 9) | move`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LearnMove {
    /// The level at which the move is learned (top 7 bits of the entry).
    pub level: u8,
    /// The move learned, a `waza` row id.
    pub move_id: u16,
}

/// One species' level-up learnset — a `wotbl` (`a/0/3/3`) member:
/// `(level << 9) | move` entries, terminated by 0xFFFF, then zero
/// padding out to the archive's 4-byte member alignment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Learnset {
    /// The moves in learn order (ascending level).
    entries: Vec<LearnMove>,
}

impl Learnset {
    /// Parses one learnset member.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the member is odd-length, has no
    /// 0xFFFF terminator, or carries nonzero bytes after it.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        if !data.len().is_multiple_of(2) {
            return Err(NdsError::Invalid {
                what: "learnset is not a whole number of entries",
            });
        }
        let mut entries = Vec::new();
        for id in 0..data.len() / 2 {
            let word = u16le(data, 2 * id)?;
            if word == 0xFFFF {
                let tail = &data[2 * (id + 1)..];
                if tail.iter().any(|&b| b != 0) {
                    return Err(NdsError::Invalid {
                        what: "learnset carries nonzero bytes after its terminator",
                    });
                }
                return Ok(Self { entries });
            }
            entries.push(LearnMove {
                level: (word >> 9) as u8,
                move_id: word & 0x1FF,
            });
        }
        Err(NdsError::Invalid {
            what: "learnset is not 0xFFFF-terminated",
        })
    }

    /// The learnable moves in learn order.
    #[must_use]
    pub fn entries(&self) -> &[LearnMove] {
        &self.entries
    }
}

/// `EVO_NONE` — the evolution method that terminates a species' row
/// (pret `constants/pokemon.h`).
pub const EVO_NONE: u16 = 0;

/// One evolution possibility — a 6-byte `(method, param, target)`
/// triple, pret `struct Evolution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Evolution {
    /// The `EVO_*` method id (pret `constants/pokemon.h`); 0
    /// ([`EVO_NONE`]) terminates the list.
    pub method: u16,
    /// The method's parameter — a level, an item id, or unused (0).
    pub param: u16,
    /// The species evolved into.
    pub target: u16,
}

/// One species' evolution possibilities — an `evo` (`a/0/3/4`) member:
/// pret's `EVOLUTION_FILE`, seven [`Evolution`] slots plus a zero u16
/// pad out to the archive's 4-byte member alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvolutionTable {
    /// All seven slots, terminator padding included; use
    /// [`Self::entries`] for the active list.
    slots: [Evolution; super::EVOLUTION_SLOTS],
}

impl EvolutionTable {
    /// Parses one evolution member.
    ///
    /// # Errors
    /// Returns an [`NdsError`] when the member is not 44 bytes or its
    /// trailing pad word is not zero.
    pub fn parse(data: &[u8]) -> Result<Self, NdsError> {
        exact(data, super::EVOLUTION_SLOTS * 6 + 2, "evolution entry")?;
        if u16le(data, super::EVOLUTION_SLOTS * 6)? != 0 {
            return Err(NdsError::Invalid {
                what: "evolution entry's padding word is not zero",
            });
        }
        let mut slots = [Evolution {
            method: EVO_NONE,
            param: 0,
            target: 0,
        }; super::EVOLUTION_SLOTS];
        for (slot, entry) in slots.iter_mut().enumerate() {
            *entry = Evolution {
                method: u16le(data, 6 * slot)?,
                param: u16le(data, 6 * slot + 2)?,
                target: u16le(data, 6 * slot + 4)?,
            };
        }
        Ok(Self { slots })
    }

    /// The active evolution list: every slot up to the first
    /// [`EVO_NONE`] terminator (possibly empty).
    #[must_use]
    pub fn entries(&self) -> &[Evolution] {
        let end = self
            .slots
            .iter()
            .position(|e| e.method == EVO_NONE)
            .unwrap_or(self.slots.len());
        &self.slots[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `personal` fixture: Bulbasaur-shaped, with distinct padding-area
    /// probes left zero.
    fn personal_bytes() -> Vec<u8> {
        let mut b = vec![0u8; BaseStats::SIZE];
        b[0..6].copy_from_slice(&[45, 49, 49, 45, 65, 65]);
        b[6..8].copy_from_slice(&[12, 3]); // Grass, Poison
        b[8] = 45; // catch rate
        b[9] = 64; // exp yield
        b[0x0B] = 1; // 1 Sp.Atk EV, zero pad nibble
        b[0x0C..0x0E].copy_from_slice(&5u16.to_le_bytes()); // item 1
        b[0x0E..0x10].copy_from_slice(&0xBu16.to_le_bytes()); // item 2
        b[0x10] = 31; // 12.5% female
        b[0x11] = 20; // egg cycles
        b[0x12] = 70; // friendship
        b[0x13] = growth::MEDIUM_SLOW;
        b[0x14..0x16].copy_from_slice(&[1, 7]); // Monster, Grass
        b[0x16] = 65; // Overgrow
        b[0x18] = 9; // great-marsh rate
        b[0x19] = 0x83; // color 3 (Green) + flip
        b[0x1C..0x20].copy_from_slice(&0x8435_0720u32.to_le_bytes());
        b[0x20..0x24].copy_from_slice(&0x0210_1E08u32.to_le_bytes());
        b[0x24..0x28].copy_from_slice(&0x9266_2420u32.to_le_bytes());
        b[0x28..0x2C].copy_from_slice(&2u32.to_le_bytes());
        b
    }

    #[test]
    fn parses_every_personal_field() {
        let stats = BaseStats::parse(&personal_bytes()).expect("fixture parses");
        assert_eq!((stats.hp, stats.atk, stats.def), (45, 49, 49));
        assert_eq!((stats.speed, stats.spatk, stats.spdef), (45, 65, 65));
        assert_eq!(stats.types, [12, 3]);
        assert_eq!(stats.catch_rate, 45);
        assert_eq!(stats.exp_yield, 64);
        assert_eq!(
            stats.ev_yields,
            EvYields {
                hp: 0,
                atk: 0,
                def: 0,
                speed: 0,
                spatk: 1,
                spdef: 0,
            }
        );
        assert_eq!((stats.item_1, stats.item_2), (5, 0xB));
        assert_eq!(stats.gender_ratio, 31);
        assert_eq!(stats.egg_cycles, 20);
        assert_eq!(stats.friendship, 70);
        assert_eq!(stats.growth_rate, growth::MEDIUM_SLOW);
        assert_eq!(stats.egg_groups, [1, 7]);
        assert_eq!(stats.abilities, [65, 0]);
        assert_eq!(stats.great_marsh_rate, 9);
        assert_eq!(stats.color, 3);
        assert!(stats.flip);
        assert_eq!(stats.tmhm, [0x8435_0720, 0x0210_1E08, 0x9266_2420, 2]);
    }

    #[test]
    fn tmhm_bits_read_word_then_bit() {
        // The real Bulbasaur words: TM22 (SolarBeam, bit 21) and HM01
        // (Cut, bit 92) yes, TM26 (Earthquake, bit 25) and HM08 no —
        // pinned from the retail table by tests/data_hg.rs.
        let stats = BaseStats::parse(&personal_bytes()).expect("fixture parses");
        assert!(stats.tmhm_bit(21), "TM22");
        assert!(!stats.tmhm_bit(25), "TM26");
        assert!(stats.tmhm_bit(92), "HM01 Cut");
        assert!(!stats.tmhm_bit(99), "HM08");
    }

    #[test]
    fn rejects_wrong_sizes_and_drifted_padding() {
        let good = personal_bytes();
        assert!(BaseStats::parse(&good[..43]).is_err(), "truncated");
        let mut long = good.clone();
        long.push(0);
        assert!(BaseStats::parse(&long).is_err(), "trailing slack");

        let mut drift = good.clone();
        drift[0x0B] = 0x10; // EV-yield pad nibble nonzero
        assert!(BaseStats::parse(&drift).is_err());

        let mut drift = good.clone();
        drift[0x1A] = 1; // struct padding
        assert!(BaseStats::parse(&drift).is_err());
    }

    #[test]
    fn growth_curve_is_indexed_by_level() {
        let mut b = Vec::new();
        for level in 0..super::super::GROWTH_LEVELS {
            b.extend_from_slice(&(level as u32 * 7).to_le_bytes());
        }
        let curve = GrowthTable::parse(&b).expect("fixture parses");
        assert_eq!(curve.exp(0), 0);
        assert_eq!(curve.exp(1), 7);
        assert_eq!(curve.exp(100), 700);

        assert!(GrowthTable::parse(&b[..b.len() - 4]).is_err());
        assert!(GrowthTable::parse(&b[..b.len() - 3]).is_err());
        let mut odd = b.clone();
        odd.push(0);
        assert!(GrowthTable::parse(&odd).is_err());
    }

    #[test]
    fn learnset_reads_entries_until_terminator() {
        // (1, Tackle), (3, Growl), terminator, one zero pad word.
        let entries = [(1u16 << 9) | 33, (3u16 << 9) | 45];
        let mut b = Vec::new();
        for e in entries {
            b.extend_from_slice(&e.to_le_bytes());
        }
        b.extend_from_slice(&0xFFFFu16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        let learnset = Learnset::parse(&b).expect("fixture parses");
        assert_eq!(
            learnset.entries(),
            &[
                LearnMove {
                    level: 1,
                    move_id: 33
                },
                LearnMove {
                    level: 3,
                    move_id: 45
                },
            ]
        );

        // No terminator, nonzero tail, and odd length are corruption.
        let mut bad = b.clone();
        bad.truncate(bad.len() - 4); // terminator and pad removed
        assert!(Learnset::parse(&bad).is_err());
        let mut bad = b.clone();
        let last = bad.len() - 2;
        bad[last] = 1; // pad after the terminator
        assert!(Learnset::parse(&bad).is_err());
        let mut bad = b.clone();
        bad.push(0);
        assert!(Learnset::parse(&bad).is_err());

        // A terminator alone is the empty learnset.
        let empty = 0xFFFFu16.to_le_bytes();
        let learnset = Learnset::parse(&empty).expect("empty learnset parses");
        assert!(learnset.entries().is_empty());
    }

    #[test]
    fn evolution_table_stops_at_the_first_none_slot() {
        // Bulbasaur: (EVO_LEVEL=4, level 16, Ivysaur), then six empty
        // slots — all 7 slots × 3 words, plus the pad word: 44 bytes.
        let mut b = Vec::new();
        b.extend_from_slice(&[4u16, 16, 2].map(u16::to_le_bytes).concat());
        b.extend_from_slice(&[0u16; 18].map(u16::to_le_bytes).concat());
        b.extend_from_slice(&0u16.to_le_bytes()); // EVOLUTION_FILE pad
        let table = EvolutionTable::parse(&b).expect("fixture parses");
        assert_eq!(
            table.entries(),
            &[Evolution {
                method: 4,
                param: 16,
                target: 2,
            }]
        );

        // All seven slots full: Eevee, whose terminator lives outside the
        // table — entries() then covers every slot.
        let mut b = Vec::new();
        for slot in 0..super::super::EVOLUTION_SLOTS {
            for word in [7u16, 82 + u16::try_from(slot).unwrap(), 134] {
                b.extend_from_slice(&word.to_le_bytes());
            }
        }
        b.extend_from_slice(&0u16.to_le_bytes());
        let table = EvolutionTable::parse(&b).expect("fixture parses");
        assert_eq!(table.entries().len(), super::super::EVOLUTION_SLOTS);

        // Short, long, and nonzero-pad members are corruption.
        assert!(EvolutionTable::parse(&b[..b.len() - 1]).is_err());
        let mut bad = b.clone();
        bad.push(0);
        assert!(EvolutionTable::parse(&bad).is_err());
        let mut bad = b.clone();
        bad[b.len() - 2] = 1; // the pad word
        assert!(EvolutionTable::parse(&bad).is_err());
    }
}
