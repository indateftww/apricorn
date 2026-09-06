# NCER / NANR — the sprite cell & animation formats, as HeartGold uses them

Reference for `apricorn-core::formats::{ncer, nanr}`, established in Phase 1
by scanning every member of every NARC in the retail US dump (`hg_usa.nds`)
and cross-checking against the NitroSDK headers bundled with
pret/pokeheartgold (`NNSG2dCellDataBank`, `NNSG2dAnimBankData` — the runtime
structs the game unpacks these files straight into) and nitrogfx's reader
(`ReadNtrCell`/`ReadNtrAnimation`). Where nitrogfx and the retail image
disagree (the UAAT block's internal layout, for one), **the retail image is
the oracle**; every invariant below is pinned with zero violations across all
retail files, and enforced at parse time.

Census (all counts pinned by `crates/apricorn-core/tests/sprite_hg.rs`, which
parses every member in the ROM):

| Format | Magic | Members | Notes |
|--------|-------|---------|-------|
| NCER — cell banks | `RECN` | 608 | 11,703 OAM entries in total |
| NANR — animation banks | `RNAN` | 591 | 2,500 sequences, 6,171 frames |

Both are NNS containers (byte-order mark 0xFEFF, version **always 0x0100**,
section count **always exactly 3**): `[KBEC|KNBA, LBAL, TXEU]`. The sections
are the same shared-reversed-magic scheme as NCGR/NCLR/NSCR
(see `docs/nitro-gfx.md`); the third section `TXEU` ("UEXT" reversed) is a
fixed 0x0C-byte block of zero marking the file as extensible with user
blocks.

## NCER — cell banks (`RECN` → KBEC, LBAL, TXEU)

The KBEC section body (`cellBankAttr`/offsets below are relative to the
section body, i.e. section offset + 8):

| Offset | Size | Field | HeartGold value |
|--------|------|-------|-----------------|
| +0x00 | 2 | `nCells` | 1–224 |
| +0x02 | 2 | `cellBankAttr` | bit 0 = extended records; **all other bits zero** |
| +0x04 | 4 | `cellRecordsOffset` | always 0x18 |
| +0x08 | 4 | `mapping` | `NNSG2dCharacterDataMapingType` (below) |
| +0x0C | 4 | `vramTransferOffset` | 0 = none (157 files), else body-relative |
| +0x10 | 4 | reserved | always 0 |
| +0x14 | 4 | `ucatOffset` | 0 = none (146 files), else body-relative |

Note the mapping field: unlike a NCGR's `GXOBJVRamModeChar` (packed DISPCNT
bits, `docs/nitro-gfx.md`), the cell bank stores the SDK's
`NNSG2dCharacterDataMapingType` enum — plain 0–4:

| Raw | Meaning | HeartGold count |
|-----|---------|-----------------|
| 0 | `1D_32` | 284 |
| 1 | `1D_64` | 198 |
| 2 | `1D_128` | 92 |
| 3 | `1D_256` | 34 |
| 4 | `2D` | 0 — never used |

The body after the header tiles exactly, in order, each block starting where
the previous ends:

1. **Cell records**, `nCells` × (8 bytes, or 0x10 when extended):
   `u16 nOAM, u16 cellAttr, u16 oamDataOffset, u16 reserved(0)`, plus
   `s16 maxX, s16 maxY, s16 minX, s16 minY` for extended records
   (`min ≤ max` always). `cellAttr` packs bits 0–5 bounding-sphere radius,
   8 h-flip, 9 v-flip, 10 hv-flip, 11 bounding-rect flag
   (`cellAttr & ~0x3FFF == 0` on all retail). `oamDataOffset` is a chain:
   cell *i*'s OAM data sits at `6 × ΣnOAM` over the preceding cells.
2. **OAM data**, 6 bytes per entry — the raw GBA/DS sprite-hardware
   registers `attr0, attr1, attr2` (shape/size, position, tile index,
   palette) — padded to 4-byte alignment relative to the body.
3. **VRAM transfer block** (when `vramTransferOffset != 0`): `u32
   szByteMax, u32 reserved(8)`, then one `(u32 srcOffset, u32 size)` pair
   per cell — the DMA sources for streaming that cell's graphics into OBJ
   VRAM. (473 files use extended cell records; 157 carry this block.)
4. **UCAT block** (when `ucatOffset != 0`): `"TACU", u32 size
   (= 0x10 + 8*nCells), u16 numCells, u16 attrsPerCell(1), u32 reserved(8)`,
   then `nCells` u32 offsets and `nCells` u32 attributes. Every pointer
   value on every retail file is **fully derived** (`8 + 4*nCells + 4*i`,
   pointing at attribute *i*) — the parser validates rather than trusts.

## NANR — animation banks (`RNAN` → KNBA, LBAL, TXEU)

The KNBA section body:

| Offset | Size | Field | HeartGold value |
|--------|------|-------|-----------------|
| +0x00 | 2 | `nSequences` | 1–43 |
| +0x02 | 2 | `nFrames` | total across all sequences; **equals Σ frameCount** |
| +0x04 | 4 | `sequenceOffset` | always 0x18 |
| +0x08 | 4 | `frameOffset` | always 0x18 + 0x10·nSequences |
| +0x0C | 4 | `resultOffset` | results pool base, body-relative |
| +0x10 | 4 | reserved | always 0 |
| +0x14 | 4 | `uaatOffset` | 0 = none (146 files), else body-relative |

Then, in order:

1. **Sequence records**, 0x10 bytes each: `u16 frameCount, u16
   loopStartFrame (< frameCount), u16 animationElement, u16 animationType,
   u32 playbackMode, u32 frameDataOffset`. `animationType` is **always 1**
   on retail. `animationElement` picks the result layout:

   | Raw | Meaning | Result size | Sequences |
   |-----|---------|-------------|-----------|
   | 0 | cell index | 2 | 2,246 |
   | 1 | SRT (scale-rotate-translate) | 0x10 | 73 |
   | 2 | translate only | 8 | 181 |

   `playbackMode` is `NNSG2dAnimationPlayMode`: 1 = Forward (1,048
   sequences), 2 = ForwardLoop (1,452); the SDK also defines Reverse(3)
   and ReverseLoop(4), unused by HeartGold.

2. **Frame records**, 8 bytes each: `u32 resultOffset, u16 frameDelay,
   u16 magic 0xBEEF` (the marker is present on every retail frame and
   enforced).

3. **The results pool** — and this is the trap: it is **deduplicated**.
   Frames may (and do) share results across sequences, so the pool is not
   per-sequence tileable; the correct validation is bounds-only —
   `resultOffset + resultSize(element)` must stay within the pool, which
   ends at the UAAT block when present, else at the section end. An SRT
   result is `u16 cell, u16 rotZ, u32 scaleX(fx32), u32 scaleY(fx32),
   s16 x, s16 y`; a translate result is `u16 cell, u16 pad, s16 x, s16 y`.

4. **UAAT block** (when `uaatOffset != 0`): `"TAAU", u32 size
   (= 0x10 + 0x10*nSeq + 8*nFrames), u16 nSequences, u16
   attrsPerFrame(1), u32 reserved(8)`, then a 0x0C record per sequence
   (`u16 frameCount` mirroring the sequence, `u16 0xBEEF`, two pointers),
   `nFrames` frame pointers, `nSeq` sequence attributes, and `nFrames`
   frame attributes, tiling the section end exactly. All four pointer
   arrays are **fully derived** — values are relative to the block body
   start (`uaatOffset + 8`), pointing respectively at `seqAttrs[i]`, at
   `frameSinglePtrs[frameStart(i)]`, and at `frameAttrs[j]` — and the
   parser validates every one. (nitrogfx's writer emits a different
   0x0C-per-sequence layout and does not round-trip retail files; the
   retail image wins.)

## LBAL — the label bank (shared)

Both formats' second section stores labels, but with a quirk: the body is a
**bare array of u32 string offsets with no count header**. The offsets are
relative to the *end* of the offset array itself, strictly increasing, first
always 0 — the count is derived by scanning while values remain plausible
(the SDK loader and nitrogfx do the same), and each label is NUL-terminated
ASCII (`CellAnime0`, `johto0`, …).

The two formats use labels differently:

- **NCER**: label count is independent of `nCells` (a/0/4/9 member 58 has
  224 cells and 16 labels: `johto0..7`, `kanto0..7`, …).
- **NANR**: label count **equals `nSequences` on every retail file** — one
  label names each sequence, in order. The parser enforces this.

## Where the pairs live

Sprite "sets" are grouped inside NARCs as runs of
(NCGR[, NCGR], NCLR, NCER, NANR) — e.g. `a/0/5/8` opens with one
four-member set (NCGR, NCLR, NCER, NANR) followed by five-member groups
(NCGR, NCGR, NCLR, NCER, NANR). The SDK's own default animation ships
loose in the NitroFS as
`data/clact_default.NANR` (one sequence, one frame, cell 0 for 4 ticks).
Follower (walk-behind) scene banks: `a/0/4/9` member 58. Bag UI:
`pbr/bag_gra.narc` member 0 (16 sequences, mixed Cell/SRT elements).
Poké Ball icons: `pbr/poke_icon.narc` member 3 (translate elements).

Inspect any of these with:

```text
apricorn-tools gfx hg_usa.nds a/0/5/8        # one line per member
apricorn-tools gfx hg_usa.nds a/0/5/8 2      # full NCER dump
```