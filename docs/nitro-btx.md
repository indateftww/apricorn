# BTX / NSBTX — the NNS-G3D texture archive, as HeartGold uses it

Reference for `apricorn-core::formats::btx`, established in Phase 1 by
scanning every NSBTX in the retail US dump (`hg_usa.nds`) and
cross-checking against the NitroSDK headers bundled with pret/pokeheartgold
(`NNSG3dResFileHeader`, `NNSG3dResTex`, `NNSG3dResDict` — the runtime
structs the G3D engine unpacks these files straight into) and GBATEK's
`BTX0` notes. **The retail image is the oracle**: every invariant below is
pinned with zero violations across all 1,157 files, and enforced at parse
time.

Census (all counts pinned by `crates/apricorn-core/tests/btx_hg.rs`, which
parses every BTX in the ROM):

| What | Count |
|------|-------|
| BTX files | 1,157 — 1,146 NARC members + 11 loose NitroFS `.nsbtx` |
| Texture entries | 14,735 |
| Palette entries | 7,729 |
| PLTT16 (4bpp) textures | 12,794 |
| PLTT4 (2bpp, 4-color) textures | 1,617 |
| A5I3 textures | 169 |
| A3I5 textures | 145 |
| PLTT256 (8bpp) textures | 10 |
| color0-transparent entries | 10,411 (4,323 use color 0) |
| PLTT4-flagged files | 216 of 1,157 |

The loose files live in `data/`: `dun_sea`, `lake_anim`, `miniasahamabe`,
`miniasasea`, `minihamabe`, `minimum`, `minirhana`, `t3_fl_b/p/r/y` (all
`.nsbtx`). Most BTX members sit in the big field-graphics archives
(`a/0/4/4` alone holds 100+).

## Not a Nitro container

The 2D formats (NCGR/NCLR/NSCR/NCER/NANR — see `docs/nitro-gfx.md` and
`docs/nitro-sprite.md`) are NNS-G2D containers with *reversed* magics and
a section list. A BTX is an NNS-G3D resource file: the magic is the
literal `BTX0`, and after the 0x10-byte header comes a **block-offset
table** (u32 per block), not the blocks themselves:

| Offset | Size | Field | HeartGold value |
|--------|------|-------|-----------------|
| +0x00 | 4 | signature | `BTX0` (literal — not reversed) |
| +0x04 | 2 | byte-order mark | 0xFEFF |
| +0x06 | 2 | version | **always 0x0001** (not 0x0100!) |
| +0x08 | 4 | fileSize | always == file length |
| +0x0C | 2 | headerSize | always 0x10 |
| +0x0E | 2 | dataBlocks | **always 1** |
| +0x10 | | blockOffset[0] | **always 0x14** (= 0x10 + 4·blocks) |

The single block is `TEX0`, and it ends the file exactly (block size ==
fileSize − 0x14, every file).

## TEX0 (`NNSG3dResTex`)

Offsets below are relative to the TEX0 block start (file offset 0x14).
The block is `[8-byte header][texInfo][tex4x4Info][plttInfo][texture
dictionary][palette dictionary][texture data][palette data]`, and each
region starts exactly where the previous ends — the file is one perfect
tiling, verified on all 1,157 files:

| Offset | Size | Field | HeartGold value |
|--------|------|-------|-----------------|
| +0x00 | 4 | kind | `TEX0` |
| +0x04 | 4 | size | == TEX0 end == file end |
| +0x08 | 4 | texInfo.vramKey | always 0 |
| +0x0C | 2 | texInfo.sizeTex | texture-data bytes / 8 (16–70,656 bytes) |
| +0x0E | 2 | texInfo.ofsDict | always 0x3C |
| +0x10 | 4 | texInfo.flag, dummy | always 0 |
| +0x14 | 4 | texInfo.ofsTex | == palette-dict end (derived) |
| +0x18 | 4 | tex4x4Info.vramKey | always 0 |
| +0x1C | 2 | tex4x4Info.sizeTex | **always 0** — no COMP4x4 textures exist |
| +0x1E | 2 | tex4x4Info.ofsDict | always 0x3C (mirrors texInfo) |
| +0x20 | 4 | tex4x4Info.flag | always 0 |
| +0x24 | 4 | tex4x4Info.ofsTex | **garbage** — differs from texInfo.ofsTex on all 1,157 files; never validated |
| +0x28 | 4 | tex4x4Info.ofsTexPlttIdx | garbage likewise |
| +0x2C | 4 | plttInfo.vramKey | always 0 |
| +0x30 | 2 | plttInfo.sizePltt | palette-data bytes / 8 (16–2,784 bytes) |
| +0x32 | 2 | plttInfo.flag | bit 15 = `NNS_G3D_RESPLTT_USEPLTT4` (216 files); all other bits zero |
| +0x34 | 2 | plttInfo.ofsDict | == texture-dict end (derived) |
| +0x36 | 2 | plttInfo.dummy | always 0 |
| +0x38 | 4 | plttInfo.ofsPlttData | == ofsTex + sizeTex·8 (derived) |

The 4x4 block's garbage pointers are the one place the SDK's writer left
uninitialized values in every retail file — the G3D engine never reads
them when `tex4x4Info.sizeTex` is 0.

## NNSG3dResDict — the name dictionary

Both the texture and the palette dictionary are
`NNSG3dResDict`s — the 3D engine's binary-search-by-name structures.
Texture dictionaries use `sizeUnit` 8; palette dictionaries 4. All
offsets are dictionary-relative:

| Offset | Size | Field | HeartGold value |
|--------|------|-------|-----------------|
| +0x00 | 1 | revision | always 0 |
| +0x01 | 1 | numEntry | 1–126 per dictionary |
| +0x02 | 2 | sizeDictBlk | fully derived (below) |
| +0x04 | 2 | dummy | always 8 (the node-array offset) |
| +0x06 | 2 | ofsEntry | always == 12 + 4·numEntry |
| +0x08 | | trie nodes | (numEntry+1) × 4 bytes; contents unvalidated |
| +ofsEntry | 4 | entry header | one `u16 sizeUnit, u16 ofsName` for the whole dictionary |
| +ofsEntry+4 | | entries | numEntry × sizeUnit packed payload bytes |
| +ofsEntry+ofsName | | names | numEntry × 16-byte slots |

Three subtleties the SDK headers don't spell out, all pinned empirically:

- The **entry header exists once**, before the packed payloads — entry
  `i`'s payload is at `ofsEntry + 4 + sizeUnit·i`. (GBATEK's per-entry
  `[sizeUnit][ofsName]` reading would leave no room for the names.)
- `ofsName` is always exactly `4 + sizeUnit·numEntry`, and
  `sizeDictBlk` always `ofsEntry + ofsName + 16·numEntry` — the whole
  dictionary is fully derived from `numEntry` and `sizeUnit`.
- Name slots are fixed **16-byte** fields, NUL-padded but possibly
  completely full: `a/0/4/4` member 27's palette 87 is
  `fh01_19tuta02_pl` — 16 characters, no terminator. Names are unique
  within each dictionary on every retail file.

Texture entry payload (8 bytes), two u32 words. Word 0 is
`TEXIMAGE_PARAM`:

| Bits | Meaning | HeartGold value |
|------|---------|-----------------|
| 0–15 | image offset / 8 (texture-data-relative) | any; 343 files share offsets |
| 16–19 | — | always 0 |
| 20–22 | width exponent (width = 8 « exp) | 0–5 (8–256 px) |
| 23–25 | height exponent | 0–5 |
| 26–28 | `GXTexFmt` | 1 A3I5, 2 PLTT4, 3 PLTT16, 4 PLTT256, 6 A5I3 — never 0/5/7 |
| 29 | color0 transparent | 10,411 of 14,735 entries |
| 30–31 | — | always 0 |

Word 1: bits 0–10 width, bits 11–21 height (both **always equal
8 « exponent**, zero violations in 14,735 entries — heights of 128/256
set bits 18–19 of this word), bit 31 always set, bits 22–30 always zero.

Palette entry payload (4 bytes): a `u16` base (bits 0–12 = palette
offset / 8, palette-data-relative; bits 13–15 always zero) plus a `u16`
of unknown meaning (GBATEK: "usually 0, sometimes 1") — 0 on 6,117
retail entries, 1 on 1,612, always one of the two. It is exposed raw by
the parser and not validated.

## Texture spans and palette sizes

Texture offsets and span ends are both validated. PLTT4 means four colors
at **2 bits per pixel**; all 14,735 texture spans fit their data areas.
The earlier report of 51 truncated shadow textures was a parser error
that treated PLTT4 as 4bpp. The bedroom renderer exposed and corrected it.

Per-palette sizes are not stored in the file, so entries expose the data
from their base to the next distinct base. Four colors need 8 bytes and
16 colors need 32 bytes; archive padding can make a slice larger. Texture
materials bind the palette used for decoding.

## Where BTX files live

`apricorn-tools gfx` summarizes a NARC's BTX members (one line each) and
dumps a member — or a loose NitroFS `.nsbtx` — in full:

```sh
apricorn-tools gfx hg_usa.nds a/0/7/0 30   # detail dump of one member
apricorn-tools gfx hg_usa.nds data/dun_sea.nsbtx
```

Sources: the retail image (the oracle), pret/pokeheartgold's bundled SDK
headers (`lib/include/nnsys/g3d/binres/res_struct.h`,
`res_struct_accessor_inline.h`, `lib/include/nitro/gx/g3.h`), and
GBATEK's BTX0/NSBTX page.