# pret/pokeheartgold inventory (Phase 0)

Shallow clone at `refs/pokeheartgold` (gitignored, not vendored).
Snapshot taken 2026-09-06 against master.

## Overall coverage

| | files | lines |
|---|---|---|
| Decompiled C (`src/`) | 389 | ~198,800 |
| Headers (`include/`) | 564 | — |
| Remaining assembly (`asm/`) | 294 | ~1,100,600 |

Roughly speaking the *core game* (overlay 0/1 = field system, script commands,
save, field rendering) is largely decompiled, while the *overlay applications*
(menus, Frontier facilities, side games) skew heavily toward asm — dozens of
`overlay_NN.s` files with no C counterpart.

## Subsystems apricorn cares about

| Subsystem | Status | Where |
|---|---|---|
| **RNG (LCG + MT)** | ✅ **fully decompiled** | `src/math_util.c` — `LCRandom()` (state × 1103515245 + 24691, /65536) and the Mersenne Twister, seeds + getters |
| **Save system** | ✅ largely C | `src/save*.c` (~3,800 lines incl. per-block files: vars_flags, misc, local_field_data, trainer_card…) |
| **Event scripts (scrcmd VM)** | ✅ largely C | `src/scrcmd_*.c` + `src/script*.c` (~12,200 lines) + `src/data/fieldmap/script_cmd_table.h` |
| **Field system** | ✅ largely C | `src/field_*` + `src/field/` (fieldmap, control, encounter_check, warps, map events) |
| **Battle** | ⚠️ **~half C** | `src/battle/` ≈ 29,100 C lines (battle_system, battle_command, setup, player controller, trainer AI); but `asm/overlay_12_*.s` still holds ≈ 33,900 asm lines incl. battle_controller_opponent, parts of battle_command |
| **Overlay apps** (Pokégear, Pokéathlon, Frontier, etc.) | ⚠️ mostly asm | dozens of `overlay_NN.s`; some exceptions (`voltorb_flip/`, `pokeathlon/`, `frontier/` in src) |

## Implications for the plan

- **Phase 4 (RNG)**: no arm-runner needed for RNG parity — we can port the
  decompiled LCG/MT directly and unit-test against the published constants.
- **Phase 5 (overworld/scripts)**: strong C documentation exists; use it as
  primary source, verify with replay traces.
- **Phase 6 (battle)**: the biggest pret gap. Battle core math that *is* in C
  (battle_system, battle_command) is ported + differential-tested; the
  opponent-controller/AI side will lean on `arm-runner` against the binary.
- **Field names still cryptic**: many `unk_0200….c` files carry their overlay
  address as the name — expect archaeology even in "decompiled" areas.

## Maintenance

- `git -C refs/pokeheartgold pull` to refresh; re-run the file/line counts.
- pret moves fast (2026 PRs still converting overlays); re-check battle
  coverage before starting Phase 6 — the gap may have shrunk by then.