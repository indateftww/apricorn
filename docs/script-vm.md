# The script/event VM — `apricorn-core::script`

Reference for the field bytecode interpreter, established in Phase 5.
Sources: pret `src/script.c` (the interpreter, 148 lines, all C),
`src/script_manager.c` (the driver task, bank loading, variable
resolution), `src/data/fieldmap/script_cmd_table.h` (the 853-entry
dispatch table), `asm/macros/script.inc` (the assembler macros, which
fix every command's operand layout), and the `ScrCmd_*` handlers in
`src/scrcmd_*.c` / `src/field/scrcmd_*.c` (850 of 853 are C). The ROM
`hg_usa.nds` is the spec: the inventory tests in `tests/script_hg.rs`
decode the retail banks against the operand table, and the run tests
execute retail scripts against the recording host.

## The bytecode

A script bank is one member of `a/0/1/2` (`NARC_fielddata_script_scr_seq`,
965 members): a table of little-endian u32 entries, one per script,
terminated by the u16 `SCRDEF_END` (`0xFD13`), then bytecode. Each
entry is relative to the byte *after* it (`ScrDef` assembles
`.word target - . - 4`; `ScriptRunByIndex` does `script_ptr += 4 * idx;
script_ptr += ScriptReadWord(ctx)` with the read having advanced the
pointer). `bank::ScriptBank` resolves the table to absolute member
offsets at parse time.

An instruction is a u16 opcode followed by the operands its assembler
macro lays down — bytes, halfwords, words — in exactly the order the C
handler reads them with `ScriptReadByte`/`Halfword`/`Word`.
`commands::OPERANDS` is that layout for all 853 commands; five have a
variable layout keyed on their first operand (`StrengthFlagAction`,
`FlashAction`, `DefogAction`, `ScrCmd_465`, `MysteryGift`). Branch
operands (`GoTo`, `Call`, `GoToIf`, `CallIf`, `ObjectGoTo`, `BGGoTo`,
`DirectionGoTo`, `ScrCmd_225`) and `ApplyMovement`'s list pointer are
relative words measured from after the word, so a backward branch is a
two's-complement word and pointer arithmetic wraps. A movement list is
`(u16 command, u16 length)` pairs ending in command 254.

Several things a script source shows are assembler pseudo-ops, not
commands: `Compare` picks `CompareVarToValue` or `CompareVarToVar` by
its second operand's range; `GoToIfSet`/`GoToIfUnset` are `CheckFlag` +
`GoToIf 1/0`; `GoToIfEq`/`Ne`/`Lt`/... are `GoToIf` with the condition
byte; `Switch`/`Case` are `CopyVar VAR_SPECIAL_x8008` + `Compare` +
`GoToIf eq`.

`commands::disassemble` walks a bank by recursive descent from every
entry point — linear until `End`/`Return`/`GoTo`, following both arms
of every branch, recording every movement list — and produces the
per-opcode histogram the inventory tests pin.

## Which bank a script id names

`LoadScriptsAndMessagesByMapId` scans `sScriptBankMapping` (30 rows,
`bank::STD_BANK_MAPPING`, highest threshold first): a script id at or
above a row's `_std_*` threshold (`include/constants/std_script.h`)
selects that row's fixed script/message bank pair and becomes an index
relative to the threshold. Ids `1..2000` are the current map's own
banks (`MapHeader.scriptsBank`/`.msgBank`), index `id - 1`; id 0 is
the "everywhere" pair (140/184). `bank::resolve_script` is the port.
`_std_init` (9600) is bank 149 index 0 — the new-game init script;
`_std_misc` (2000) is bank 3, where `std_signpost`,
`std_give_item_verbose`, `std_play_mom_music` and friends live.

## The init-script header

`MapHeader.scriptHeaderBank` is another `a/0/1/2` member: `(u8 type,
u32 payload)` entries ending in a 0 byte. Types 2/3/4
(ON_TRANSITION/ON_RESUME/ON_LOAD) carry a map script id in the low
halfword — `GetMapLoadScriptId` — and the field runs it synchronously
with `StartMapLoadScript`. Type 1 (ON_FRAME_TABLE) carries an offset,
relative to after the entry, to rows of `(u16 var1, u16 var2, u16
script)` ending in `var1 == 0`; `GetMapSceneScriptId` returns the
first row whose two variables read equal (ids below `VAR_BASE` are
literals), and the field starts it as a scene script task.
`header::InitScriptHeader` is a minimal reader for both; note for the
orchestrator: the map-data workstream parses the same member in
`field/script_header.rs`, and the two should be merged.

Of the early maps: New Bark Town has a frame table, ON_TRANSITION 7
and ON_RESUME 10; Elm's lab ON_RESUME 11 and a table; the player's
house only a table (`VAR_SCENE_PLAYERS_HOUSE_1F == 0` → script 1, the
Mom scene); Route 29 ON_TRANSITION 1; the bedroom nothing at all.

## Execution model

`context::ScriptContext` is pret's `ScriptContext`: a mode
(STOPPED/BYTECODE/NATIVE), a program counter and a 20-deep call stack
(offsets into the owned bank), `comparisonResult`, four scratch
registers `data[4]`, the installed native wait, and the decoded
message bank. `step` is `RunScriptCommand`: in BYTECODE mode it runs
commands back to back until one *yields* (returns `TRUE` in C); in
NATIVE mode it polls the wait once and, when it holds, returns to
bytecode for the *next* frame. `End` stops the context.

`env::ScriptEnvironment` is pret's `ScriptEnvironment`: up to three
contexts (a scene script and the std scripts it `CallStd`s), the
fourteen special variables `0x8000..=0x800D` (not saved), the
`MessageFormat` placeholder fields, the two string buffers, the object
the player talked to and their facing, the `CallStd` wait mask
(`unk_7`), and the dialogue-window state (`unk_8` and
`fieldSystem->textbox_open`, kept apart because signposts set only the
latter). `run_frame` is `Task_RunScripts`: the first call creates
context 0, then every call steps each live context in slot order — a
`CallStd` callee spawned this frame runs in the same frame — and
destroys the ones that stopped; when none is left the task is finished
(with `scrctx_end_cb` armed if `ScrCmd_061` ran). `run_map_load_script`
is `StartMapLoadScript`: `while (RunScriptCommand(ctx) == TRUE) {}` in
one call, a step budget guarding against a script that never ends.

`CallStd script` creates the callee in slot `activeScriptContextCount`
with that id, sets bit `caller.id` of the wait mask, and parks the
caller on `ScrNative_WaitStd`; the callee's `RestartCurrentScript`
clears bit `id - 1`, releasing the caller while the callee keeps
running. A fourth context is refused (`TooManyContexts`) where the
original would write past the array.

Variables: `FieldSystem_VarGet` reads an id below `VAR_BASE` (0x4000)
as a literal, `0x4000..=0x416F` from save block 4, `0x8000..=0x800D`
from the environment; `ScriptGetVarPointer` names one of the latter two
to write. Flags: `0x4000..` are the static, never-saved `sTempFlags`
(`save::vars_flags::TempFlags`); everything below is block 4. The
typed view `save::vars_flags::VarsFlags` is the port of
`Save_VarsFlags_*`: `u16 vars[0x170]` at 0 then `u8 flags[2912/8]` at
0x2E0, flag 0 reserved.

## The host

The VM reaches everything that is not its own state through
`host::ScriptHost`: the save's flags-and-vars block, the temporary
flags, the field RNG, this frame's new keys, bank loading, the player's
and rival's names, the party, object positions, the dialogue window
(open/close/print), movement (`apply_movement`), and three typed enums
— `FieldAction` (side effects: locks, fades, warps, sound, items,
menus, ...), `FieldQuery` (reads: gender, money, badges, time, ...),
`WaitFor` (native-mode predicates: print finished, movement finished,
fade finished, a child task returned, ...) — plus `launch` for the
applications a script hands control to (naming screen, starter choice,
tutorial battle, mail). Sound is recorded, never played. The
`CallTask_*` commands (`Warp`, `RestoreOverworld`, `ScrCmd_436`,
`CameronPhoto`) wait on `WaitFor::ChildTask`, since the original's
script task is simply not run while the child task is up.

`host::RecordingHost` is the mock: it logs every call as a
`HostEvent`, answers queries and waits from tables (`queries`,
`pending`, `poll_values`, `action_results`), holds a real block-4
image, and can borrow a ROM for banks — so the unit tests and the
ROM-gated runs drive one code path.

## The command subset

`exec::execute` implements 146 of the 853 commands: the opcode
inventory of the early-game banks (below) plus the trivial neighbours
of the control-flow, flag, variable, message and BGM families.
Everything else returns `ScriptError::Unimplemented { opcode, offset }`
— never a panic — with the context stopped, so a field scene can log
and recover. `is_implemented` / `implemented_opcodes` expose the set.

Every handler reads its operands in the C's order and widths and yields
exactly where the C returns `TRUE`. Ported quirks worth knowing:

* `CompareLocalToLocal`/`CompareLocalToValue` truncate the registers
  to `u8` before comparing.
* `NPCMsgVar`/`NonNPCMsgVar` truncate the resolved id to `u8`.
* `GetFriendSprite`, `Random`, `SetSignpostMap`, `SetSignpostAction`,
  `ScrCmd_609`, `BankOrWalletIsFull` and `DirectionSignpost` yield
  without installing a wait: the next command runs next frame.
* `LotoIDSet` draws `LCRandom` twice and, as retail
  `Save_VarsFlags_SetLotoId` does without `BUGFIX_LOTO_NUMBER_HI`,
  writes both halves to `VAR_LOTO_NUMBER_LO`.
* `MenuInit` does not write `data[0]`; retail `MenuExec` reads it as
  the result variable anyway (stale — zero in the Mom savings script),
  while the menu itself holds `MenuInit`'s variable pointer. The port
  writes the choice to `MenuInit`'s variable, and to `data[0]`'s too
  when that resolves.
* `NicknameInput 255` with no Bug Contest catch returns `TRUE` before
  reading its result operand, as the C does.
* `ApplyMovement` on an object the host does not have is not an error
  (the C asserts it was the follower and continues).

### The early-game inventory

`tests/script_hg.rs` decodes bank 149 (`_std_init`), 842 (New Bark
Town), 843 (Elm's lab 1F), 845 (player's house 1F), 846 (bedroom),
225 (Route 29) and the eight `std_misc` scripts they `CallStd` (entries
0, 8, 9, 29, 30, 33, 36, 38) and pins each bank's `(opcode, count)`
histogram, script count, reachable-instruction count and movement-list
count. All 108 distinct opcodes those banks use have handlers. By
family: control flow (End, Wait, Compare*, CallStd,
RestartCurrentScript, GoTo, Call, Return, GoToIf, CallIf), flags and
variables (Set/Clear/CheckFlag, AddVar, SetVar, CopyVar), the dialogue
window (NPCMsg, GenderMsgBox, MsgBoxExtern, GetStdMsgNaix, CloseMsg,
WaitButton, WaitButtonOrDpad), signposts (DirectionSignpost,
SetSignpostMap, SetSignpostAction, WaitSignpostAction, TrainerTips,
WaitSignpost), sound (PlaySE, WaitSE, PlayCry, WaitCry, PlayFanfare,
WaitFanfare, StopBGM, ResetBGM, FadeOutBGM, TempBGM), objects and
movement (ApplyMovement, WaitMovement, LockAll, ReleaseAll, Lock,
Release, ShowPerson, HidePerson, FacePlayer, GetPlayerCoords,
GetPersonCoords, GetPlayerFacing, MovePersonFacing), items (GiveItem,
TakeItem, HasSpaceForItem, HasItem, GetItemPocket, SetStarterChoice),
the party and player (GetPlayerGender, GetFriendSprite, HealParty,
CheckBadge, GetPartyCount, GetPartyMonSpecies, MonGetFriendship,
GetPartyMonForm2, MonHasRibbon, GiveRibbon, GetPartyLeadAlive,
HasEnoughMoneyVar), placeholder buffers (BufferPlayersName,
BufferRivalsName, BufferFriendsName, BufferMonSpeciesName,
BufferPartyMonNick, BufferItemName, BufferItemNamePlural,
BufferPocketName, BufferPartyMonSpeciesNameIndef), applications
(NameRival, NicknameInput, ChooseStarter, CatchingTutorial,
ScrCmd_376), fades and warps (FadeScreen, WaitFade, Warp,
RestoreOverworld, ScrCmd_436, ScrCmd_582), the Pokégear
(RegisterGearNumber, UnsetPhoneCallTrigger), the following Pokémon
(ScrCmd_596/600/602–605/608/609/729), overlay-1 effects
(ScrCmd_307–311), PlaceStarterBallsInElmsLab, menus and Mom's savings
(TouchscreenMenuHide/Show, GetMenuChoice, MenuInit, MenuItemAdd,
MenuExec, BankTransaction, CheckBankBalance, BankOrWalletIsFull,
ScrCmd_795/796), and misc queries (ScrCmd_377 mailbox count,
ScrCmd_379 time of day, GetWeekday, GetGameVersion, LotoIDSet,
PhotoAlbumIsFull, CameronPhoto, ScrCmd_061).

### What the retail scripts do when run

* **`_std_init`** (bank 149, script 0): 143 `SetFlag`s hiding story
  objects, one `LotoIDSet`, `End`. One `RunScriptCommand` call, no
  field side effects; the only variable touched is `VAR_LOTO_NUMBER_LO`.
* **New Bark Town ON_TRANSITION** (script 7): `GetFriendSprite` into
  `VAR_OBJ_0` (97, `SPRITE_HEROINE`, for a male player), then — unless
  `FLAG_UNK_189` is set, in which case it is cleared — clears
  `VAR_TEMP_x4007`, and hides Cameron unless the player has the Plain
  Badge and it is Tuesday. ON_RESUME (script 10) does nothing until
  `VAR_SCENE_NEW_BARK_TOWN_OW == 1`, then shows the friend and her
  Marill at (686, 396) facing west and (685, 396) facing south.
* **Route 29 ON_TRANSITION** (script 1): `GetFriendSprite` into
  `VAR_OBJ_1`, then sets `FLAG_UNK_207` (hides the weekday sibling)
  unless the player has the Zephyr Badge and it is Tuesday.
* **The bedroom** has no ON_TRANSITION. Its PC (script 1) locks,
  plays a SE, greets the player by name, counts the mailbox: with no
  mail one more message dismissed by a button; with mail a fade-out,
  the mail application, `RestoreOverworld`, and a fade-in.
* **The Mom scene** (player's house 1F, frame table → script 1 on a
  new game): the player turns, Mom walks over (four movement lists),
  `std_play_mom_music` runs in a second context, a 30-frame `Wait`,
  five messages with four fanfares setting `FLAG_GOT_BAG`,
  `FLAG_GOT_TRAINER_CARD`, `FLAG_GOT_SAVE_BUTTON`,
  `FLAG_GOT_OPTIONS_BUTTON`, a 15-frame `Wait`, Mom walks back,
  `std_fade_end_mom_music`, `VAR_SCENE_PLAYERS_HOUSE_1F = 1`.

## Not ported

The hidden-item parameter fill `SetupScriptEngine` does for ids
`8000..8800` (`GetHiddenItemParams`), the engaged-trainer records, the
`RunScript` (opcode 19) second-slot spawn, and every command outside
the subset. The field scene that implements `ScriptHost` for real —
the dialogue box on `app/text.rs`'s `TextPrinter`, the map object
manager, the palette fader — belongs to the field workstream.
