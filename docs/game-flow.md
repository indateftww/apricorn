# New game through the bedroom

`apricorn_core::app::game::Game` runs the main-overlay chain through the
intro, title, save check, main menu and Oak's speech. Oak launches
`app::naming::NamingScreen` as a nested overlay. While it runs, the parent
scene's printers and animation counters stop. Its last frame survives the
handoff, then Oak restores its graphics and asks for name confirmation.
The confirmed `PlayerIdentity` is written into the initialized save blocks
in `AfterOakSpeech`. `Bedroom` displays the room and chosen character.

The desktop already used `Game` before this landing. It now also maps a
left mouse click onto bottom-LCD stylus coordinates, including integer
scaling and letterbox offsets. Losing focus releases held input.

## Running

```text
cargo run -p apricorn-desktop -- --rom hg_usa.nds
```

Arrow keys move the cursor, A activates, B deletes a name character,
Enter/Start focuses OK, and Shift/Select cycles uppercase, lowercase and
symbols. Clicking keys enters characters too. Seven characters fills the
player-name buffer; after the cursor animation, focus moves to OK.
R searches the original Japanese conversion table and therefore does
nothing to the characters on the three US keyboard pages.

The desktop starts with a blank card and a fixed 2010-03-14 noon RTC.
`Game::new` also accepts a retail save blob; desktop save selection is not
implemented. New-game state is available through `Game::new_game_data()`. Its `snapshot()`
produces a checksummed card in memory; this flow does not write a disk save.

## Source and data

The implementation follows `refs/pokeheartgold/src/naming_screen.c`:
`NamingScreen_HandleInput`, `GetPlayerInput`, `MoveKeyboardCursor`,
`HandleCharacterInput`, `HandlePageSwitch`, `HandleTouchInput`, and the
overlay init/main/exit functions. Only player mode is exposed.

Keyboard data comes from the verified retail ROM's expanded ARM9:

| Table | Address |
|---|---|
| Home-row pointer array | `0x021104E4` |
| Character-row pointer array | `0x021104F8` |
| Uppercase first row (discovery anchor) | `0x02101E80` |

The loader follows pointers for all three localized pages, including
spaces and symbols. No keyboard strings or default player names are
copied into the repository. Graphics come from `a/0/3/1` (`namein.narc`),
fonts from `a/0/1/6`, prompt bank 249 and default-name bank 254 from
`a/0/2/7`.

An empty or all-ASCII-space entry selects a default via exactly one
`LCRandom() % 18` draw. Male names start at message 0 and female names at
18. A typed name consumes no RNG, and repeated result reads cannot draw
again.

## Rendering fixes

The naming assets exercised three renderer cases the earlier scenes did
not cover:

- OBJ palette placement must include each cell's OAM palette bank.
  Ignoring it made button faces black and washed out the avatar.
- Glyph palette index zero preserves existing window pixels. Treating it
  as a hole through the BG corrupted the keyboard's checkerboard fill.
- WIN0 hides keyboard BG0/BG1 in the top 64 pixels of the bottom screen.
  Without that screen-space mask, outgoing keyboard pages overwrote the
  name box after page changes.

Rectangle fills now preserve the keyboard's 16-by-19-pixel checkerboard.
The upper LCD's black area follows the reference scene: its backdrop is
explicitly black, and its BG0 draws only the prompt near the bottom.
Local review renders cover all three pages and a filled name after a
complete page cycle; the user confirmed the corrected rendering.

Two suspected text-box defects were checked against the retail ROM on
the oracle (2026-09-11) and turned out to be retail behavior, pixel for
pixel modulo melonDS's 5→8-bit color expansion:

- The "two stacked rounded boxes" at the right end of Oak's gender and
  name dialogs and of the naming prompt are the `{YESNO 0}` screen-focus
  icon (`RenderScreenFocusIndicatorTile`, `text.c:296`: frame 0 of
  `graphic_font` member 6 blitted at `(width-3)·8`), which the retail ROM
  draws at the same place (oracle frames 3576–3640, 3876, 4110). They are
  not the page-wait arrow. The arrow (`TextPrinter_DrawDownArrow`,
  `render_text.c:395`) is a light-grey triangle over the frame's `+10/+11`
  border columns, and the engine's three poses match the retail ROM's
  held paragraph wait (a scratch oracle case with the A pulses removed,
  frames 2124–2200) tile for tile, nine frames per pose in the
  `{0, 1, 2, 1}` cycle. `scripts/engine-new-game.apin` pulses A on
  alternate ticks, so every engine paragraph wait is drawn and cleared
  within one tick and the arrow never reaches its PNGs — a script
  artifact, not a renderer defect; the corpus retail script pulses with
  period 8.
- The 16-pixel dark band before the box's right border is the dialogue
  frame's own art: `sub_0200E6B4` (`render_window.s`) writes three border
  columns right of the interior (`+3/+4/+5`, `+9/+10/+11`, `+15/+16/+17`)
  and two left of it; the interior fill spans exactly `width` tiles. The
  retail frames 1959/2081/3580/3638/3876 show the same band.

`gfx/tests/raster.rs::dialogue_fill_border_arrow_and_focus_extents_follow_pret`
pins that geometry ROM-free; `gfx/tests/oak_hg.rs` pins the three arrow
poses' block hashes and their nine-frame timing plus the gender question's
top LCD, and `gfx/tests/naming_hg.rs` the prompt's top LCD, all pinned
after the retail comparison.

## Validation and remaining parity work

`crates/apricorn-core/tests/naming_hg.rs` checks layout loading, cursor
wrapping and duplicate-button skipping, touch boundaries and press edges,
page input gates, deletion, length limits, and gender-specific default
names with exact RNG consumption. `tests/apps_hg.rs` now drives the real
nested keyboard instead of injecting a name: naming starts at global frame
2300, “MATTS” is entered with stylus presses, Oak exits at 2998, and the
post-Oak re-seed/bedroom transition occurs at 2999 for that input script.
These indices are engine regression pins, **not measured retail timing**.

The renderer has ROM-independent regressions for OAM banks, zero-index
glyph blits, fill ordering/scrolling, and the hardware-window BG mask.
`crates/apricorn-gfx/tests/naming_hg.rs` verifies that page changes preserve
the name box. Set `APRICORN_RENDER_OUT` to an absolute local directory to
write the four review PNGs; they are not committed.

## New-game data

`save::new_game::NewGameData` ports `Save_InitDynamicRegion`'s 42 block
initializers and `overlay_36.c`'s two initialization passes. Defaults include
empty encrypted party/box Pokémon, ROM-loaded PC box names, mail templates,
RTC fields, options and all ancillary block sentinels. The starting position
is map 64, warp -1, tile (6,6), south; money is 3,000, the fishing record
is 56,150, and flag 960 is set.

The post-Oak pass writes the player name/gender, trainer ID and avatar,
friend-group state, Safari area selection, apricorn trees, ten Pokewalker
seeds and the friend's Marill mail. Initialization consumes two MT draws
for roamers; after Oak the reseeded MT supplies twelve draws and LC supplies
two for the temporary mail Pokémon. RTC years use the SDK's 0–99 field,
including `InitializeMainRNG`'s seed arithmetic, while the public pinned
clock accepts full calendar years.

`ConsoleProfile` makes RTC offset, MAC, birthday, authentication ID and
entropy explicit, frozen inputs. The desktop uses zeros. System properties
are copied after Oak, as in the original; offline DWC defaults include the
original ID construction and CRC. No online operation occurs.

`core/tests/new_game_hg.rs` covers both genders, draw counts, identity,
position and all 42 blocks through checksum/parse/write round trips. The
harness's test invokes all 42 original ARM init functions. Clock, console
entropy/authentication and the external block-CRC wrapper are controlled
boundaries. PC box text loading is outside this isolated ARM fixture;
money, position and flags are checked separately as overlay mutations.
This is a block-default differential, not a whole post-Oak or original
scheduler equivalence test. The interpreter fixes it exposed have focused
instruction regressions (multiply decoding, long multiply, register-offset
loads/stores and Thumb-to-ARM BLX).

## Bedroom landing

The static field loader follows map 64's matrix 72 to land member 217 in
`a/0/6/5`. Area 25 supplies map and prop texture IDs. Geometry comes from
the land's embedded NSBMD and eight furniture models in `a/1/4/8`;
textures come from `a/0/4/4` and `a/0/7/0`. Player images come from members
69/70 of `a/0/8/1`. Geometry and pixels remain local ROM assets.

The NSBMD subset handles the room's identity nodes, material bindings and
packed GX triangle/quad streams. The CPU renderer uses the type-4 field
camera parameters and a depth buffer. It is a static rendering subset:
lighting, hardware raster precision, field entry animation, movement,
scripts, collision and the lower-screen field UI still belong to the
field-engine work. The bottom LCD stays black. `Bedroom` deliberately
holds the room; it does not yet respond to movement keys. Continue also
remains a terminal placeholder.

`gfx/tests/bedroom_hg.rs` renders both genders and can emit review PNGs via
`APRICORN_RENDER_OUT`. Room textures exposed an older BTX bit-depth error:
PLTT4 is 2bpp, not 4bpp. The corrected parser validates complete texture
spans across every retail BTX archive; the former supposed truncated shadow
textures were an artifact of that error.

## Acceptance and remaining fidelity work

Naming now uses ROM sine-table palette glow, the seven-tick bar wiggle
and BACK/OK press animations, covered by core regressions. These tests
pin effect behavior, not the original task scheduler's frame alignment.

The user accepted the functional Phase 4 milestone after regression testing
on 2026-09-11 and authorized its commit. Oak's missing Poké Ball was a
double palette-bank offset: the ROM cell already selects bank 1. The fix
is covered by a replay that checks visible ball pixels before the flash.

Phase 8 retains original-ROM frame/state comparisons of Oak and naming
(nested overlay scheduling and fades), Oak's remaining sprite waits and
yes/no cursor, and a whole post-Oak differential. Acceptance of the playable
flow is not a claim that these exact-equivalence checks have passed.
