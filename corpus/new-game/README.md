# `new-game` — power-on to the bedroom on the retail ROM

The Phase 4 corpus case: the oracle (patched headless melonDS 1.1,
`docs/oracle.md`) boots `hg_usa.nds` with a **blank card** and a pinned
RTC of `2010-03-01T09:00:00`, and `input.apin` drives it from power-on
through the intro, the title screen, Oak's speech, the naming screen
(the name `MATTS`, entered by stylus), the two confirmations and the
fade into the player's bedroom (map 64), ending at frame 5000 with the
field fade-in long complete. `regions.conf` is identical to
`boot-idle`'s (the two boot RNG states), so `expected.trace` pins the
RNG state at every frame of the flow — a ground-truth baseline for the
engine's new-game path.

Every frame below was measured on the oracle with `--shots` (both LCDs
as PNGs at the end of every frame of the run) and confirmed by looking
at the pictures; "frame N" means the picture the oracle writes at the
end of frame N, i.e. the machine state the trace's `F N …` records
hash. Regenerate the review set with

```
scripts/shots.ps1 corpus/new-game "201,421,643,1001,1032,1262,1312,1434,1534,1661,1714,1822,1959,2081,2701,2743,3577,3585,3638,3732,3876,3902,3912,3922,3932,3942,3962,4060,4110,4172,4545,4582,4700,4722,4810,4816,4999"
```

(PNGs land in `out/oracle-shots/new-game/`, gitignored — review
artifacts, never corpus data.)

## The flow, with input events

Screens are named after pret's state/message identifiers
(`src/intro_movie.c`, `title_screen.c`, `application/main_menu/main_menu.c`,
`oaks_speech.c`, `naming_screen.c`); no game text is reproduced here.

| Frame | What is on the LCDs / what happens | Input event (`input.apin`) |
|---|---|---|
| 0–198 | both LCDs blank while the ARM9 image decompresses and the intro overlay loads | |
| 185 | `InitializeMainRNG`: `sLCRNG_State` / `sMTRNG_State` seeded from the pinned clock (first LCRNG change in the trace) | |
| 186 | `IntroMovie_Init` saves the LCRNG seed and zeroes it (`SetLCRNGSeed(0)`) — the LCRNG hash returns to the zeroed-bss value | |
| 199 | black | |
| 201 | copyright notice on the top LCD, the ESRB notice on the bottom | |
| 321–379 | copyright fades out (4-frame steps); 379 black | |
| 421 | the Game Freak logo card on the **bottom** LCD (the intro movie runs with the screens flipped) | |
| 643 | intro movie scene 1 (sky on top, the sea at sunset below); the skip flag is set inside this scene | |
| 1000 | A pressed: the intro is skipped | `1000 down A`, `1002 up A` |
| 1001 | both LCDs white (the skip's white-out) | |
| 1002 | `IntroMovie_Exit` restores the saved LCRNG seed (trace: the frame-185 hash again) | |
| 1032 | title screen begins: the sun flash on top, Ho-Oh on the bottom | |
| 1262 | title logo settled, start prompt visible (stable top-LCD hash from here) | |
| 1300 | A pressed at the title (`TITLESCREEN_EXIT_MENU`; never UP+SELECT+B, the save-clear combo) | `1300 down A`, `1302 up A` |
| 1304–1338 | the exit flash (white at 1312), back to the title by 1338 | |
| 1424–1434 | fade to black; 1434 black — `gApplication_CheckSave` runs with nothing to report and the main menu (`ov74`) sees no save file, so **there is no menu screen**: it registers `ov36_App_MainMenu_SelectOption_NewGame` directly | |
| 1480 | new-game init: `InitializeMainRNG` re-seeds the LCRNG (trace hash changes); the MT state changes at its next sample (1500) after the two roamer draws | |
| 1524–1534 | Oak's speech: the button-tutorial background fades in (complete at 1534) | |
| 1627–1657 | the tutorial prompt text fades in on the top LCD | |
| 1661 | the three-button tutorial menu on the bottom LCD (`OAK_SPEECH_MAIN_STATE_TUTORIAL_MENU_HANDLE_INPUT`) | |
| 1700 | B: the menu enters pad mode (cursor frame visible at 1703) | `1700 down B`, `1702 up B` |
| 1710 | B again: pad-mode B selects the **last** option (index 2, `NO_INFO_NEEDED`); its 20-frame press flash starts at 1714 | `1710 down B`, `1712 up B` |
| 1740 | A pulses begin: 4 frames down, 4 up, period 8, until 3780 — Oak's dialogs page-wait on a fresh A press (`AUTO_SCROLL_OFF`), and a held A speeds the printer | `1740 down A`, `1744 up A`, … `3776 down A`, `3780 up A` |
| 1756–1806 | prompt text fades out, then both LCDs fade to black | |
| 1812–1822 | the no-info layout fades in (the touch-to-advance button on the bottom LCD) | |
| 1906 | the time-of-day greeting dialog opens (msg 2 for a 09:00 clock); printed by 1959 | |
| 2049–2081 | Oak fades in (`SHOW_OAK`; complete at 2081), then his dialogs page by page | |
| 2701 | the Poké Ball flash (both LCDs brightened) | |
| 2743 | Marill appears | |
| 3547–3555 | the bottom LCD fades out for the gender question | |
| 3577 | gender select menu: both portraits (`GENDER_SELECT_MENU_HANDLE_INPUT`) | |
| 3585 | the boy is chosen: one A pulse arms the pad, the next confirms the resting cursor (index 0) | (pulses) |
| 3631–3638 | the gender confirmation YES/NO menu; YES flashes from 3661 (the pulses arm and confirm it) | (pulses) |
| 3683–3732 | the name prompt dialog (msg 40) prints; then a 40-frame wait | |
| 3780 | last A pulse released — nothing must be pressed once the keyboard is up (A would type the cursor's key) | `3780 up A` |
| 3823–3833 | fade to black: the naming screen overlay is constructed | |
| 3846–3876 | the naming screen fades in; 3876 is the first fully-faded-in frame: uppercase page, cursor on the first key, empty name | |
| 3900 | stylus on M — row 2 (`y 0x6B`) column 2 of `sTouchHitboxDef`, 16×19 cells from `x 0x1C`; the letter is in the name box at 3902 | `3900 touch 68 116`, `3902 lift` |
| 3910 | A (row 1, column 0) | `3910 touch 36 97`, `3912 lift` |
| 3920 | T (row 2, column 9) | `3920 touch 180 116`, `3922 lift` |
| 3930 | T again — a fresh touch (`touchNew`) after the lift | `3930 touch 180 116`, `3932 lift` |
| 3940 | S (row 2, column 8); the name box reads `MATTS` at 3942 | `3940 touch 164 116`, `3942 lift` |
| 3960 | OK (the 32×22 home-row button at `x 0xC5, y 0x3C`); pressed at 3962 | `3960 touch 213 71`, `3962 lift` |
| 3964–4028 | the naming screen fades out (top first, then the keyboard) | |
| 4000 | A pulses resume (period 8, hold 4) until 4600 | `4000 down A` … `4596 down A`, `4600 up A` |
| 4050–4060 | Oak returns with the name confirmation dialog (msg 41) and its YES/NO menu | |
| 4110–4132 | YES selected and flashing (armed and confirmed by the pulses) | (pulses) |
| 4138–4168 | bottom LCD fades out and back in with the touch button | |
| 4172 | the closing dialog (msg 43) begins; its last page is printed by 4545 | |
| 4558–4566 | both LCDs fade to black | |
| 4574–4582 | the player's picture fades in (`FADE_IN_TO_SHRINK_ANIM`) | |
| 4639–4692 | the picture whitens and shrinks (`RUN_SHRINK_ANIM`) | |
| 4712–4722 | fade to black; Oak's overlay exits | |
| 4727 | the post-Oak pass (`ov36_App_MainMenu_SelectOption_NewGame` exit): `InitializeMainRNG` re-seeds the LCRNG; LC draws at 4729 and 4736 (the two for the temporary mail Pokémon), and the MT re-seed plus its twelve draws show at the 4740 sample | |
| 4806–4816 | the bedroom (map 64) fades in over six 2-frame steps; **4816 is the first fully-faded-in frame** | |
| 4816–4999 | standing in the bedroom, facing south; both LCDs are pixel-identical from 4816 to the end of the run | |

The LCRNG hash sequence in `expected.trace` therefore changes at
exactly 185, 186, 1002, 1480, 4727, 4729 and 4736; the MT hash (sampled
every 30 frames) at 150, 210, 1500 and 4740.

## How the script was written

Empirically, on the oracle: coarse `--shots` scans every 10–20 frames
located each screen, per-frame scans pinned the transitions, and the
input was extended one step at a time (`apricorn-replay --shots … <case>`
writes the pictures even before an `expected.trace` exists). The
choices that keep it deterministic and short:

* the intro movie is skipped with A (allowed once scene 1's title card
  is up), and the title is left with A — the touch-to-start and START
  paths are equivalent but untested here;
* the tutorial menu is answered with B, B (pad-arm, then "last option")
  rather than DOWN, DOWN, A, so no cursor position is assumed;
* A is *pulsed*, not held: page waits need `newKeys`. The pulses double
  as the gender pick and both YES answers (arm, then confirm the
  resting cursor), so they must be silent only while the keyboard is
  up — between 3780 and 4000;
* the name is typed by stylus with a lift between taps (a repeated key
  needs a new `touchNew`), then OK is tapped; a typed name draws no RNG.
