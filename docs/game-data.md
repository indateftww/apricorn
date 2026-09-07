# Game data — personal, moves, items (the species/move/item tables)

Reference for `apricorn-core::data`, established in Phase 4 by scanning
the retail US dump (`hg_usa.nds`) and cross-checking against
pret/pokeheartgold's headers and build manifests — `include/
pokemon_types_def.h` (`BASE_STATS`, `Evolution`, `MAX_EVOS_PER_POKE`),
`include/move.h` (`MoveTbl`), `include/item.h` + `files/itemtool/
itemdata/item_data.txt` (the 34-byte item row), `files/poketool/personal/
growtbl.txt` and `evo.json.txt`, and `src/pokemon.c` (the loaders and
`GetTMHMCompatBySpeciesAndForm`/`GetEggSpecies`). **The retail image is
the oracle**: every invariant below was verified across every member of
every table (zero violations) before it became a parse-time check, so
`GameData::load` refuses a drifted or foreign dump instead of parsing it
to plausible garbage.

Nothing here is hand-typed: `GameData::load(&NdsRom)` reads the tables
straight out of the ROM at runtime, the same way the original does.

## Where the tables live

pret's `NarcId` enum (`include/filesystem_files_def.h`) does not name
paths; it *indexes* `sNarcFileList[]`. Resolving the ids the game's
loaders use (`src/pokemon.c`) gives the NitroFS paths:

| Table | pret id | NitroFS path | Members | Row size |
|-------|---------|--------------|---------|----------|
| personal (base stats) | 2 | `a/0/0/2` | 508 | 44 B fixed |
| growtbl (exp curves) | 3 | `a/0/0/3` | 8 | 404 B fixed (101 × u32) |
| waza (moves) | 11 | `a/0/1/1` | 471 | 16 B fixed |
| item_data | 17 | `a/0/1/7` | 514 | 34 B fixed |
| wotbl (learnsets) | 33 | `a/0/3/3` | 508 | 4–44 B variable |
| evo (evolutions) | 34 | `a/0/3/4` | 508 | 44 B fixed |
| pms (egg species) | — | `poketool/personal/pms.narc` | 1 file | 1016 B (508 × u16) |

`pms.narc` is not a NARC at all — just a raw file whose name lies; pret
reads it with plain `FS` file reads (`ReadFromPersonalPmsNarc`).

### Id spaces

* **Species rows (508):** slot 0 empty; national ids 1–493 (Arceus last);
  `SPECIES_EGG` = 494; alternate-form slots 495–507 (Deoxys ATK/DEF/SPD
  496–498, Wormadam Sandy/Trash 499–500, Giratina Origin 501, Shaymin
  Sky 502, Rotom Heat/Wash/Frost/Fan/Mow 503–507). Species-indexed tables
  all have 508 members.
* **Move rows (471):** `NUM_MOVES` = 467 (`SHADOW_FORCE`), plus 4 unused
  trailing rows the archive still carries.
* **Item rows (514):** ids past the last real item are zeroed rows.
* **Growth rates (8):** one curve per id — 0 medium-fast, 1 erratic,
  2 fluctuating, 3 medium-slow, 4 fast, 5 slow, 6–7 unused (and in fact
  copies of the medium-fast curve).

## personal — `BASE_STATS`, 44 bytes

| Offset | Field |
|--------|-------|
| +0x00 | 6 × u8: hp, atk, def, speed, spatk, spdef |
| +0x06 | 2 × u8: types (`TYPE_*` 0–17; `[t, t]` when single-typed) |
| +0x08 | u8 catch rate |
| +0x09 | u8 exp yield |
| +0x0A | EV yields, 2 bits each: hp, atk, def, speed |
| +0x0B | EV yields: spatk, spdef (top 4 bits always zero) |
| +0x0C | u16 wild held item 1, u16 item 2 |
| +0x10 | u8 gender ratio (0 genderless, 255 male; else ♀ = (code−1)/254) |
| +0x11 | u8 egg cycles, +0x12 friendship, +0x13 growth rate |
| +0x14 | 2 × u8 egg groups, +0x16 2 × u8 abilities |
| +0x18 | u8 Great Marsh flee rate (Sinnoh leftover; zero everywhere) |
| +0x19 | u8 color:7 \| flip:1 |
| +0x1A | 2 B padding — always zero |
| +0x1C | 4 × u32: 128 TM/HM compatibility bits |

TM/HM bit math matches `GetTMHMCompatBySpeciesAndForm`: bit
`tmhm % 32` of little-endian word `tmhm / 32`; TM01 is bit 0, HM01
(Cut) bit 92, through HM08 (Rock Climb) bit 99. The top 28 bits are
unused. Bulbasaur's retail words are `[0x84350720, 0x02101E08,
0x92662420, 0x2]` — TM22 yes, TM26 no, HM01 yes, HM08 no.

## growtbl — experience curves

Member id = growth-rate id. Each member is 101 little-endian u32s,
`lv000`…`lv100` (per `files/poketool/personal/growtbl.txt`; the
manifest's `rate:` column is skipped into the member *index*, not the
data). Level caps at 100: rate 0 → 1,000,000; 1 → 600,000; 2 →
1,640,000; 3 → 1,059,860; 4 → 800,000; 5 → 1,250,000; unused 6 and 7
repeat rate 0's curve.

## waza — `MoveTbl`, 16 bytes

| Offset | Field |
|--------|-------|
| +0x00 | u16 effect (`MoveAttr` id) |
| +0x02 | u8 category — 0 physical, 1 special, 2 status |
| +0x03 | u8 power, +0x04 type, +0x05 accuracy %, +0x06 PP |
| +0x07 | u8 effect chance % |
| +0x08 | u16 range (`RANGE_*` target selector) |
| +0x0A | s8 priority (−7…+7) |
| +0x0B | u8 unk, +0x0C u8 unk, +0x0D u8 contest type, +0x0E u16 unk |
| +0x0F | *(ends at 16; the C struct adds no padding)* |

The category semantics were pinned from the retail rows (Karate Chop 0,
Thunderbolt 1, Roar 2). Priority is a *signed* byte — Roar carries −6,
Quick Attack +1.

## item_data — the 34-byte item row

Built by `csv2bin --pad 0xFF` (`files/itemtool/itemdata/item_data.txt`);
the 0xFF padding lands *between* members, never inside them. The C
`ItemData` struct is 36 bytes only because of trailing struct padding
the archive does not store.

| Offset | Field |
|--------|-------|
| +0x00 | u16 price |
| +0x02 | u8 hold effect, +0x03 hold-effect param |
| +0x04 | u8 pluck effect, +0x05 fling effect, +0x06 fling power |
| +0x07 | u8 Natural Gift power |
| +0x08 | u16 bitfield — naturalGiftType:5, prevent_toss:1, selectable:1, fieldPocket:4, battlePocket:5 |
| +0x0A | u8 field use func, +0x0B battle use func, +0x0C party use func |
| +0x0D | 1 B padding — always zero |
| +0x0E | 20 B `ItemPartyParam` union (kept raw until the bag code consumes it) |

`naturalGiftType` 31 is the "no Natural Gift" sentinel written into the
data itself (31 is past the last real type id, 17). Field pocket ids:
0 Items, 1 Medicine, 2 Balls, 3 TMs/HMs, 4 Berries, 5 Mail,
6 Battle items, 7 Key items.

## wotbl — level-up learnsets

Each member is u16 entries `(level << 9) | move` until a `0xFFFF`
terminator, then zero padding to the archive's 4-byte member alignment
(member sizes run 4–44 bytes). Every retail member carries exactly one
terminator with an all-zero tail; the parser requires both.

## evo — `EVOLUTION_FILE`, 44 bytes

Seven slots of `struct Evolution { u16 method; u16 param; u16 target; }`
plus a u16 pad word (always zero) — 7 × 6 + 2 = 44. `EVO_NONE` (0)
terminates the active list; most species have an all-`EVO_NONE` row.
Eevee is the one species whose seven slots are all live: methods 25/26
(the location evolutions, param 0), 7 (`EVO_STONE`, param = item id),
2/3 (`EVO_FRIENDSHIP_DAY`/`_NIGHT`). Bulbasaur: one slot,
`{EVO_LEVEL=4, 16, Ivysaur}`.

## pms — the egg-species map

508 little-endian u16s: `pms[species]` is the species an egg of
`species` hatches as (Ivysaur 2 → Bulbasaur 1). pret's `GetEggSpecies`
special-cases nine species to return *themselves* — the incense
families (Chansey 113, Mr. Mime 122, Snorlax 143, Marill 183, Sudowoodo
185, Wobbuffet 202, Mantine 226, Roselia 315, Chimecho 358) — because
the caller's held-item check decides whether the baby (e.g. Azurill,
the table's entry for Marill) applies. `GameData::egg_species` ports
the switch exactly.

## Validation and tests

Invariants enforced at parse time, each verified across the whole retail
image first (probe results: 0 violations in 508 + 8 + 471 + 514 + 508 +
508 members):

* every member exactly its row size — short *and* long are corruption;
* personal: EV-yield pad nibble 0, bytes 0x1A–0x1B zero;
* item: byte 0x0D zero (the 0xFF inter-member pad never enters a row);
* wotbl: exactly one 0xFFFF terminator, all-zero tail after it;
* evo: trailing pad word zero;
* waza: category byte in 0–2;
* table member counts equal the retail census (508/8/471/514/508/508).

Row-level decoding is unit-tested against synthetic fixtures in the
module (`src/data/*.rs`); the retail facts are pinned by
`crates/apricorn-core/tests/data_hg.rs`, which loads every table from
`hg_usa.nds` and asserts the values above (Bulbasaur's full row,
the growth caps, the classic moves, the item rows, both learnsets,
Bulbasaur/Eevee evolutions, and the egg-species special cases).

`GameData` keeps the rows private behind typed accessors
(`base_stats`, `growth_table`, `move_data`, `item`, `learnset`,
`evolutions`, `egg_species`) so cross-table ids can only be consumed
with the id spaces made explicit. The item `partyParam` blob is
deliberately raw: its per-field meaning belongs to the use-function
dispatcher and gets decoded with the bag/party code (Phase 6).