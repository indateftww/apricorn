# The save format — `apricorn-core::save`

Reference for the card-backup container, established in Phase 4 step
4. Source: pret `src/save.c` + `include/save.h` +
`src/save_arrays.c`, with the retail ARM9 image as the oracle: every
constant table here was *measured out of the ROM* (the size stubs
called through arm-runner, the CRC compared stream by stream), not
hand-copied.

## The card

HeartGold writes a 512-KiB card backup: 128 flash sectors of
`SAVE_SECTOR_SIZE` (0x1000) bytes. The layout, in card offsets:

```text
0x00000   slot 0 ── main chunk    (blocks 0..=40 + footer)     0x F628 bytes
0x0F700   slot 0 ── PC chunk      (PCSTORAGE + footer)         0x12310 bytes
0x23000   extra chunks, primary copies   (sectors 35, 38, 39, 41, 43, 45)
0x40000   slot 1 ── the same two chunks, one save generation behind
0x63000   extra chunks, mirror copies     (primary sector + 64)
0x80000   end
```

The two slots are a mirror pair: every save writes the slot the last
good save does *not* occupy (`lastGoodSector == 0 ? 1 : 0`), bumps
the save counter, and flips the good sector — so a crash mid-write
always leaves the previous generation intact. `Save_DeleteAllData`
(the "delete save" menu item) clobbers all four chunk footers first,
then fills all 128 sectors with 0xFF.

## The block table

The dynamic region — 35 flash pages, `SAVE_PAGE_MAX *
SAVE_SECTOR_SIZE` = 0x23000 bytes — holds 42 blocks laid out at boot
by `SaveData_InitSubstructs`:

* Each block is `((raw + 3) & ~3) + 4` stored bytes: the subsystem
  struct padded to 4, a trailing u16 CRC, and 2 tail bytes.
* After block 40 comes the main chunk footer, then the address is
  aligned to 0x100 and block 41 (PCSTORAGE, 0x122FC bytes — over a
  third of the whole region) starts chunk 1.
* The port computes the same layout at compile time
  (`save::layout::BLOCKS`, a const-fn port of the original's
  arithmetic, quirks included). No value is hand-typed: the raw sizes
  are the returns of the 42 `gSaveChunkHeaders` size stubs, each one
  a pin in `pins/arm9.tsv` that `tests/save_hg.rs` calls out of the
  ROM and compares to the committed `BLOCK_RAW_SIZES`.

Two invariants pin the table as a whole (unit tests, `layout.rs`):

* The chunks tile the flash budget *exactly* — 16 pages main + 19 PC
  = 35 = `SAVE_PAGE_MAX`, zero slack. A single wrong size cannot
  satisfy this.
* `PCStorage_sizeof`'s 0x122FC falls out of pret's
  `struct PokemonStorageSystem` arithmetic independently
  (18 boxes × 0x1000 + bookkeeping).

## Checksums

Everything is CRC-16/CCITT-FALSE (`GF_CalcCRC16` → SDK
`MATH_CalcCRC16CCITT`): poly 0x1021, init 0xFFFF, no reflection, no
final XOR — check value `crc16(b"123456789") == 0x29B1`. Three
scopes:

* **Per block**: a u16 at `offset + stored_size - 4`, computed over
  the *padded* span (`stored_size - 4` bytes, `SaveSubstruct_UpdateCRC`).
  For word-unaligned structs like SPECIAL_RIBBONS (0xE bytes), the
  padding bytes count.
* **Per chunk** (`struct SaveChunkFooter`, 16 bytes:
  `{count, size, magic, slot, crc}`): the `crc` covers the chunk's
  `size - 16` stored bytes. The `magic` is `0x20060623`; `slot` is
  the chunk id (0 main, 1 PC); `count` is the save counter.
* **Per extra chunk** (`struct SaveArrayFooter`, 16 bytes:
  `{magic, saveno, size, idx, crc}`): the `crc` covers the raw bytes
  *plus the footer's first 14 bytes* — everything up to the `crc`
  field itself (`size + offsetof(struct SaveArrayFooter, crc)`), a
  detail easy to get wrong. `saveno` is `lastGoodSaveNo + 1`, the
  extra-chunk generation counter.

The differential (`apricorn-harness/tests/save_hg.rs`) runs the
original's own path: `MATHi_CRC16InitTable` builds a real table in
scratch RAM, the game's `sCRC16TablePtr` global is pointed at it,
and `GF_CalcCRC16` hashes each test stream — including the exact
vectors the core unit tests carry.

## The boot probe (`Save_GetSaveFilesStatus`)

`SaveData::parse` ports the probe verbatim:

1. Read both slots' main and PC chunk footers; each validates iff
   size, magic, slot id, and CRC all match (`ValidateSaveSectorFooter`).
2. `SaveSlotCheckCompare` orders each chunk kind's two probes: how
   many are good, which slot is newer, which older (`2` = none).
3. The matrix: both slots good and counters agree → load, clean.
   The newer generation torn (its two chunks disagree on the
   counter) → fall back to the older slot, *degraded* — the case the
   game banners with a "save file is corrupted but can be loaded"
   warning (`SaveData::slot_degraded`). One chunk of a kind lost on
   both slots → refuse (`SaveError::Corrupt`). Nothing valid at all →
   blank card (`SaveError::NoSaveData`, the game starts a new game).

**The wraparound quirk** (`SaveCounterCompare`): `(0xFFFFFFFF, 0)`
compares as *older* — an all-0xFF clobbered footer counts as -1, so
a freshly wrapped counter of 0 beats it. This is deliberate
(clobbered sectors must lose to real saves at the wrap point) and
pinned by tests in both directions, for chunk counters and extra
chunk `saveno`s alike.

## Saving (`SaveGameNormal`)

`SaveData::save_game` reproduces the card-side effect of a save:

* `saveCounter++` *before* the footers are built
  (`Save_WriteManInit`), so the new generation's footers carry
  counter + 1.
* Both chunks copy verbatim from the good generation into the other
  slot; each footer is rebuilt with the new counter
  (`SaveSlot_BuildFooter`).
* `lastGoodSector` flips (`Save_WriteManFinish`).

The original's per-PC-box write skip (`boxModifiedFlags`) is flash
wear leveling only: skipped boxes already hold identical bytes on the
card, so writing the whole chunk is byte-identical. Block CRCs are
not touched by the save itself — subsystems update their own via
`SaveSubstruct_UpdateCRC` (`SaveData::update_block_crc`) when they
edit.

## Extra chunks (`gExtraSaveChunkHeaders`)

Six chunks live outside the slots, each with a primary copy at its
sector and a mirror at sector + 64: the Hall of Fame (0x2AB0) at
sector 35, a battle-record chunk (0xBA0) at 38, and four 0x1D50
record chunks at 39/41/43/45 (one size stub, `sub_0202FBCC`, serves
all four). `SaveData::extra_chunk` ports `ReadExtraSaveChunk`: both
copies are validated, the newer `saveno` wins, and neither copy
validating yields no data. Writing them (`WriteExtraSaveChunk`,
which stamps `saveno` and double-writes for atomicity) is deferred
to the steps that own their contents (Hall of Fame recording, etc.).

## Quirks pinned deliberately

* **`SaveCounterCompare` wraparound** — see above.
* **The boot-time OOB read.** `SaveData_InitSubstructs` reads
  `hdr[i + 1]` past the table's end at the last block; the alignment
  condition's `i + 1 < N` guard means the value never flows
  anywhere. The port skips the read (the const-fn boundary test
  derives block i+1's chunk from its id instead of the half-built
  array — a second-order trap that porting "verbatim" naively into a
  const context would spring).
* **`Save_GetSaveFilesStatus`'s 1+1 branch returns IS_GOOD**, not
  SLOT_FAIL: when one whole slot survives alone (one good main, one
  good PC, same slot), it loads *clean* even though half the card
  failed.
* **Extra-chunk CRC spans 14 footer bytes**, not 12 (`offsetof(crc)`
  in a `{u32, u32, u32, u16, u16}` struct) — found the hard way.

## Coverage

`apricorn-harness/tests/save_hg.rs` (ROM-gated): all 45 size stubs
vs `BLOCK_RAW_SIZES` / `EXTRA_CHUNK_SIZES`; `GF_CalcCRC16` vs
`crc16` over the unit-test vectors, odd lengths, the empty stream,
and an LCG-generated flash page.

`apricorn-core/tests/save.rs` (headless): synthesized retail-shaped
blobs — byte-identical load/save round-trip, the full boot status
matrix, the wraparound quirk in both directions, `save_game`'s
alternating slots with the previous generation preserved untouched,
block views and CRC upkeep, extra-chunk selection — plus a
retail-gated test: a real `hg.sav` at the repo root must load,
round-trip byte-identically, and re-load after a `save_game`
generation with counter + 1 (skips silently when no dump exists,
same policy as the ROM tests).

## Deferred

* **Per-block contents.** Each subsystem owns its struct; the views
  (`SaveData::block`) land with their consumers (Party is Phase 6).
* **New-game initialization.** `Save_InitDynamicRegion_Internal`
  zeroes the region and runs every `initFunc` — Phase 4 step 5+.
* **Extra-chunk writing**, the battle-record saveno bookkeeping in
  MISC, and the Frontier/`Save_CheckFrontierData` status flags: with
  the subsystems that use them.