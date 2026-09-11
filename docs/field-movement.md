# Field movement — the player and the map-object step machine

Reference for Phase 5's movement layer: `apricorn-core::field::map_object`
(the `LocalMapObject` state subset and the per-frame movement-command
machine), `field::avatar` (the player avatar's control path) and
`field::input` (the per-frame input digest). Source: pret
`pokeheartgold`, where the object step machine and most of the avatar
are still asm — `asm/unk_02062108.s`, `asm/unk_0205FD20.s`,
`asm/unk_0205CB48.s` — so the port is locked to the retail ARM9 image
with arm-runner (`crates/apricorn-harness/tests/movement_hg.rs`), the
same per-function differential method as `docs/rng.md`. The C that
exists (`src/map_object.c` accessors, `src/player_avatar.c`,
`src/field/field_control.c`) is the documentation.

Nothing here reads a clock or an RNG: every tick is a pure function of
the object's state and the frame's pad word, so a frame replays.

## Coordinates

`MapObject_SetPositionFromXYZAndDirection` (`src/map_object.c:1989`):

```text
world.x = tile.x * 16 * FX32_ONE + 8 * FX32_ONE     (tile centre)
world.z = tile.z * 16 * FX32_ONE + 8 * FX32_ONE
world.y = level  *  8 * FX32_ONE                    (one elevation = 8 units)
FX32_ONE = 4096;  one tile = 0x10000 world units
```

Directions are `DIR_NORTH 0, SOUTH 1, WEST 2, EAST 3`
(`include/constants/global_fieldmap.h:5`); north is `-z`, east is `+x`
(`_020FD49C` / `_020FD4AC`, `asm/unk_0205FD20.s:2737`). The new-game
player stands on map 64 at tile (6, 6) facing south, which is world
(0x68000, 0, 0x68000).

`VecFx32::from_tile` and `Direction::delta_x/z/reverse` are the port.

## The object

`MapObject` keeps the movement-visible subset of `LocalMapObject`
(`include/map_object.h:48`, 0x12C bytes) under pret's names, offsets in
the docs: `flags` (+0x00), `flags2`, `id`, `spriteId`, `movement`, the
five facings (+0x24…+0x34), `xRange`/`yRange`, `initial`/`previous`/
`current` X,Y,Z (+0x4C/+0x58/+0x64), `positionVector` (+0x70),
`unkA0` (anim group), `movementCmd` (+0xA4), `movementStep` (+0xA8),
`unkAC`/`unkAE` (current / previous tile behaviour), `unk128` (the
behaviour latched at a move's end) and the 16-byte work buffer `unkF8`
the step functions use through `sub_0205F3C0` / `sub_0205F3E4`.

The flag bits movement touches (`include/map_object.h:116`):

| bit | pret | meaning here |
|---|---|---|
| 0 | `ACTIVE` | |
| 1 | `SINGLE_MOVEMENT` | blocks `ready_for_movement` |
| 2 | `START_MOVEMENT` | raised by a move's `Step0`; consumed by the tick's effect hook |
| 3 | `END_MOVEMENT` | raised on the frame a linear move lands; consumed likewise |
| 4 | `UNK4` = `HELD_MOVEMENT` | a command is loaded (`MapObject_SetHeldMovement`) |
| 5 | `UNK5` = `HELD_MOVEMENT_DONE` | the loaded command finished |
| 7 | `UNK7` = `FACING_LOCKED` | `MapObject_SetFacingDirection` is a no-op |
| 11 | `UNK11` = `BEHAVIOR_STALE` | the standing-tile behaviour must be refreshed |
| 12 | `UNK12` = `HEIGHT_STALE` | the BDHC height must be re-fetched |
| 16 / 17 | `UNK16` / `UNK17` | start / end effect variants, consumed with 2 / 3 |
| 23 | `IGNORE_HEIGHTS` | skip the height lookup |

Two predicates drive everything (`asm/unk_02062108.s:25,:101`):

* `ready_for_movement` (`MapObject_AreBitsSetForMovementScriptInit`):
  `ACTIVE && !SINGLE_MOVEMENT && (!HELD_MOVEMENT || HELD_MOVEMENT_DONE)`.
* `is_movement_idle` (`MapObject_IsMovementPaused`, despite the name):
  `!HELD_MOVEMENT || HELD_MOVEMENT_DONE`.

## The commands

`gMovementCmdTable` (`asm/unk_data_020FCBD8.s:623`, pinned as a data
pin: 113 pointers) maps a command index to a `gMovementCmdSteps_NNN`
array of step functions. `MovementCmd` names all 113 plus
`MOVEMENT_STEP_END` (254) and `MOVEMENT_NONE` (255)
(`include/constants/movements.h:22`). The direction families come from
`_020FD198` (`asm/unk_data_020FCBD8.s:540`): 22 rows of four commands
in `DIR_*` order; `sub_0206234C(direction, family)` finds the row that
contains `family` and returns its entry for the direction,
`sub_02062390(cmd)` returns a command's column (its direction). Rows
0x5C/0x5D and 0x5E/0x5F are two-entry rows repeated.

Implemented families and their measured timing — every row below is
locked to the ROM tick by tick by `movement_hg.rs`:

| family | commands | `Step0` | speed / frame | frames busy | position | `END_MOVEMENT` |
|---|---|---|---|---|---|---|
| face | 000–003 | `sub_0206247C(dir)` | – | 0 (done within the tick) | – | no |
| walk slowest | 004–007 | `sub_020624CC(dir, 0x800, 32, anim 1)` | 0x800 | 32 | +1 tile | yes |
| walk slower | 008–011 | `sub_020624CC(dir, 0x1000, 16, 2)` | 0x1000 | 16 | +1 tile | yes |
| walk (`MOVEMENT_STEP_*`) | 012–015 | `sub_020624CC(dir, 0x2000, 8, 3)` | 0x2000 | 8 | +1 tile | yes |
| walk faster | 016–019 | `sub_020624CC(dir, 0x4000, 4, 4)` | 0x4000 | 4 | +1 tile | yes |
| walk fastest | 020–023 | `sub_020624CC(dir, 0x8000, 2, 5)` | 0x8000 | 2 | +1 tile | yes |
| on-spot slowest | 024–027 | `sub_020627B0(dir, 32, 1)` | – | 33 | none | no |
| on-spot slower (the bump) | 028–031 | `sub_020627B0(dir, 16, 2)` | – | 17 | none | no |
| on-spot | 032–035 | `sub_020627B0(dir, 8, 3)` | – | 9 | none | no |
| on-spot faster | 036–039 | `sub_020627B0(dir, 4, 4)` | – | 5 | none | no |
| on-spot fastest (the turn) | 040–043 | `sub_020627B0(dir, 2, 5)` | – | 3 | none | no |
| instant step | 084–087 | `sub_020624CC(dir, 0x10000, 1, 0)` | 0x10000 | 1 | +1 tile | yes |
| run | 088–091 | `sub_020624CC(dir, 0x4000, 4, 9)` | 0x4000 | 4 | +1 tile | yes |

Everything else (jumps `sub_02062958` 044–059, the flag setters
060–074, emotes 075/103, the `sub_02062FAC` animations, the timed
waits, the `sub_020632B0` sequences 105–112) is `Unimplemented`:
`run_step` returns the error, `run_held_movement` retires the command
as finished (so the object never wedges) and `TickReport::unimplemented`
names it. The player's ledge hop (family 056) is therefore *issued*
(`MoveOutcome::Jump`) but not yet run.

### The step functions

* `sub_020624CC` (`asm/unk_02062108.s:595`), the linear init: zero 12
  work bytes, store `{u16 anim; s16 frames; u32 direction; fx32 speed}`,
  `sub_02060F24` (previous := current, current += delta — **the tile
  coordinates advance at the first step**, before the sprite has
  moved), `MapObject_SetOrQueueFacing`, anim group, set
  `START_MOVEMENT`, step++. Returns 1, so `Step1` runs in the same
  tick.
* `MapObjectMovementCmd090_Step1` (`:629`), per frame: `sub_0206101C`
  adds `speed` along the axis, `sub_02061070` refreshes the height,
  `frames -= 1`; while `frames > 0` return 0 (yield). On the frame it
  reaches 0: set `END_MOVEMENT | HELD_MOVEMENT_DONE`, `sub_02060F78`
  (previous := current), the sprite callback, anim group 0, step++,
  return 1.
* `sub_020627B0` (`:1044`), the on-spot init: **stores `frames + 1`**,
  `MapObject_SetFacingDirection` (not the queue), anim group, previous
  := current, step++. `MapObjectMovementCmd040_Step1` (`:1069`) counts
  down like the walk but sets only `HELD_MOVEMENT_DONE` — no
  `END_MOVEMENT`, so an on-spot command never produces the end-of-move
  effects and, for the player, never reports a `PLAYER_MOVE_STATE`
  phase. That is why a turn costs three frames and a bump seventeen.
* `sub_0206247C` (`:543`), the face step: `SetFacingDirection`, anim
  0, previous := current, step++.
* `MapObjectMovementCmd098_Step2` (`:534`), the shared terminal step:
  set `HELD_MOVEMENT_DONE`, return 0. It never advances the step, so
  the runner re-executes it (harmlessly) every tick until a new command
  is loaded.

`sub_02062400` (`:471`) is the runner: while the held command is not
`MOVEMENT_NONE`, call `gMovementCmdTable[cmd][step]` and loop while it
returns 1.

## The tick

`sub_0205FD30` (`asm/unk_0205FD20.s:28`), the object's SysTask body,
ported as `MapObject::tick`:

1. the manager's pause bit (`sub_0205F5E8(obj, 2)` reads the
   *manager's* flags, not the object's — no manager exists here, so the
   tick always runs);
2. `sub_0205FE0C`: `HEIGHT_STALE` → `sub_02061070`;
3. `sub_0205FE24`: `BEHAVIOR_STALE` → `sub_02061108`; becoming valid
   raises `START_MOVEMENT`;
4. `sub_0205FE48`: `START_MOVEMENT` → `sub_0205FEDC` (behaviour refresh
   + start effects); clear bits 2 and 16;
5. `HELD_MOVEMENT` → `sub_02062400`; otherwise the autonomous-movement
   callback `sub_0205F430` (not ported: the player's movement type does
   nothing there, and no NPCs are ticked yet);
6. `sub_0205FE6C`: bit 16 → `sub_02060020`, else `START_MOVEMENT` →
   `sub_0205FF6C` (both refresh the behaviours); clear 2 and 16;
7. `sub_0205FEA4`: bit 17 → `sub_02060114`, else `END_MOVEMENT` →
   `sub_0206008C` (`unk128 := unkAE`, refresh); clear 3 and 17.

`sub_02061108` (`:2516`) latches the behaviours of the previous and
current tiles (`TILE_BEHAVIOR_NONE` for objects with `flags2` bit 2),
returning whether the current tile has one. The sprite/effect leaves
(`ov01_*`, the `sub_0206*` effect spawners) are presentation and
skipped.

`sub_02061070` (`:2445`) needs the BDHC height map, which is not loaded
yet: the port keeps the flag protocol (`IGNORE_HEIGHTS` clears
`HEIGHT_STALE`; otherwise the lookup counts as failed and sets it) and
leaves `y` alone. Tests set `IGNORE_HEIGHTS`.

## Movement lists

`EventObjectMovementMan` (`asm/unk_02062108.s:170`) applies a scripted
list of `{u16 cmd; u16 count}` entries to one object through the
`MovementScriptMachine` states `Init → WaitReady → SetCommand →
WaitCommand → LoopCheck → Done` (`:241`–`:352`): wait until
`ready_for_movement`, hold the command, wait until `is_movement_idle`,
repeat `count` times, advance, finish at `MOVEMENT_STEP_END`.
`MovementList::tick` is the port; it runs before the object's tick in
a frame, as the SysTask priorities order them. Holding `End`/`None`
is `MovementError::NotHoldable` (pret asserts `command < 113`).

## The player

Frame order (`src/main.c:95`, `src/field_system.c:207`,
`FieldSystem_Control`): `PlayerAvatar_UpdateMovement` →
`FieldInput_Update` → `FieldInput_Process` → `PlayerAvatar_MoveControl`,
then the SysTask queue ticks the objects. A command loaded by
`MoveControl` takes its first step in the same frame.
`PlayerAvatar::run_frame` keeps that order (minus `FieldInput_Process`,
which belongs to the field system: events, encounters and menus consume
the digest and may skip `move_control` for the frame while the object
still ticks).

### `PlayerAvatar_UpdateMovement` (`asm/unk_0205CB48.s:401`)

Derives `playerMoveState` from the avatar's `moveState` and the
object's held-movement flags as the previous tick left them:

* forced movement (ice; kind ≠ 0, ≠ 2) → `MOVING`;
* object not ready (a command is running): `moveState MOVING` and the
  command is not a bump → `START` if the previous phase was `NONE`/`END`,
  else `MOVING`; `moveState TURNING` → `MOVING`; a bump → `NONE`;
* object idle and `moveState ≠ NONE`: previous `NONE` → stays `NONE`,
  previous `END` → `NONE`, else `END`.

So a walk reports `NONE, START, MOVING×6, END` over its nine frames
(the END lands on the frame *after* the position reaches the tile
centre), and a turn reports `MOVING, MOVING, END`. Locked to the ROM
over the full (moveState × phase × flags × command × flag1) grid.

### `FieldInput_Update` (`src/field/field_control.c:121`)

Ported verbatim as `FieldInput::update`: the running-shoes lock forces
B into `heldKeys`; on `END`/`NONE` frames the buttons digest (Y /
touch-latch 9 → registered item 1, latch 10 → item 2, latch 11 →
touch menu, X or any latch → menu, A → interact, any direction →
sign + map transition, `standing`), otherwise the touch latch is
cleared; `movement` = `END` while the avatar was moving (not turning),
`endMovement` = `END`; `transitionDir` = the facing if that key is held;
`playerDir` from `sub_0205DD94`. The things it reads off the
`FieldSystem` (bag icon, registered-item usability, the touch menu's
button state) arrive as `FieldInputContext`.

`sub_0205DDD4` (`:2398`), the pad resolver: LEFT beats RIGHT, UP beats
DOWN within an axis; one axis held gives that axis; both held resolve
against what the previous `MoveControl` recorded in `unk28`/`unk2C`
and the object's `nextFacing` — the same diagonal as last frame keeps
`nextFacing`; a vertical that was already held makes the horizontal
the new axis; otherwise the vertical wins. Locked over all 16 pad
words × 3 × 3 remembered components × 4 next facings.

### `PlayerAvatar_MoveControl` (`:18`), walking state

1. direction := `sub_0205DDD4(held)`;
2. `sub_0205CBEC` (`:100`): may a command start? yes when the object is
   ready; during a *bump*, yes if the requested way has become free
   (or is water while surfing); otherwise the frame is `Busy`;
3. record the pad components (`sub_0205CC4C`), clear flag 6, apply
   transition flags, consult the forced-movement handler
   (`sub_0205D004`; handler 0 is a no-op without ice);
4. `sub_0205D40C` (`:1123`) → `sub_0205D450` (`:1169`), **the
   turn-versus-walk rule**, measured and locked over every (facing,
   direction, moveState):

   ```text
   no direction                                  → 0 stand   (moveState NONE)
   direction != facing && moveState != MOVING    → 2 turn    (moveState TURNING)
   otherwise                                     → 1 walk    (moveState MOVING)
   ```

   A direction change *while walking* therefore does not turn — the
   next step's `Step0` sets the facing and steps in one command. Once
   stopped (`moveState NONE` after a stand), a new direction costs a
   turn first.
5. `sub_0205D340` → `sub_0205D3A8` (`:1070`) dispatches on the rule
   (it calls `sub_0205D40C` again; the second call is idempotent):
   * stand `sub_0205D494`: hold `face(facing)` (family 0);
   * turn `sub_0205D610` (`:1380`): hold family 40, queue the facing,
     reset and flip `unkC`;
   * walk `sub_0205D4B4` (`:1221`): `bits = sub_0205DA34`; a ledge
     (bit 2) → family 56; anything else nonzero → family 28 (the bump),
     queue the facing, wall-hit SE unless bit 3 (door/warp); free →
     family 12, or family 88 (run) with running shoes and B held
     (`sub_0205DE88`: `heldKeys & 2`), `GAME_STAT_STEPS_WALKED`++,
     flag 6.
6. `sub_0205CC74`: flag 6 && moveState MOVING → clear flag 1 (the
   "no step taken yet" latch). Footstep sounds (`sub_0205CC94`) are
   presentation.

At a new game `hasRunningShoes` is `FALSE` (`PlayerSaveData_Init`), so
B does nothing until the shoes are obtained; `runningShoesLock` forces
B on in the input digest.

### Collision (`sub_0205DA34`, `:1921`)

The raw probe `sub_0205DAA8` (`:1983`) of the tile ahead:

* bit 0: outside the object's `xRange`/`yRange` box (`sub_02060D94`) —
  *ignored* by the aggregate (the player's ranges are −1 anyway);
* bit 1: `sub_020549F4` (`asm/unk_02054648.s:561`) — the BDHC
  elevation comparison (`sub_02054954`; a difference also sets bit 3),
  map gimmick collision, else `sub_020548C0`: **bit 15 of the tile's
  attribute word** (`:376`); or `sub_02060DEC` (`asm/unk_0205FD20.s:2103`):
  the target has no behaviour (`0xFF`, i.e. outside the map), the
  standing tile's behaviour forbids leaving that way (`_020FD4CC[dir]`)
  or the target's forbids entering (`_020FD4BC[dir]`) — the edge
  predicates `sub_0205B8F4/918/93C/960` (`src/metatile_behavior.c:438`);
* bit 2: another object stands on or is leaving the tile
  (`sub_02060BFC`).

The aggregate: raw bits 1 or 3 → `BLOCKED`, plus `DOOR` if
`sub_0205DBF4` (`:2146`; standing on a warp entrance — its switch falls
through, so north tests all four entrance kinds, south three, west two,
east one — or facing a door); raw bit 2 → `OBJECT`; `sub_0205DB68`
(`:2076`; the matching `JUMP_*` tile ahead) → `LEDGE`; `sub_0205DCA0`
(`:2231`) → `WATER` (`TILE_BEHAVIOR_115` unless flag 28, any surfable
water, or `TILE_BEHAVIOR_34` for the fishing sprites); the bicycle
probe `sub_0205DCFC` (`:2275`, `state == CYCLING` only) is not ported.

The port takes the decision exactly for what it can see: the
`Collision` trait (`attr(x, z) -> u16`: behaviour in the low byte, bit
15 impassable; `ATTR_NONE` = `0x00FF` outside the map) is what the
map-data layer's terrain attributes will implement; tests stub it.
**Deferred and documented:** BDHC elevation (both the cliff check and
the `y` update), map gimmicks, other objects, and the jump execution.

### Counters

`FieldSystem` (`include/field_system.h:207`): `encounterInhibitSteps`
counts move ends (`fieldInput->endMovement` — walks *and* turns;
`FieldSystem_CheckWildEncounter`, `src/field/field_control.c:471`) and
gates encounters at `> 3` (`encounter_check.c:216`);
`FieldSystem_UpdateTurnFrameCounter` (`:1397`) runs once that gate and
the tile's encounter rate pass, counting a facing opposite to
`lastFacingDirection` as a reverse turn and latching the facing; map
changes zero both counters (`src/field_warp_tasks.c:268`).
`EncounterSteps` holds the three with those rules.

## Worked example

Spawn (6, 6) facing south, DOWN held on a free floor:

```text
frame 0  MoveControl: facing == direction → walk; hold 013; tick: Step0
         (current = (6,7), START) + Step1 (z += 0x2000, frames 7)
frames 1–7  Busy; Step1 each tick; frame 7: frames 0 → END, DONE
frame 8  UpdateMovement: END; FieldInput: movement + endMovement;
         MoveControl: ready → walk again; tick: Step0 + Step1
```

Spawn facing south, LEFT held: frame 0 turns (family 42, 3 ticks:
frames 0–2, `Busy` on 1 and 2), frame 3 walks west. Release after the
first frame and frame 3 is `Idle` (face west). Into a wall: frame 0
bumps (family 29, 17 ticks), frames 1–16 `Busy`, frame 17 bumps again;
a free direction pressed meanwhile interrupts the bump at once, a
blocked one does not.

## Differential coverage

`crates/apricorn-harness/tests/movement_hg.rs` (needs `hg_usa.nds`; CI
skips) places a synthesized `LocalMapObject` at `0x0220_0000` and a
`PlayerAvatar` at `0x0220_1000` in scratch RAM and compares, after every
call, the flags, four facings, previous/current tiles, position vector,
anim group, command/step and 16-byte work buffer:

* `step_machine_matches_the_original_per_frame`: every ported command
  through `MapObject_SetHeldMovement` + `sub_02062400` for 36 ticks (so
  the dispatch through the ROM's own `gMovementCmdTable` is verified);
* `step_functions_match_when_called_directly`: `sub_020624CC` /
  `MapObjectMovementCmd090_Step1`, `sub_020627B0` /
  `MapObjectMovementCmd040_Step1`, `sub_0206247C` and
  `MapObjectMovementCmd098_Step2` against `run_step`, return values
  included;
* `coordinate_helpers_match_for_every_direction`: `sub_0206101C` (all
  four directions, eight speeds including a negative one),
  `sub_02060F24`, `sub_02060F78`;
* `held_movement_predicates_match`, `family_and_direction_lookups_match`
  (`sub_0206234C`, `sub_02062390`, `sub_0205DE64` over the whole range);
* `pad_direction_resolution_matches_for_all_inputs` (`sub_0205DDD4`),
  `turn_versus_walk_decision_matches` (`sub_0205D450`),
  `player_move_state_derivation_matches` (`PlayerAvatar_UpdateMovement`).

`PlayerAvatar_MoveControl` itself walks the `FieldSystem` (terrain
callbacks, the object manager) and is not called; its decision tree is
covered by the ROM-free `crates/apricorn-core/tests/movement.rs`.

The 26 pins (`pins/arm9.tsv`, verified by `pins_hg.rs`) name each
function by its address as pret does; every prologue was matched
against the listing before pinning.

The first run of the per-frame test exposed an arm-runner bug rather
than a port bug: Thumb `add pc, Rm` — the compiler's switch idiom in
`sub_0206101C` — added the operand to the already-advanced r15 (pc + 2)
instead of the read-ahead value (pc + 4), landing inside the jump-table
data. Fixed in `arm/exec.rs`; the `coordinate_helpers` test walks every
case of that switch.
