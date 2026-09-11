# The live field system

`apricorn_core::field::system::FieldSystem` is pret's `FieldSystem`
(`src/field_system.c`) for the part of the overworld that exists so
far: the player walking a loaded map with collision, the camera
following, the player billboard animated from the real map-object
frames, the new-game fade-in, and the warp tasks between maps: bedroom
(64) to house 1F (63) by the stairs, house 1F to New Bark Town (60) by
the front door, and free walking around the town. `Game`'s terminal
`Field` state runs it (`docs/game-flow.md`); the desktop and
`apricorn-run` drive `Game`, so the arrows walk the player with no new
keys. The player stands on the maps' BDHC ground heights. NPCs,
scripts, menus, encounters and the following Pokemon are later
workstreams; the seams they fill are named below.

## Architecture

| Piece | pret | Where |
|---|---|---|
| `FieldSystem`: scene, avatar, sprite, location, camera, fade, task | `FieldSystem`, `FieldSystem_Main` | `field/system.rs` |
| `FieldScene`: cells, props, terrain, events, camera preset (static, `Arc`) | `MapLoadManager`, `MapMatrix`, map headers | `field.rs` |
| `PlayerAvatar` + `MapObject`: the move control and the step machine | `PlayerAvatar_MoveControl`, `sub_0205F12C` | `field/avatar.rs`, `field/map_object.rs` |
| `FieldInput`: one tick's pad digest | `FieldInput_Update` | `field/input.rs` |
| `height`: the BDHC plate solver behind the objects' elevation | `ov01_021FAE50`, `sub_02054654` | `field/height.rs` |
| `PlayerSprite`: the hero's NSBTX frames and animation frame | `ov01_021F772C` and the sprite animation tables | `field/system.rs` |
| `FieldFade`: the 6-step master-brightness fade | `BeginNormalPaletteFade`, `sub_02010B14` | `field/system.rs` |
| `Transition`: a running warp task | `sub_02055DBC`, the exit/enter routine pairs | `field/system.rs` |
| `FieldFrame` / `ObjectView`: the per-tick view the rasterizer draws | the camera push and the billboard submits | `frame.rs`, `apricorn-gfx/src/field` |

Everything is deterministic and integer: ticks are pure functions of
the tick count and the `Input`; the ROM's tables are read through the
`AssetStore` once at construction (the sprite strip, the behaviour
flags) and at warps (the destination map).

## The frame model

`EngineFrame.field` used to hold a static `Arc<FieldScene>`; it now
holds a `FieldFrame`: the scene `Arc` (shared; a map load is the
expensive part and happens only at warps), the camera preset in force,
the camera target (the player's position vector, `Camera_SetFixedTarget`,
before the preset's look-at offset), and the `ObjectView`s, one
billboard each: the decoded frame strip (`Arc<Texture>`), the texel
rectangle of the frame shown, the anchor in fx32 world units (the
object's feet), the quad's size in world units and a `mirrored` flag for
the classes that store one side view. Frames compare with `==` and
replay without the system that produced them.

`FieldFrame::static_scene` is the shim the Phase 4 renderer goldens
(`bedroom_hg`, `field_hg`, `raster`) draw through: the header's preset
at the player's tile with the scene's south-facing player image as the
one object, what the bedroom landing showed. It places that object as
the live system places a standing player — the tile centre at the
cells' BDHC ground height for the camera target, plus `SPRITE_OFFSET`
for the billboard — so the two paths pin the same anchor; the
`field_hg` bedroom goldens were re-pinned when the shim took the
offset (the hair box moved 5 px down onto the live frame's and the
oracle's). `apricorn_gfx::field::scene_view` turns any `FieldFrame` into
the rasterizer's `SceneView`; `preset_from_core` maps the ROM preset row
(`perspectiveType` 0 is perspective, anything else orthographic) onto
the gfx camera. The player's hero NSBTX (`a/0/8/1` member 69/70)
carries separate east and west textures, so the player never mirrors;
the flag is there for the NPC classes.

### The hero frame layout (measured)

The hero member holds 32 textures `hero.1` to `hero.32`, four per
animation: north 1-4, south 5-8, west 9-12, east 13-16, then the
running set 17-32 in the same order. The class's eight animations are
16 frames each and `FRAMES_PER_TEXTURE` is 4, so texture =
`(frame >> 12) / 4`. The walk cycle the oracle shows (frames 4906-4941
of the walk case: textures 9, 10, 10, 10 ... alternating legs, the END
tick's callback advancing one more frame, the standing snap to a
multiple of 8) is pinned by `walk_cycle_alternates_legs_as_the_oracle_shows`.

### The sprite's anchor

The billboard's anchor is the object's position vector (its y the
ground height, see Heights) plus `SPRITE_OFFSET` (6.5 world units
toward the camera on z): against the oracle's bedroom frame the
player's image lands five pixels lower than a quad anchored at the
tile centre's projection, and the offset reproduces that for the field
cameras; with the ground height under it the New Bark frames match the
oracle's too (the outline's rows 62-98 at (695, 397), frame 7372).
Retail composes the position from two per-object offsets and six units
of height (`ov01_021F93AC`, `ov01_021FA3E8`); the exact composition is
still to be ported (see Deferred).

## Per-frame order vs pret

`NitroMain`'s loop iteration (`src/main.c:95-130`) spans two VBlanks in
retail, so one `FieldSystem::tick` is one retail game tick. Through
`FieldSystem_Main` (`src/field_system.c:180`):

1. `FieldSystem_Control` (`:203`), when movement is allowed (no field
   task running): `PlayerAvatar_UpdateMovement`, `FieldInput_Update`,
   `FieldInput_Process` (`src/field/field_control.c:193`), which is the
   step handler's transition check, the held-direction transition
   check, the interact and menu seams, then `PlayerAvatar_MoveControl`
   against the scene's terrain.
2. `FieldSystem_RunTaskFrame` (`src/task.c:49`): the field task stack,
   one state per tick, the new-game fade-in (`sub_020553C0`,
   `asm/unk_02055244.s:213`) or a map transition (`sub_02055DBC`).
3. The field overlay's frame (`FieldMap_Main` then `ov01_021E6220`):
   the camera at the player's position vector and the geometry and
   billboards submitted, the `FieldFrame` this tick returns, showing
   the objects where the previous tick's step left them.
4. The SysTask queue: the map object's movement step (`sub_0205FD30`:
   a stale height re-fetched, the held movement's step, whose walk
   refreshes the height from the plates under the new position,
   `sub_02061070`) then its sprite update (`ov01_021F772C`).
5. After the VBlank, `HandleFadeUpdateFrame`'s brightness step, the
   register the returned frame carries.

So the frame a tick returns shows the previous step's positions and
this tick's brightness, the retail display latency, kept so the
oracle's screenshot sequence lines up tick for tick.

Trajectories the tests pin (`crates/apricorn-core/tests/field_system_hg.rs`):
a held direction while standing turns first (three ticks, family 40),
then walks 0x2000 per tick for eight ticks (`MOVEMENT_STEP_*`); a
direction change while still walking turns and steps in one command
(`sub_0205D40C`); a blocked step walks on the spot for seventeen ticks
(family 28); a tap turns in place without moving; B does nothing
without running shoes (`PlayerSaveData::default()` for the new game).

## Collision

`SceneTerrain` implements `Collision` over the scene's
`TerrainAttributes` window: bit 15 of the attribute word is impassable
(`sub_020548C0`), a tile outside the resident cells is `ATTR_NONE`, and
surfable water comes from the ROM's `sMetatileBehaviorFlags`
(`src/metatile_behavior.c:7`) read out of the ARM9 image at
`0x020FCA74` (237 bytes, located by pattern): `BehaviorFlags`. New Bark
Town's pond is behaviour 21 (`WATER_SEA`) with bit 15 clear, blocked
only by that rule. Ledges (`JUMP_*` behaviours) are detected but the
hop is not runnable yet; the elevation rule (`sub_02054954`: a BDHC
height step of `5 << 14` or more blocks) is not applied yet (see
Deferred), though the solver it needs now exists.

## Heights

`field/height.rs` ports the BDHC plate solver, `ov01_021FAE50`
(`asm/overlay_01_021FAD1C.s:207`) behind `sub_02054654`
(`asm/unk_02054648.s:37`), the matrix-mode entry of the
`fieldSystem->unk60` table `sub_02054940` dispatches to. A cell's BDHC
block (`land::Bdhc`, six counted arrays: points `{x, z}`, slopes
`{nx, ny, nz}`, heights `h`, plates `{point1, point2, slope, height}`,
strips `{z, count, first}`, the access list of plate indices) describes
the ground as plates: the rectangle two corner points span, on the
plane `nx·x + ny·y + nz·z + h = 0`. For a position the solver runs the
retail binary search over the strips' z boundaries (ported step for
step, boundary answers included), tests the plates the strip's run of
the access list names for containment, evaluates each hit as
`y = -((nx·x + 0x800 >> 12) + (nz·z + 0x800 >> 12) + h) / ny` with
`FX_Div`, and with several hits keeps the one nearest the current y
(mode 0, the map object's), the highest (1) or the lowest (2), with
the asm's sentinels and strictness. Positions are relative to the
cell's centre (`(cell * 32 + 16) << 16` subtracted from the world
position); `scene_height` finds the resident `LandCell` of the tile
and answers `None` where no plate covers it or the cell is not
resident, which is the lookup's "not found".

`Collision::height_at` carries it to the movement code (`None` by
default, so the terrains of the movement unit tests keep their objects
at their creation elevation). `MapObject::update_height_from` is
`sub_02061070`: `IGNORE_HEIGHTS` only clears `HEIGHT_STALE`; otherwise
a found height becomes the position vector's y and, in 8-unit steps,
the tile y (the previous one latched), clearing the flag; a miss sets
it for the next tick's re-fetch. The step machine's walk step calls
the source-less `update_height` (it does not carry the terrain), and
`MapObject::tick` resolves the stale flag it leaves against the terrain
right after the held movement — the same outcome as retail's in-step
call, since nothing between reads the height. The player's object is
created with `HEIGHT_STALE` (`sub_0205EC90`) and, the field overlay
being up, fetched at once (`sub_0205EFB4` → `sub_0205EF8C`,
`src/map_object.c:217,805`), so the first frame already targets the
ground.

Measured on the pinned ROM (`the_player_stands_at_the_bdhc_ground_height`):
the bedroom's plate is at 0; New Bark Town's land surface is 16 units
(`16 << 12`) under every tile of the walk, so the player, its tile y
(2), the camera target and the billboard sit on it — the town frame
moved down by the ground height's projection (the mailbox's top row
50 → 62, the oracle's) and the whole body shows (outline rows 64-98
against the oracle's 62-98) where before only the cap cleared the
ground; 1F's floor plane is faintly tilted (140/4096 units at the
stairs, 0.03) and the tile behind its stairs carries a raised plate
(24724, 6.04 units). The stairs arrival places the player on that
plate: `sub_02056AEC` looks the shifted position's height up
(`sub_02054940`) before `sub_0205C810`, and the arrival walk's per-step
refresh brings him down onto the floor, as retail does.

Not ported: the dynamic terrain heights `sub_02054654` consults after
the plates (`fieldSystem->dynamicTerrainHeightManager`,
`ov01_021FB42C`; scripts register them for bridges — kind 2 of the
lookup, gated by the object's `UNK29`), and the elevation collision
rule above.

### The land section order

Fixing the town exposed a data-layer bug: HGSS land members with an
`extraSize` put the extra section **before** the attribute grid. Read
from `0x14 + extraSize`, New Bark's member 0 has its four door tiles
(`0x8069`) exactly on the map's four warp events; read from `0x14`
the grid is 44 words off and the pond had no water where the geometry
shows it. `field/land.rs` and `tests/field_hg.rs` carry the evidence.
`land.rs` belongs to the map-data workstream: the fix (`+24` lines,
parse `extra` before `attributes`) needs reconciling with that branch
at the merge rather than a silent revert — the test pins the
door/warp coincidence so a revert cannot pass unnoticed.

## Warps

`FieldInput_Process` starts a transition in two places:

* **On a step** (`FieldSystem_CheckTransition`, `field_control.c:706`):
  the tile just landed on is `WARP_ENTRANCE_NORTH` / `WARP_NORTH`, so
  the entrance transition facing north. Escalators, warp panels and
  ladders are not ported.
* **Standing with a direction held** (`FieldSystem_CheckMapTransition`,
  `:504`): the tile ahead impassable with a warp and behaviour `DOOR`
  gives the door transition (a house door faced from outside); the
  standing tile's `WARP_ENTRANCE_*` / `WARP_*` / `WARP_STAIRS_*` in its
  own direction gives the entrance or stairs transition. The bedroom's
  stairs at (3, 4) are `WARP_STAIRS_WEST` (0x5F): west starts it.

`FieldSystem_MapConnection` (`:909`) turns the warp event into the
destination `Location` (`header`, `anchor` as the target's warp id; the
target map's warp entry supplies the tile at load, `sub_02052F94`) and
records the entrance. Dynamic-warp anchors (`0x100`) need the save's
dynamic warp, not carried yet.

The task schedule (`Transition`). The stairs column is measured on
the oracle's bedroom-to-1F stairs (fade-out at VBlank 5132, fade-in at
5220, two VBlanks per tick); the door column shares its exit walk and
is read from the same routine shapes; the entrance column (the door
out of 1F) is **not measured** — the oracle's walk stalls in Mom's
scene before it — its fade-out tick comes from `sub_02056004`'s state
order and its fade-to-fade interval is assumed equal to the stairs'.

| tick | stairs / door | entrance |
|---|---|---|
| 0 | trigger (`FieldEvent::Warp`); movement stops | same |
| 4-19 | the exit walk, `WalkSlower*` one tile in the facing direction | none |
| 21 | fade-out begins: six 1-frame steps, brightness -2, -5, -7, -10, -13, -16 | fade-out at tick 3 (assumed) |
| +7 | the destination loads; the player is placed (stairs: one tile into the wall behind the target stairs, at that tile's ground height, facing back) | same |
| +44 | fade-in begins with the arrival walk (`WalkSlower*`) | same (assumed) |
| +17 | the walk's end seen at +16, the fade's finish at +17: the task unwinds (`Transition::end_tick`) | same |
| +18 | movement allowed | same |

A destination that fails to load (a map or warp member missing from
the store — no retail counterpart, where it would be a broken ROM)
does not unwind the game: the transition is abandoned at the load
tick, the current map fades back in with the player where the exit
left him, and `FieldSystem::last_error` reports the `MapLoadError`
(`a_warp_whose_map_fails_to_load_is_abandoned_on_the_current_map`).
`FieldSystem::warp(destination, kind)` starts a transition from outside
the input path — the seam for scripted warps (`sub_02055CD8` from a
script's warp command); it runs the same schedule.

The fade arithmetic is `sub_02010B14`'s (`asm/unk_0201010C.s:1486`):
`current = start << 7`, `delta = ((end - start) << 7) / steps`, the
register written as `current / 128`, so an OUT fade to black is a
gradual six-step darkening, as the oracle shows, unlike
`app::fade::BrightnessFade`'s IN-only derivation. The new-game entry
(`CallTask_FadeFromBlack`) is the same 6 x 1 IN fade; its poll reads
true the tick after the flag clears and movement is allowed from the
ninth tick.

## Tests and review images

* `crates/apricorn-core/tests/field_system_hg.rs`: the fade-in, the
  pinned trajectories, walls, the turn-in-place rule, the stairs warp
  with its timing (the wall plate's height on arrival), the door into
  New Bark Town, the camera target, the behaviour-flags table, the
  pond, the BDHC ground heights of the three maps and the abandoned
  warp. `field/height.rs`'s unit tests pin the solver's arithmetic,
  strip search and modes on synthetic blocks.
* `crates/apricorn-gfx/tests/field_system_hg.rs`: SHA-1 goldens of the
  bedroom mid-step, house 1F on arrival and New Bark Town on arrival
  (`APRICORN_RENDER_OUT` writes `field-system-*.png`); the 1F and New
  Bark hashes were re-pinned with the heights (the constants' doc
  carries the before/after landmark rows).
* `scripts/engine-walk.apin` continues `scripts/engine-new-game.apin`:
  bedroom, stairs, 1F, the front door, New Bark Town, the pond's rim,
  west along the path, south past the lab. Milestone frames:

  ```
  $CARGO_TARGET_DIR/debug/apricorn-run --rom hg_usa.nds --input scripts/engine-walk.apin \
    --png 3010,3040,3066,3071,3090,3095,3130,3140,3153,3200,3222,3270,3290,3320,3372,3460,3514,3600,3659 \
    --out out/engine-shots/walk
  ```

  3010 bedroom faded in; 3040 walking west; 3066 at the stairs; 3071
  the warp trigger; 3090 the exit walk done; 3095 fading out; 3130
  black; 3140 1F fading in (the player on the wall tile's raised
  plate, walking out); 3153 arrival on the stairs; 3200 down the
  hall; 3222 the door's fade-out; 3270 New Bark fading in; 3290 off
  the door, standing on the town's ground (the oracle's frame 7372
  framing); 3320 walking south; 3372 the pond's rim; 3460 west along
  the path; 3514 the west end; 3600 south past the lab; 3659 the tree
  line, the canopies over the body as in retail.

## Deferred

* **NPCs and map objects** (`FieldEvent::Interact` is reported, not
  consumed): the object events, Mom on 1F (the oracle's walk stalls in
  her script), the town's people; the `mirrored` flag in `ObjectView`
  is for their one-sided classes.
* **Scripts and menus** (`FieldEvent::Menu`): the location-name popup
  the oracle shows on arrival in New Bark Town, the start menu, the
  touch screen (engine B stays black; its frame structure is intact).
* **Encounters**: `EncounterSteps` counts; `BehaviorFlags::encounter`
  is read from the table; nothing rolls yet.
* **Heights, the rest**: the dynamic terrain heights (bridges, by
  script), the elevation collision rule (`sub_02054954`), the exact
  sprite-anchor composition (`ov01_021F93AC`) now that the position
  vector carries the ground height.
* **Prop fidelity**: `a/1/4/8` member 28 (`machine_l02`, the windmill
  blade layer) decodes as a 765 x 740-unit plane at y = 160, the thin
  white streaks across the town frames; the oracle draws short crosses
  atop the poles. A model-decode item for the data layer.
* **Following Pokemon**, ladders, escalators, warp panels, dynamic
  warps, running shoes and surfing states.
