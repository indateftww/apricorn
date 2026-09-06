# SDAT — the sound data archive

Reference for `apricorn-core::formats::sdat`, established in Phase 1 by
scanning both sound archives in the retail US dump (`hg_usa.nds`) and
cross-checking against GBATEK's SDAT spec, pret/pokeheartgold's
`lib/include/nitro/snd/common/data.h`, `lib/include/nnsys/snd/sndarc.h`
and `seqdata.h`, and the `NNSi_SndArcLoadGroup` assembly in
`lib/asm/nnsys.s`. **The retail image is the oracle**: every invariant
below is pinned with zero violations across both archives and enforced
at parse time. Playing any of this (SSEQ programs, SWAR samples) is
Phase 7 work — this module resolves container structure only.

The ROM carries two SDATs:

| | Main archive | PBR archive |
|---|---|---|
| Path (FAT id) | `data/sound/gs_sound_data.sdat` (479) | `pbr/sound_data.sdat` (507) |
| Bytes | 8,660,096 | 7,484,768 |
| Files | 2,353 = 1,231 SSEQ + 561 SBNK + 561 SWAR | 1,846 = 812 + 517 + 517 |
| File bytes | 8,488,588 | 7,337,508 |
| SEQ entries | 2,379 (1,372 present/named) | 2,133 (829) |
| BANK entries | 778 (561) | 1,519 (517) |
| WAVEARC entries | 778 (561) | 1,519 (517) |
| PLAYER entries | 9 (PLAYER_PV, PLAYER_FIELD, PLAYER_ME, PLAYER_SE_1–4, PLAYER_BGM, PLAYER_OPED) | 8 |
| GROUP entries | 17 (GROUP_GLOBAL, GROUP_SE_FIELD, GROUP_SE_BATTLE, …) | 16 |
| SEQARC / STRM entries | 0 / 0 | 0 / 0 |

The main archive is byte-identical to pret's `files/data/sound/gs_sound_data.sdat`.
Neither archive contains STRM streams — all audio is SSEQ sequences
played through SBNK banks drawing on SWAR sample archives. All counts
are pinned by `crates/apricorn-core/tests/sdat_hg.rs`, which parses both
archives end to end.

## Header and block walk

The container opens with the SDK's `SNDBinaryFileHeader` extended to
0x40 bytes:

| Offset | Size | Field |
|--------|------|-------|
| +0x00 | 4 | magic `SDAT` |
| +0x04 | 2 | byte-order mark 0xFEFF |
| +0x06 | 2 | version 0x0100 |
| +0x08 | 4 | file size (== archive length) |
| +0x0C | 2 | header size 0x40 |
| +0x0E | 2 | block count (4 with SYMB, 3 without) |
| +0x10 | 4+4 | SYMB offset + size, or (0, 0) |
| +0x18 | 4+4 | INFO offset + size |
| +0x20 | 4+4 | FAT offset + size |
| +0x28 | 4+4 | FILE offset + size |
| +0x30 | 0x10 | reserved (zero) |

All four offsets are from SDAT start. The blocks then chain by their
**own** 8-byte headers (`magic`, `u32 size`) starting at 0x40 — not by
the header pairs. Own sizes are 4-byte aligned and may exceed their pair
by up to 3 bytes of padding: the main archive's SYMB owns 0xD7D4
against a pair of 0xD7D1 (own == align4(pair)). The FILE block must
cover everything to EOF exactly. Block positions in the retail archives:

| | SYMB | INFO | FAT | FILE |
|---|---|---|---|---|
| Main | 0x40, own 0xD7D4 | 0xD814, own 0xA638 | 0x17E4C, own 0x931C | 0x21168, own 0x821318 |
| PBR | 0x40, own 0xBA04 | 0xBA44, own 0x9EF8 | 0x1593C, own 0x736C | 0x1CCA8, own 0x7068B8 |

## SYMB — symbol block

Eight sub-lists in file order — seq, seqArc, bank, waveArc, player,
group, strmPlayer, strm — each `{u32 count, u32 label_offset[count]}`,
then one shared string pool. Two quirks pinned empirically:

* The sub-lists **tile back-to-back exactly** from block offset 0x40:
  `rel[i+1] == rel[i] + 4 + 4·count[i]`. (Contrast INFO, below.)
* Label offsets are relative to the **SYMB block start**, not the list
  or pool — with 0 meaning unnamed. 1,007 of the main archive's 2,379
  seq entries are unnamed.

Labels are non-empty printable ASCII, NUL-terminated inside the block.
Retail archives carry all eight sub-lists, and their seqArc sub-list is
empty — folder labels inside an SSAR are not supported by this parser.

## INFO — info block

The same eight sub-lists, but each holds `{u32 count, u32
record_offset[count]}` where record offsets are relative to the block
start and 0 means an absent record. The lists do **not** tile: each
list's records interleave in the gap between its offsets array and the
next list's start (the block end for the last list). Present record
offsets are strictly ascending and start at or after their own list's
array end.

Record layouts (`sndarc.h`, GBATEK, `seqdata.h`):

* **SEQ** (12): `{u16 file, u16 -, u16 bank, u8 vol, u8 channel_prio,
  u8 player_prio, u8 player, u16 -}` — the fields of
  `NNSSndSeqParam`.
* **SEQARC** (4): `{u16 file, u16 -}`. Count 0 in retail.
* **BANK** (12): `{u16 file, u16 -, u16 swar[4]}` — 0xFFFF = slot
  unused.
* **SWAR** (4): `{u16 file, u16 -}`.
* **PLAYER** (8): `{u8 seq_count, u8 -, u16 channels, u32 heap}` — the
  arguments of `SetPlayableSeqCount` / `SetAllocatableChannel` /
  `CreateHeap`. Retail: PLAYER_PV (2, 0xC000, 24200), PLAYER_FIELD
  (1, 0xA7FE, 15500).
* **GROUP** (variable): `{u32 count, item[count]}` with each item 8
  bytes. GBATEK's "group id" values (0x700/0x803/0x601/0x402) are the
  little-endian byte encoding of `{u8 type, u8 flags}`: `NNSi_SndArcLoadGroup`
  reads byte 0 as a jump-table type (0 SEQ, 1 BANK, 2 WAVEARC, 3
  SEQARC), byte 1 as the flag argument, and the `u32` at +4 as the
  index into the named sub-list. All retail items are SEQ with flags 7
  (one flags-6 item in the pbr archive).
* **STRMPLAYER** (0x18): parsed structurally only, not exposed. Count 0
  in retail.
* **STRM** (12): `{u16 file, u16 -, u8 vol, u8 pri, u8 player, u8
  -[5]}`. Count 0 in retail — the layout follows GBATEK and is verified
  by synthetic test only.

## FAT and FILE — the file image

**FAT** is `{magic, u32 size, u32 count, entry[count]}`, where size must
be exactly `12 + 16·count` and each entry is `{u32 offset, u32 size,
u32 mem, u32 reserved}`:

* Offsets are **absolute SDAT offsets** (not FILE-relative), which is
  unusual — the NitroFS FAT and NARC BTAF are both container-relative.
* mem and reserved are zero; `(0, 0)` means an empty entry (none in
  retail).
* Files are 4-byte aligned, strictly ascending, and confined to
  `[FILE+0x18, archive end]`.

**FILE** is `{magic, u32 size, u32 count, 12 reserved bytes}` followed
by the file data from +0x18. `count` equals the FAT count. Note the
0x18-byte header: GBATEK documents 0x10, but retail HGSS stores twelve
zero bytes after the count.

## Retail invariants (all enforced at parse time)

* A named SYMB entry is exactly a present INFO record — per entry, on
  every list, in both archives.
* Every SEQ record's file id resolves to a file beginning `SSEQ` (banks
  → `SBNK`, wave arcs → `SWAR`, sequence archives → `SSAR`, streams →
  `STRM`).
* SEQ `bank` is 0xFFFF or a valid bank index (retail uses 1, 2, and
  700–777 — never 0xFFFF); `player` is below the player count (0–7);
  `volume` ≤ 127.
* GROUP items reference indices inside their own sub-list.
* The SYMB and INFO list counts agree per list.

SDAT is the one format **excluded from the round-trip guard**
(`docs/roundtrip.md`): the writer's string-pool packing and record
interleaving are not retained, so the file cannot be re-serialized
byte-exact from the parsed struct. Phase 7 revisits SDAT.

## Worked example

`SEQ_PV001` (main archive, seq index 1): record
`00 00 00 00 01 00 78 7F 40 00 00 00` — file 0 (an SSEQ of 0x2C bytes),
bank 1 (`BANK_PV001`, file 1231, swar slot 0 = `WAVE_ARC_PV001`, file
1792), volume 120, channel priority 127, player priority 64, player 0
(`PLAYER_PV`). The game loads it through `PLAYER_PV`, whose record
`02 00 00 C0 98 5E 00 00` gives 2 simultaneous sequences, channels
0xC000, and a 24,200-byte heap.