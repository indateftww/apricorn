# Field data — map headers, matrices, land, area, events, scripts, ov01

Reference for `apricorn-core::field` (Phase 5's map-engine data layer),
established by reading the retail US dump (`hg_usa.nds`) alongside
pret/pokeheartgold: `include/map_header.h` and `src/data/map_headers.h`
(the header table), `src/map_matrix.c` (matrices),
`src/field/map_prop_manager.c` and `include/terrain_attributes.h` (land
data), `src/map_events.c` + `include/map_events_internal.h` (events),
`src/script_manager.c` + `include/constants/init_script_types.h` (script
headers), and the overlay-1 asm (`asm/overlay_01_021F4704.s`,
`asm/overlay_01_021EABA8.s`, `asm/overlay_01_sprite_data.s`) for what
pret has not decompiled. **The ROM is the spec**: every layout below was
checked across every member of its archive before it became a parse-time
check, and nothing is hand-typed — `FieldScene::load` reads the tables
out of the ROM at run time, the way the game does. No table, name or
model is committed; the tests hold structural counts, pret's constants
and hashes only.

The chain this layer mirrors is the field system's map entry
(`FieldSystem_CreateMap`, `src/field/fieldmap.c:658`): read the map
header → load its matrix (`MapMatrix_Load`) → the area data
(`AreaDataManager_Alloc`) with its texture sets and prop models → the
cells around the player (`MapLoadManager_InitialLoad`: land model,
props, attributes, BDHC) → events (`Field_InitMapEvents`) → the script
header (`MapScriptHeader_ReadFromNarc`) → the camera
(`FieldCamera_Create`). Nothing here ticks: movement, the script VM and
the per-step cell streaming are the other Phase 5 workstreams.

## Where the tables live

| Data | pret name | Where | Members | Loader |
|------|-----------|-------|---------|--------|
| map headers | `sMapHeaders` | ARM9 `.rodata` @ `0x020F6BE0` | 540 × 24 B | `map_header::MapHeaders` |
| map names | `mapname.bin` | `fielddata/maptable/mapname.bin` | 540 × 16 B | `MapHeaders::name` |
| matrices | `map_matrix` | `a/0/4/1` | 288 | `matrix::MapMatrix` |
| land data | `land_data` | `a/0/6/5` | 676 | `land::LandData` |
| area data | `area_data` | `a/0/4/2` | 106 × 8 B | `area::AreaData` |
| building-set id lists | — | `a/0/4/3` | 104 | `AreaData::building_ids` |
| map texture sets (NSBTX) | — | `a/0/4/4` | 106 | `FieldScene::load` |
| building texture sets (NSBTX) | — | `a/0/7/0` | 104 | `FieldScene::load` |
| outdoor prop models | `bm_field` | `a/0/4/0` | 340 | `AreaData::prop_model_narc` |
| indoor prop models | `bm_room` | `a/1/4/8` | 222 | `AreaData::prop_model_narc` |
| prop animation lists | — | `a/1/0/7` (340), `a/1/0/8` (222) | | `AreaData::prop_animation_narc` (not read yet) |
| map events | `zone_event` | `a/0/3/2` | 491 | `events::MapEvents` |
| scripts and script headers | `scr_seq` | `a/0/1/2` | 965 | `script_header::InitScripts` |
| move-model textures | — | `a/0/8/1` | 863 | `FieldScene::load` (the player) |
| the field overlay | `OVY_1` | overlay 1 at `0x021E5900`, 0x24260 B after BLZ | | `ov01::Ov01` |

`AssetStore` grew the helpers this needs: `narc(path)` (parse an
archive once, then `narc_member`), `nitrofs_file(path)`, `overlay(id)`
(overlay-table entry plus the BLZ-expanded image), `member_count`,
`arm9_image` / `arm9_base`.

## Coordinates

* A tile is 16 world units; fx32 carries 12 fractional bits. The player
  stands at the centre of tile `(x, z)`: world
  `((16x + 8) << 12, y, (16z + 8) << 12)`. Tile coordinates are
  **matrix-wide** — events, warps and `TerrainAttributes` all use them.
* A cell is 32 × 32 tiles (512 units). Its origin, `ov01_021F5FB8`
  (`asm/overlay_01_021F4704.s:3320`), is the cell's *centre*:
  `x = (cx << 21) + (1 << 20)` (= `(512·cx + 256) << 12`),
  `y = altitude << 15` (8 units per altitude step),
  `z = (cz << 21) + (1 << 20)`. Land models are authored around that
  centre; prop translations are relative to the origin's x and z and
  absolute in y.
* The bedroom (map 64, matrix 72, 1 × 1) puts the new game on tile
  `(6, 6)`. New Bark Town (map 60) is cell `(21, 12)` of matrix 0, so its
  tiles are `672..703 × 384..415`; its front-door warp sits at
  `(695, 396)`.

## Map headers — `sMapHeaders`

pret ships the table as C (`src/data/map_headers.h`, 540 records) but no
symbol file. The table was found by encoding map 64's record
(`MAP_NEW_BARK_PLAYER_HOUSE_2F`) from pret's values — GCC bitfields, LSB
first, little-endian — and scanning the decompressed ARM9 image for the
24 bytes: exactly one hit, at `0x020F71E0` = slot 64 of a table starting
at `0x020F6BE0`; neighbours 60/63/65 decode to pret's records. The
harness pins it as `sMapHeaders` (`crates/apricorn-harness/pins/arm9.tsv`,
data pin, 12960 bytes) so a foreign dump fails loudly.

| Offset | Field |
|--------|-------|
| +0x00 | u8 `wildEncounterBank` (`a/0/3/7` member; 255 = `ENCDATA_NA`) |
| +0x01 | u8 `areaDataBank` (`a/0/4/2` member) |
| +0x02 | u16: `moveModelBank:4`, `worldMapX:6`, `worldMapY:6` |
| +0x04 | u16 `matrixId` (`a/0/4/1`) |
| +0x06 | u16 `scriptsBank` (`a/0/1/2`) |
| +0x08 | u16 `scriptHeaderBank` (`a/0/1/2`) |
| +0x0A | u16 `msgBank` (`a/0/2/7`) |
| +0x0C | u16 `dayMusicId`, +0x0E u16 `nightMusicId` |
| +0x10 | u16 `eventsBank` (`a/0/3/2`) |
| +0x12 | u16: `mapsec:8`, `areaIcon:4`, `momCallIntroParam:4` |
| +0x14 | u32: `regionNo:1`, `weather:7`, `mapType:4`, `cameraType:6`, `followMode:2`, `battleBg:5`, then one bit each `bikeAllowed`, `runningAllowed_Unused`, `escapeRopeAllowed`, `flyAllowed`, `outgoingCalls`, `incomingCalls`, `radioSignal` |

`MapHeader::parse`/`encode` round-trip every retail record; `mapType`
and `followMode` are typed enums (`MapType`, `FollowMode`) and reject
values pret has no name for. `mapsec` runs up to 234
(`MAPSEC_CLIFF_EDGE_GATE`). Every record's banks index real members of
their archives and every `cameraType` is below 17 (checked in
`field_hg.rs`). Map names come from `mapname.bin`, 16 NUL-padded ASCII
bytes each (`T20R0202` for the bedroom, `T20` for New Bark Town).

## Matrices — `a/0/4/1`

`MapMatrix_MapMatrixData_Load` (`src/map_matrix.c:15`):

```text
u8 width, height, hasHeadersSection, hasAltitudesSection, nameLength
u8 name[nameLength]                       (≤ 16, MAP_MATRIX_MAX_NAME_LENGTH)
u16 headers[width*height]     if hasHeadersSection   — else every cell = the loading map
u8  altitudes[width*height]   if hasAltitudesSection — else zeros
u16 landDataIds[width*height]             — 0xFFFF = no land (NO_LAND)
```

Index `z * width + x`; at most 799 cells (`MAP_MATRIX_MAX_SIZE`). Matrix
0 is the 47 × 17 overworld (799 cells, 492 with land, 204 distinct land
members, ≈6.6 MB of land source); matrix 72 (`m_hh0102_`) is the
bedroom's 1 × 1 with land 217. `MapMatrix::load(store, id, map_id)`
takes the loading map so a member without a headers section fills like
`MIi_CpuClear16(map_no)`; `cell_origin(x, z)` is `ov01_021F5FB8`.

## Land data — `a/0/6/5`

One member per cell. Header 0x14 bytes, then the sections contiguously:

```text
u32 attrSize (0x800)  u32 propSize (0x30·n)  u32 modelSize  u32 bdhcSize
u16 magic 0x1234      u16 extraSize
attrs   0x800 B   u16 per tile, index (x % 32) + (z % 32) * 32
extra   extraSize u16 words (all 0x8006 on the members checked); undocumented in pret
props   0x30 each MapPropArcData: s32 buildModel; VecFx32 translation, rotation, scale; u8 pad[8]
model   modelSize BMD0 (the cell's ground and walls)
bdhc    bdhcSize  "BDHC" height data
```

The extra section's position (between the attributes and the props) was
measured on the 230 members that have one: the `BMD0` magic lands at
`0x14 + attr + extra + props`, and the sizes then account for every
byte of every member. Across the archive: 2859 prop placements, at most
30 per cell (`MAP_PROP_MAX` is 32), and no placement has a non-zero
rotation or a non-unit scale — the loader still carries both fields.

Attribute words: bits 0–7 the metatile behavior
(`include/constants/metatile_behavior.h`), bit 15 impassable. Land 217
(the bedroom) is 936 × `0x0000`, 83 × `0x8000` (walls), 4 × `0x8086` and
1 × `0x005F`.

BDHC: `"BDHC"`, six `u16` counts, six arrays whose element sizes — 8, 12,
4, 8, 8, 2 bytes (points, slopes, heights, plates, strips, access list)
— are the only assignment under which the count arithmetic reproduces
`bdhcSize` on all 676 members. It is parsed into raw sections
(`land::Bdhc`); the height solver is a later step, so every loaded
vertex keeps the model's own y and the player's y is 0.

## Area data — `a/0/4/2`

`AreaDataManager_Alloc` / `_Load` (`asm/overlay_01_021FB878.s:29`, `:276`):
8-byte records `u16 buildingSet; u16 mapTexture; u16 unknown; u8
outdoor; u8 lightSelector`. `buildingSet` selects the `a/0/4/3` id list
(`u16 count; u16 ids[count]`) and the `a/0/7/0` building NSBTX;
`mapTexture` the `a/0/4/4` NSBTX; a non-zero byte 6 picks the outdoor
prop archive `a/0/4/0` (bm_field) with animation lists `a/1/0/7`, zero
the indoor `a/1/4/8` (bm_room) with `a/1/0/8`. Record 25 (the bedroom)
is `{1, 25, 0xFFFF, 0, 0}` — indoor, building set 1 (58 ids).

A placement whose `buildModel` is not in the area's building set is
drawn as model 0 (`MapPropManager_LoadFromNARC`, via `ov01_02204154`:
no resource file loaded for that index).

## Terrain attributes

`terrain::TerrainAttributes` is the counterpart of pret's
`TerrainAttributes` cache (`src/terrain_attributes.c:31`, up to 16
blocks) and the lookup behind `GetMetatileBehavior`: a window of cells,
each holding its 32 × 32 words; `attr(x, z)` / `behavior(x, z)` /
`impassable(x, z)` take matrix-wide tile coordinates and answer
`TILE_BEHAVIOR_NONE` (0xFF) — and impassable — for a tile whose cell is
not resident.

## Map events — `a/0/3/2`

`MapEvents_ComputeRamHeader` (`src/map_events.c:154`): four
count-prefixed arrays, in order —

```text
u32 numBg;    BgEvent[numBg]       20 B: u16 scriptId, type; s32 x, z, y; u16 dir, pad
u32 numObj;   ObjectEvent[numObj]  32 B: u16 id, spriteId, movement, type, eventFlag, scriptId;
                                         s16 facingDirection; u16 param[3]; s16 xRange, yRange;
                                         u16 x, z; s32 y
u32 numWarp;  WarpEvent[numWarp]   12 B: u16 x, z, header (destination map), anchor; u32 y
u32 numCoord; CoordEvent[numCoord] 16 B: u16 scriptId; s16 x, z; u16 w, h, y, val, var
```

Every retail member is exactly its arrays (no slack) and fits the
game's 0x800-byte buffer. Member 61 (the bedroom) is 2 signs, 3 objects,
1 warp (the stairs at (3, 4) to map 63, anchor 1), 0 triggers; member 57
(New Bark Town) is 5 / 10 / 5 / 4.

## Script headers — `a/0/1/2`

`GetMapLoadScriptId` / `GetMapSceneScriptId` (`src/script_manager.c:618`):
5-byte records `{u8 type; u32 payload}` terminated by type 0, read into
a 0x100-byte buffer. Types (`init_script_types.h`): 1 `ON_FRAME_TABLE`,
2 `ON_TRANSITION`, 3 `ON_RESUME`, 4 `ON_LOAD`; for 2–4 the script id is
the payload's low 16 bits (0xFFFF = none). For type 1 the payload is an
offset from the byte after the record to 6-byte `{u16 varA, varB,
scriptId}` entries terminated by `varA == 0`; each frame the first entry
whose variables compare equal runs. Header 619 (the bedroom) is empty;
615 (New Bark Town) has a two-entry frame table plus `ON_TRANSITION` 7
and `ON_RESUME` 10.

## Overlay 1 tables

Overlay 1 (`OVY_1`, id 1 in the ROM overlay table, BLZ-compressed, loads
at `0x021E5900`) is read and expanded at run time; nothing is copied.

* **Camera presets** `ov01_02206478` (`asm/overlay_01_021EABA8.s:465`):
  17 × 0x24 `{fx32 distance; u16 angle[3], pad; u16 perspectiveType; u16
  fovyAngle; fx32 near, far; VecFx32 lookAtOffset}`, indexed by
  `MapHeader::cameraType` in `FieldCamera_Create`. Preset 4 (interiors:
  orthographic, pitch 0xDC82) and preset 0 (the overworld: perspective,
  pitch 0xDD62) are checked exactly in `field_hg.rs`.
* **Sprite → move-model table** `ov01_022074A8`
  (`asm/overlay_01_sprite_data.s:436`): 6-byte `{u16 spriteId, mmodelId,
  packed}` rows to a 0xFFFF terminator (901 rows); `SPRITE_HERO` 0 →
  `MMODEL_HERO` 69, `SPRITE_HEROINE` 97 → 70. The player's south-facing
  image is the first texture of that `a/0/8/1` member.
* **Land base transform** `ov01_02206BD8` (scale) / `ov01_02206BE4`
  (rotation, `asm/overlay_01_021F4704.s:4507`): what
  `MapLoadManager_RenderLoadedMap` (`:2424`) hands `GF3dRender_DrawModel`
  with the cell origin. Unit and identity in retail — read, not
  assumed (`Ov01::land_base`).

## `FieldScene::load(store, map_id, x, z, gender)`

1. Header, name, matrix, area data, the area's two NSBTX sets and its
   building ids.
2. The **cell window**: the player's cell ± `LOAD_RADIUS` (1) — 3 × 3,
   clamped to the matrix. Retail keeps the player's cell plus the
   neighbours toward the player's quadrant (2 × 2) and reloads as the
   player crosses cell boundaries; the 3 × 3 window is that set's
   superset for every quadrant. `load_all` loads every cell (for
   sub-matrices and tooling; see costs).
3. Per cell: `LandData::parse`, the model parsed once into model space
   (`model::parse`, with node SRTs and POSSCALE — bm_field props such as
   model 27 have pivot-rotated nodes) and placed under the land base
   transform at the cell origin; attributes into `TerrainAttributes`;
   each prop placed as `ov01_021F3A3C` (`src/field/map_prop_manager.c:186`)
   draws it — translation = archive translation + origin x/z (never y),
   the function's local identity rotation, archive scale. Prop models
   parse once per distinct id. Every matrix is fx32 with 64-bit products
   and an arithmetic shift by 12 (`model::fx_mul`), the SDK's rounding.
4. Camera preset, the player texture through the sprite table, events,
   init scripts.

`FieldScene::bedroom(store, gender)` is `load(64, 6, 6, gender)`; the
pre-Phase-5 fields (`map_id`, `meshes` — every cell's and prop's meshes
concatenated in draw order — `player`, `position`) are kept so the
Phase 4 rasterizer and `tests/bedroom_hg.rs` are unchanged.

### Costs (release build, this machine)

| Scene | Cells | Props | Triangles (land + props) | Time |
|-------|-------|-------|---------------------------|------|
| bedroom (map 64) | 1 | 8 | 260 + 354 | 3.4 ms |
| New Bark Town, 3 × 3 window | 9 | 18 (11 models) | 4982 + 1342 | 4.6 ms |
| overworld `load_all` | 492 | — | ≈270 k land | fails |

`scene.meshes` duplicates the cells' and props' triangles, so a scene
costs about twice its triangle count until the rasterizer reads
`cells`/`props` directly. `load_all` on the overworld is not just heavy:
it uses the loading map's area data for every cell, and land 89's model
binds a texture that is not in New Bark's set (`a/0/4/4` #2) — retail
never loads a cell with a foreign area, so a whole-matrix view needs
per-cell area data, which this layer does not attempt.

## Not ported yet

* Cell streaming on movement (`MapLoadManager`'s per-step reload) — the
  window is fixed at load.
* The BDHC height solver; the extra section's meaning.
* Prop animations (`a/1/0/7`, `a/1/0/8`) and the shape-wise draw
  (`DrawModelShapewise` with `bm_*_matshp.dat`).
* The `overrideRotation` path: `MapPropManager_LoadOne` +
  `sub_02020D2C` (`MTX_RotX33 · RotY33 · RotZ33` from
  `FX_SinCosTable_`), reached by scripts, not by map load.
* Safari Zone props (`MapPropManager_LoadFromSafariZone` stamps `0x8023`
  over their tiles).
* Map objects other than the player (the sprite table is read; only the
  player's texture is decoded), `lightSelector` lighting, weather, the
  `mm_list` move-model bank.

## Tests

`crates/apricorn-core/tests/field_hg.rs` (ROM-gated, silent skip
without `hg_usa.nds`): map 64 exact against pret and its neighbours
63/65 (same flags, own banks); every header's banks in range and
round-tripping; matrix 72 and matrix 0; land 217's sizes
(2048/384/6576/144/0), prop count and histogram, the extra section's
position, whole-archive prop statistics; area 25 and 2 with their
archive selection; events 61 (and its warp) and 57; script headers 615,
618, 619; camera presets 0 and 4 exact, sprite rows 0 and 97, the land
base transform; the bedroom scene for both genders (equal to
`load(64, 6, 6)`), New Bark Town's 3 × 3 window with bm_field props and
a pivot-rotated model; and `load_all` on two sub-matrices. All 540
headers, 288 matrices, 676 land members and 491 event members parse.
Unit tests in each module cover the parsers on synthetic members. The
harness's `pins_hg.rs` verifies the `sMapHeaders` pin.
