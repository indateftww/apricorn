//! Integration tests: the game data tables against a retail HeartGold
//! (US) dump.
//!
//! [`GameData::load`] already validates every row of every table
//! strictly — sizes, member counts, and generator-zero padding — so a
//! successful load through the real parser is itself the layout test.
//! These tests then pin a handful of well-known rows per table
//! (Bulbasaur, Eevee, the growth curves, classic moves and items)
//! against values established by scanning the retail image while
//! writing the parser; see `docs/game-data.md` for the layouts and
//! the pret citations behind each.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM dump).

use apricorn_core::data::{
    self, EVOLUTION_SLOTS, GameData, LearnMove, MoveCategory, NO_NATURAL_GIFT, growth, pocket,
};
use apricorn_core::nds::NdsRom;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Loads every table from the retail dump, or `None` to skip silently.
fn game_data() -> Option<GameData> {
    let bytes = match std::fs::read(ROM_PATH) {
        Ok(bytes) => bytes,
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            return None;
        }
    };
    let rom = NdsRom::parse(&bytes).expect("retail ROM must parse");
    match GameData::load(&rom) {
        Ok(data) => Some(data),
        Err(err) => panic!("retail tables must parse: {err}"),
    }
}

#[test]
fn loads_every_retail_table() {
    let Some(data) = game_data() else {
        return;
    };
    // The member counts the loaders were written against.
    assert_eq!(data.species_count(), data::SPECIES_ROWS);
    assert_eq!(data.move_count(), data::MOVE_ROWS);
    assert_eq!(data.item_count(), data::ITEM_ROWS);
    // The accessor range ends exactly at the last row.
    assert!(data.base_stats(0).is_some(), "the empty species slot");
    assert!(data.base_stats(493).is_some(), "Arceus");
    assert!(
        data.base_stats(507).is_some(),
        "Rotom Mow, the last form row"
    );
    assert!(data.base_stats(508).is_none());
}

#[test]
fn bulbasaur_pins_the_personal_layout() {
    let Some(data) = game_data() else {
        return;
    };
    let bulba = data.base_stats(1).expect("Bulbasaur");
    assert_eq!((bulba.hp, bulba.atk, bulba.def), (45, 49, 49));
    assert_eq!((bulba.speed, bulba.spatk, bulba.spdef), (45, 65, 65));
    assert_eq!(bulba.types, [12, 3], "Grass, Poison");
    assert_eq!(bulba.catch_rate, 45);
    assert_eq!(bulba.exp_yield, 64);
    assert_eq!(bulba.ev_yields.spatk, 1, "the only nonzero yield");
    assert_eq!(
        (bulba.ev_yields.hp, bulba.ev_yields.atk, bulba.ev_yields.def),
        (0, 0, 0)
    );
    assert_eq!((bulba.ev_yields.speed, bulba.ev_yields.spdef), (0, 0));
    assert_eq!((bulba.item_1, bulba.item_2), (0, 0), "holds nothing");
    assert_eq!(bulba.gender_ratio, 31, "12.5% female");
    assert_eq!(bulba.egg_cycles, 20);
    assert_eq!(bulba.friendship, 70);
    assert_eq!(bulba.growth_rate, growth::MEDIUM_SLOW);
    assert_eq!(bulba.egg_groups, [1, 7], "Monster, Grass");
    assert_eq!(bulba.abilities, [65, 0], "Overgrow, none");
    assert_eq!(bulba.great_marsh_rate, 0, "a Sinnoh leftover");
    assert_eq!(bulba.color, 3, "Green");
    assert!(!bulba.flip);
    assert_eq!(
        bulba.tmhm,
        [0x8435_0720, 0x0210_1E08, 0x9266_2420, 2],
        "the retail TM/HM words"
    );
    // The compatibility bits those words encode (the same reads
    // pret's GetTMHMCompatBySpeciesAndForm makes).
    assert!(bulba.tmhm_bit(21), "TM22 SolarBeam");
    assert!(!bulba.tmhm_bit(25), "TM26 Earthquake");
    assert!(bulba.tmhm_bit(92), "HM01 Cut");
    assert!(!bulba.tmhm_bit(99), "HM08 Rock Climb");
}

#[test]
fn growth_curves_pin_the_experience_table() {
    let Some(data) = game_data() else {
        return;
    };
    // Total experience at the caps, per growth rate id.
    assert_eq!(
        data.growth_table(growth::MEDIUM_FAST)
            .expect("rate 0")
            .exp(100),
        1_000_000
    );
    assert_eq!(
        data.growth_table(growth::ERRATIC).expect("rate 1").exp(100),
        600_000
    );
    assert_eq!(
        data.growth_table(growth::FLUCTUATING)
            .expect("rate 2")
            .exp(100),
        1_640_000
    );
    assert_eq!(
        data.growth_table(growth::MEDIUM_SLOW)
            .expect("rate 3")
            .exp(100),
        1_059_860
    );
    assert_eq!(
        data.growth_table(growth::FAST).expect("rate 4").exp(100),
        800_000
    );
    assert_eq!(
        data.growth_table(growth::SLOW).expect("rate 5").exp(100),
        1_250_000
    );
    // The two unused slots carry a copy of the medium-fast curve.
    for rate in [growth::UNUSED_6, growth::UNUSED_7] {
        let curve = data.growth_table(rate).expect("unused slot still parses");
        assert_eq!(curve.exp(100), 1_000_000);
        assert_eq!(curve.exp(50), 125_000);
    }
    // Every curve starts at zero and level 50 lands where the
    // medium-slow formula says.
    for rate in 0..data::GROWTH_RATES as u8 {
        let curve = data.growth_table(rate).expect("rate row");
        assert_eq!(curve.exp(0), 0);
        assert_eq!(curve.exp(1), 0);
    }
    assert_eq!(
        data.growth_table(growth::MEDIUM_SLOW)
            .expect("rate 3")
            .exp(50),
        117_360
    );
    assert!(
        data.growth_table(data::GROWTH_RATES as u8).is_none(),
        "past the table"
    );
}

#[test]
fn moves_pin_the_waza_layout() {
    let Some(data) = game_data() else {
        return;
    };
    let pound = data.move_data(1).expect("Pound");
    assert_eq!(pound.effect, 0);
    assert_eq!(pound.category, MoveCategory::Physical);
    assert_eq!(pound.power, 40);
    assert_eq!(pound.type_, 0, "Normal");
    assert_eq!(pound.accuracy, 100);
    assert_eq!(pound.pp, 35);
    assert_eq!(pound.effect_chance, 0);
    assert_eq!(pound.priority, 0);
    assert_eq!(
        (pound.unk_0b, pound.unk_0c, pound.contest_type),
        (115, 5, 4)
    );

    let karate = data.move_data(2).expect("Karate Chop");
    assert_eq!(karate.effect, 43, "a high-crit-ratio effect");
    assert_eq!(karate.power, 50);
    assert_eq!(karate.type_, 1, "Fighting");
    assert_eq!(karate.pp, 25);

    let cut = data.move_data(15).expect("Cut");
    assert_eq!((cut.power, cut.accuracy, cut.pp), (50, 95, 30));

    let roar = data.move_data(46).expect("Roar");
    assert_eq!(roar.category, MoveCategory::Status);
    assert_eq!(roar.power, 0);
    assert_eq!(roar.priority, -6, "an early phazing move");

    let thunderbolt = data.move_data(85).expect("Thunderbolt");
    assert_eq!(thunderbolt.category, MoveCategory::Special);
    assert_eq!(thunderbolt.power, 95);
    assert_eq!(thunderbolt.type_, 13, "Electric");
    assert_eq!(thunderbolt.effect_chance, 10, "10% paralysis");

    let quick = data.move_data(98).expect("Quick Attack");
    assert_eq!(quick.effect, 103, "the priority effect");
    assert_eq!(quick.power, 40);
    assert_eq!(quick.pp, 30);
    assert_eq!(quick.priority, 1);

    let flamethrower = data.move_data(126).expect("Flamethrower");
    assert_eq!(flamethrower.category, MoveCategory::Special);
    assert_eq!(flamethrower.type_, 10, "Fire");
    assert_eq!(flamethrower.accuracy, 85);
    assert_eq!(flamethrower.effect_chance, 10, "10% burn");

    let struggle = data.move_data(165).expect("Struggle");
    assert_eq!(struggle.effect, 254);
    assert_eq!(struggle.power, 50);
    assert_eq!(struggle.accuracy, 0, "the effect decides");
    assert_eq!(struggle.pp, 1);

    let shadow_force = data
        .move_data(467)
        .expect("Shadow Force, the last real move");
    assert_eq!(shadow_force.effect, 272);
    assert_eq!(shadow_force.power, 120);
    assert_eq!(shadow_force.type_, 7, "Ghost");
    assert_eq!(shadow_force.pp, 5);
    assert!(data.move_data(467).is_some());
    assert!(data.move_data(468).is_some(), "the four trailing rows");
    assert!(data.move_data(data::MOVE_ROWS as u16).is_none());
}

#[test]
fn items_pin_the_item_data_layout() {
    let Some(data) = game_data() else {
        return;
    };
    let master_ball = data.item(1).expect("Master Ball");
    assert_eq!(master_ball.price, 0, "not for sale");
    assert_eq!(master_ball.natural_gift_type, NO_NATURAL_GIFT);
    assert_eq!(master_ball.field_pocket, pocket::BALLS);
    assert_eq!(master_ball.battle_pocket, 1);
    assert_eq!(
        (
            master_ball.field_use_func,
            master_ball.battle_use_func,
            master_ball.party_use
        ),
        (0, 1, 0)
    );

    let poke_ball = data.item(4).expect("Poké Ball");
    assert_eq!(poke_ball.price, 200);
    assert_eq!(poke_ball.field_pocket, pocket::BALLS);

    let potion = data.item(17).expect("Potion");
    assert_eq!(potion.price, 300);
    assert_eq!(potion.hold_effect_param, 20);
    assert_eq!(potion.fling_power, 30);
    assert_eq!(potion.field_pocket, pocket::MEDICINE);
    assert_eq!(potion.battle_pocket, 4);
    assert_eq!(
        (
            potion.field_use_func,
            potion.battle_use_func,
            potion.party_use
        ),
        (1, 2, 1)
    );
    // The raw `ItemPartyParam` blob, pinned whole: mostly zero, with the
    // heal amount (20 HP) at byte 13. Decoding it is the bag/party
    // code's job (Phase 6).
    assert_eq!(
        potion.party_param,
        [0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 20, 0, 0, 0, 0, 0, 0]
    );

    let moon_stone = data.item(81).expect("Moon Stone");
    assert_eq!(moon_stone.price, 2100);
    assert_eq!(moon_stone.field_use_func, 20, "the evolve-a-mon function");

    let qualot = data.item(149).expect("Qualot Berry");
    assert_eq!(qualot.price, 20);
    assert_eq!(qualot.fling_power, 10);
    assert_eq!(qualot.natural_gift_power, 70);
    assert_eq!(qualot.natural_gift_type, 3, "Poison");
    assert_eq!(qualot.field_pocket, pocket::BERRIES);

    // The last row is a key item: never tossable, never selectable.
    let last = data.item(513).expect("row 513");
    assert!(last.prevent_toss);
    assert!(!last.selectable);
    assert_eq!(last.field_pocket, pocket::KEY_ITEMS);
    assert!(data.item(data::ITEM_ROWS as u16).is_none());
}

#[test]
fn learnsets_pin_the_wotbl_encoding() {
    let Some(data) = game_data() else {
        return;
    };
    // The empty species slot learns nothing.
    assert!(data.learnset(0).expect("row 0").entries().is_empty());

    // Bulbasaur's level-up set, as the retail table orders it.
    let expected = [
        (1, 33),   // Tackle
        (3, 45),   // Growl
        (7, 73),   // Leech Seed
        (9, 22),   // Vine Whip
        (13, 77),  // Poison Powder
        (13, 79),  // Sleep Powder
        (15, 36),  // Take Down
        (19, 75),  // Razor Leaf
        (21, 230), // Sweet Scent
        (25, 74),  // Growth
        (27, 38),  // Double-Edge
        (31, 388), // Worry Seed
        (33, 235), // Synthesis
        (37, 402), // Seed Bomb
    ];
    let bulba = data.learnset(1).expect("Bulbasaur");
    let pinned: Vec<(u8, u16)> = bulba
        .entries()
        .iter()
        .map(|e| (e.level, e.move_id))
        .collect();
    assert_eq!(pinned, expected);

    // And spot-check the same family one evolution in (Ivysaur adds
    // the level-1 starters and Razor Leaf moves earlier).
    let ivy = data.learnset(2).expect("Ivysaur");
    assert_eq!(ivy.entries().len(), 16);
    assert_eq!(
        &ivy.entries()[..3],
        &[
            LearnMove {
                level: 1,
                move_id: 33
            },
            LearnMove {
                level: 1,
                move_id: 45
            },
            LearnMove {
                level: 1,
                move_id: 73
            },
        ]
    );
    assert!(data.learnset(508).is_none());
}

#[test]
fn evolutions_pin_the_evo_layout() {
    let Some(data) = game_data() else {
        return;
    };
    // Bulbasaur evolves once, by level.
    let bulba = data.evolutions(1).expect("Bulbasaur");
    assert_eq!(bulba.entries().len(), 1);
    assert_eq!(bulba.entries()[0].method, 4, "EVO_LEVEL");
    assert_eq!(bulba.entries()[0].param, 16);
    assert_eq!(bulba.entries()[0].target, 2, "Ivysaur");

    // Eevee fills all seven slots — two location methods (Leafeon,
    // Glaceon), three stones, and two friendship evolutions — with no
    // EVO_NONE terminator left inside the table.
    let eevee = data.evolutions(133).expect("Eevee");
    let entries = eevee.entries();
    assert_eq!(entries.len(), EVOLUTION_SLOTS);
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.method, e.param, e.target))
            .collect::<Vec<_>>(),
        vec![
            (25, 0, 470), // Leafeon, near the Moss Rock
            (26, 0, 471), // Glaceon, near the Ice Rock
            (7, 83, 135), // Thunder Stone → Jolteon
            (7, 84, 134), // Water Stone → Vaporeon
            (7, 82, 136), // Fire Stone → Flareon
            (2, 0, 196),  // friendship, day → Espeon
            (3, 0, 197),  // friendship, night → Umbreon
        ]
    );
    // Most species evolve never: the empty slot and Arceus both do.
    assert!(data.evolutions(0).expect("row 0").entries().is_empty());
    assert!(data.evolutions(493).expect("Arceus").entries().is_empty());
    assert!(data.evolutions(508).is_none());
}

#[test]
fn egg_species_pins_the_pms_map_and_special_cases() {
    let Some(data) = game_data() else {
        return;
    };
    // The pms map: evolved species hatch as their base form.
    assert_eq!(data.egg_species(2), Some(1), "Ivysaur eggs are Bulbasaur");
    assert_eq!(data.egg_species(493), Some(493), "Arceus breeds true");
    // Eevee's table entry is itself (no baby form).
    assert_eq!(data.egg_species(133), Some(133));
    // The nine incense species hatch as themselves, table be damned —
    // pret's GetEggSpecies switch (Marill's table entry is Azurill,
    // 298, and the switch still answers Marill).
    for species in [113u16, 122, 143, 183, 185, 202, 226, 315, 358] {
        assert_eq!(data.egg_species(species), Some(species));
    }
    assert_eq!(data.egg_species(508), None);
}
