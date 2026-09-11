# Menus: the field start menu

`apricorn_core::app::start_menu::StartMenu` is the first Phase 5 menu
slice: the X-button menu of the field, ported from pret
`src/start_menu.c` (the field task) and overlay 27 (`asm/overlay_27.s`,
the sub-screen icon grid — asm only, so the ROM is the spec there). It
is a self-contained scene component: it draws into a `LogicalFrame`
over a caller-supplied base frame, reads the field through the
`StartMenuHost` trait, and reports what happened through
`StartMenuEvent`. Nothing in it touches the field system, so the
orchestrator wires it into the field scene once that exists.

## What HeartGold's menu is

It is not a top-screen list. Pressing X keeps the field on the top LCD,
adds a 56-pixel bar along its bottom (MAIN BG3, `a/0/1/4` members
12/13/15, palette bank 14) with a bobbing arrow sprite at (100, 144)
pointing at the touch screen, and turns the touch LCD into a two-column
grid of icon sprites — POKéDEX, POKéMON, BAG, POKéGEAR down the left,
TRAINER CARD, SAVE, OPTIONS down the right — each with a centered label
in a 9×2 window under it, a `(X) MENU` header at the top-left, and the
selected icon on its highlight palette bank.

`start_menu.c` owns the *what*: `StartMenu_Init` (`:216`) picks the
inhibit mask (safari, bug contest, pal park, the battle tower partner
room, else the normal flag-gated mask, `:288-331`),
`StartMenu_BuildActionLists` (`:483`) turns it into the insertion and
display lists, `Task_StartMenu_DrawCursor` (`:455`) loads the bar and
builds the cursor, `StartMenu_HandleKeyInput` (`:591`) runs
`selectionToAction[fieldSystem->unkD3]` on A, and the B/X edge closes
(`:576-579`). Overlay 27 owns the *where*: the sub BG (`ov27_0225AC00`),
the icon sprites at `ov27_0225D038`, the label windows at
`ov27_0225D074`, the d-pad walk over the `ov27_0225D0B4` graph, the
touch hitboxes `ov27_0225CF68`, and the compact selection index it
writes back into `fieldSystem->unkD3` (`ov27_0225C170`) — the seam the
two halves share. `StartMenuEvent::Selected(action)` names the pick;
the launched apps are later slices.

## Entry visibility

Every gate is a save flag read through the host
(`FieldSystem_ShouldDrawStartMenuIcon`, `:535`):

| Icon         | Gate                                        |
|--------------|---------------------------------------------|
| POKéDEX      | `FLAG_GOT_POKEDEX` (0x6B)                   |
| POKéMON      | `FLAG_GOT_STARTER` (0x6A)                   |
| BAG          | `FLAG_GOT_BAG` (0x11B)                      |
| POKéGEAR     | `FLAG_GOT_POKEGEAR` (0x9C)                  |
| TRAINER CARD | `FLAG_GOT_TRAINER_CARD` (0x11C)             |
| SAVE         | `FLAG_GOT_SAVE_BUTTON` (0x11D)              |
| OPTIONS      | `FLAG_GOT_OPTIONS_BUTTON` (0x11E)           |

The C's action list and the grid gate differently, and both are kept:
the normal inhibit mask only removes the dex, starter, bag, and gear
entries, so a fresh save's list is TRAINER CARD, SAVE, OPTIONS, EXIT
(plus `START_MENU_ACTION_9/10` forced into display slots 7 and 8) —
but the grid draws only icons whose flag is set, and a pick on a
list entry whose icon gate is shut is refused
(`FieldSystem_StartMenuActionIsAvailable`). A brand-new game therefore
opens a bare panel with no icon, label, header, or cursor slot. Mom's
bedroom scene (`scr_seq_0845_T20R0201.s:29-41`) sets BAG, TRAINER
CARD, SAVE, and OPTIONS in turn; the starter, Pokégear, and Pokédex
come later. The party count is not a gate (POKéMON gates on the
starter flag), so `party_count()` is on the trait for the later party
and bag slices, not read here.

Layouts: `ov27_0225BD50` picks one of seven slot rows
(`ov27_0225CFC8`) — normal, safari (RETIRE leads), bug contest, pal
park, union room (`bottomScreenType == 3`, LOG in the gear slot),
colosseum, and the battle tower partner room. `StartMenu::open` is the
X-button open; `open_union_room` and `open_colosseum` are
`sub_0203BCDC`/`sub_0203BD20`. Each takes the store, the host, the
base frame, and the *opening frame's input* (see below).

`actions()` is `selectionToAction[..numActiveButtons]`, the display
list the compact index addresses, and it has holes: 9 and 10 go to
display slots 7 and 8 while the count keeps advancing, so a fresh
save's slots 4–6 and a full menu's slot 9 are never written. They are
`None` in the accessor; the C's cleared struct holds 0 (POKéDEX) there,
which a pick would read as the dex — a read overlay 27's compact index
never makes.

## Input

- The opening frame's input seeds the edge state. `gSystem.newKeys`
  is global: the X that opened the menu was the field's edge
  (`field_control.c:303-306`), and `Task_StartMenu`'s `INIT` pass
  only sets `HANDLE_INPUT` and breaks (`start_menu.c:338-352`), so
  the first pass that reads keys is a frame later and a still-held X,
  B, or A is not new. Pass the input the field's opener read to
  `open` and the same holds here: a button held across the open
  closes or picks nothing until it is released and pressed again, and
  a stylus already down is not a new touch.
- UP/DOWN/LEFT/RIGHT (`ov27_0225B404`): the first new direction, three
  candidates per slot and direction from `ov27_0225D0B4`, the first lit
  one wins. Columns wrap; a row whose candidates are all unlit stays
  put (LEFT from SAVE with POKéMON unlit stays on SAVE — the table's
  own fallback). Keys are `gSystem.newKeys` edges: holding is one press.
- A: `StartMenu_HandleKeyInput`. EXIT (`STARTMENUTASKFUNC_CANCEL`)
  closes; app launches run the six-step `FieldMap_FadeScreen`
  brightness fade and then report `Selected` with the bar cleared and
  both screens black — the A frame writes step 1, the next five write
  steps 2–6, the seventh clears the fade manager's flag, and the
  eighth is the first `WaitFade` pass that sees it finished, so the
  launch lands eight frames after the press; SAVE reports `Selected`
  at once (the touch save app takes the sub screen); RETIRE clears the
  bar and reports at once; the union-room LOG reports and then closes
  two ticks later.
- B or X: `START_MENU_STATE_CLOSE` on the next pass — the base frame
  returns and the event is `Closed`. START does not close (it is not
  in the C's mask).
- Touch (`ov27_0225B4D8`): the header strip queues
  `lastTouchMenuInput = 1` (close); a lit icon moves the cursor and
  queues `2 + compact index`; the C task reads the queue on its next
  pass (`StartMenu_HandleTouchInput`). An unlit slot swallows the
  press. A held stylus is one press.

`selection_index()` is `fieldSystem->unkD3` — carry it back through
`StartMenuHost::last_menu_selection` so the next open restores the
cursor, as the field does.

## Frame layout

- MAIN: the base frame plus BG3 (256×256 4bpp, char block 1, priority
  0) carrying the bar's screen, palette bank 14, and the cursor sprite
  (members 61–64, one 44-frame bob). Closing or launching drops the
  screen and the sprite (`sub_0203C38C`); the char and palette loads
  stay, as on hardware.
- SUB: a fresh engine — BG0 (priority 2) with the panel (members 7/8/9,
  a full 256-color palette load), BG1 (priority 0) carrying the label
  windows (palette 4, base tiles `0xF6 + 0x12·slot`, `MAKE_TEXT_COLOR
  (14, 2, 0)`, font 0, centered in 72 px) and the header window
  (10×2 at (9, 0), tile `0xE2`, font 4, color (15, 1, 0)); the icon
  sprites (cells 16, anims 17, chars per `ov27_0225CF94`, the female
  BAG member 27, palette 14 with the highlight in bank 1) and the `(X)`
  glyph from the button sheet (members 68/69/70); the open menu's blend
  register (alpha, BG0|BG1|BD, EVA 6 / EVB 9). The base frame's sub
  backdrop carries over.

The asset table is `apricorn_core::assets::start_menu`. Message bank
196 holds the labels (`msg_0196_00000`–`00008`, `00014`) and the
header (`00012`); the trainer card's label is the player's name
(`{STRVAR_1 3, 0, 0}`).

## Tests and goldens

- `crates/apricorn-core/tests/start_menu_hg.rs` (ROM-gated): the
  fresh, post-Mom, and full entry sets; column wrap and the unlit
  fallback; A picks (the eight-frame fade-out launch, immediate SAVE);
  B/X close and START not; touch close and touch pick; X, B, A, and
  the stylus held across the open are not new presses.
- `crates/apricorn-gfx/tests/start_menu_hg.rs` (ROM-gated): SHA-1 of
  both screens for the full, post-Mom, and fresh menus over a black
  base frame; a cursor walk moving the highlight; close restoring the
  base. `APRICORN_RENDER_OUT=out/review` writes
  `start-menu-{full,after-mom,fresh}-open.png` and
  `start-menu-full-down.png`.
- Unit tests in the module pin the tables (`sStartMenuActions`, the
  masks, the layout rows, the hitboxes, the walk graph).

## Deferrals

Audio (`PlaySE`); the launched apps; the closed-state sub-screen UI
(registered items, the running shoes toggle, the A-button hint —
overlay 27's other look); the pop-in animation of newly unlocked icons
(`ov27_0225AAD4`); the safari / bug-contest / pal-park ball counters
(`ov27_0225C0E0`); the per-sprite OAM-mode override that dims
unselected icons through the blend unit (`ov27_0225B4AC`,
`overlay_27.s:2601-2640`, sets OAM mode 1 on every icon but the
selected one, and `ov27_0225A8E8(1)` programs the sub blend unit at
EVA 6 / EVB 9 — so on hardware every unselected icon is translucent
over the panel). The frame model's `Sprite` carries no per-sprite mode
and the rasterizer takes the mode from the cell's own OAM attributes
(`apricorn-gfx/src/sprites.rs:153`), so honoring the override is a
frame-model change (a `Sprite` field plus the rasterizer) outside this
slice; the register state is set and the icons render opaque, and the
goldens pin that look — an oracle comparison of an open-menu frame
will differ on every unselected icon until it lands. Also deferred:
the `MenuInputStateMgr` memory beyond one open, and
`fieldSystem->unk90`'s write-only `lastButtonSelected`.