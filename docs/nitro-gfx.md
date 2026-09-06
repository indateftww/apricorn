# NCGR / NCLR / NSCR — the 2D graphics formats, as HeartGold uses them

Reference for `apricorn-core::formats::{ncgr, nclr, nscr}`, established in
Phase 1 by scanning every member of every NARC in the retail US dump
(`hg_usa.nds`) and cross-checking against the NitroSDK headers bundled with
pret/pokeheartgold (`NNSG2dCharacterData`, `NNSG2dPaletteData`,
`NNSG2dScreenData`). The game loads these files by unpacking them straight
into those SDK structs (`NNS_G2dGetUnpacked*Data` in `gf_gfx_loader.c`), so
the on-disk field order *is* the SDK struct layout.

Census (all counts pinned by `crates/apricorn-core/tests/gfx_hg.rs`, which
parses every member in the ROM):

| Format | Magic | Members | Notes |
|--------|-------|---------|-------|
| NCGR — character (tile) sheets | `RGCN` | 7,937 | 7,888 4bpp / 49 8bpp |
| NCLR — palettes | `RLCN` | 4,945 | see the fmt quirks below |
| NSCR — BG screen maps | `RCSN` | 791 | 772 text screens |

## The shared NNS container header

All three open with the same 0x10-byte header layout as a NARC, but the
byte-order mark differs: the NNS graphics containers store **0xFEFF**
(bytes `FF FE`), where a NARC stores 0xFFFE (`FE FF`).

| Offset | Size | Field | HeartGold value |
|--------|------|-------|-----------------|
| 0x00 | 4 | magic | `RGCN` / `RLCN` / `RCSN` |
| 0x04 | 2 | byte-order mark | 0xFEFF |
| 0x06 | 2 | version | 0x0100 (a few NCGRs 0x0101) |
| 0x08 | 4 | total file size | always equals the file's length |
| 0x0C | 2 | header size | 0x10 |
| 0x0E | 2 | section count | 1 or 2 |

Then that many sections, each `reversed magic + u32 size` where the size
counts the section *including* its 8-byte header. Sections tile the file
exactly; the parser (`formats::nitro_sections`) enforces all of this.

## NCGR — character data (`RGCN` → CHAR `RAHC`, optional CPOS `SOPC`)

The CHAR section body is a serialized `NNSG2dCharacterData`:

| Offset | Size | Field | Meaning |
|--------|------|-------|---------|
| +0x00 | 2 | `H` | tile rows; 0xFFFF = none (linear OBJ data) |
| +0x02 | 2 | `W` | tile columns; 0xFFFF = none |
| +0x04 | 4 | `fmt` | `GXTexFmt`: 3 = 4bpp, 4 = 8bpp |
| +0x08 | 4 | `mapping` | `GXOBJVRamModeChar` (see below) |
| +0x0C | 4 | `charFmt` | low byte = character format; bit 8 = VRAM-transfer flag |
| +0x10 | 4 | `szByte` | tile data size in bytes |
| +0x14 | 4 | `pRawData` offset | always 0x18 |

followed by `szByte` bytes of tile data. BG data is **linear** (row-major),
not GBA 8×8-tiled; the game feeds it to the 2D engine directly.

`GXOBJVRamModeChar` values are the packed `GX_DISPCNT` register bits the
SDK writes (OBJMAP bit 4, extended-OBJ size bits 20-21), which is why the
1D modes look odd on disk:

| Raw | Meaning | HeartGold count |
|-----|---------|-----------------|
| 0 | 2D mapping | 4,928 |
| 0x10 | 1D, 32K boundary | 2,539 |
| 0x00100010 | 1D, 64K | 197 |
| 0x00200010 | 1D, 128K | 239 |
| 0x00300010 | 1D, 256K | 34 |

Census facts: 4,928 files declare a grid (`H`/`W` set — always 2D mapping,
and `szByte` is always exactly `H×W×tile_size`); 3,009 store linear OBJ
data (`H = W = 0xFFFF`, always 1D mapping); 860 files carry a CPOS section
(`u32 0, u16 W, u16 H` — note **width first**, unlike the CHAR struct's
height-first), which always restates the CHAR grid dims and only appears
on grid files. `charFmt` takes exactly three values: 0 (3,594 files), 1
(4,186), and 0x100 = format 0 + the VRAM-transfer flag (157; pinned via
pret's `obj_char_transfer.c:399`).

## NCLR — palettes (`RLCN` → PLTT `TTLP`, optional PMCP `PMCP`)

The PLTT section body is a serialized `NNSG2dPaletteData`:

| Offset | Size | Field | Meaning |
|--------|------|-------|---------|
| +0x00 | 4 | `fmt` | see the quirks below |
| +0x04 | 4 | `bExtendedPlt` | 1 = extended-palette bank (115 files) |
| +0x08 | 4 | `szByte` | logical palette size the game consumes |
| +0x0C | 4 | `pRawData` offset | always 0x10 |

followed by BGR555 color bytes. The PMCP body is a serialized
`NNSG2dPaletteCompressInfo`: `u16 numPalette`, `u16 pad 0xBEEF`,
`u32 table offset (always 8)`, then `numPalette` u16 indices — the VRAM
slot each stored sub-palette loads into (a `NNS_G2dLoadPaletteEx`-style
patch load; see the quirk below).

Two quirks that a naive strict parser would reject, both enforced by the
parser instead:

- **Palette compression** (193 files, all of them with a PMCP section):
  `szByte` exceeds the bytes stored. The PLTT section carries only the
  sub-palettes actually used, and the PMCP table names each one's VRAM
  destination slot — a patch load, not decompression: the game loads
  stored sub-palette *i* into slot `table[i]` (`NNS_G2dLoadPaletteEx`
  consumes both) and leaves the rest of the logical palette space
  untouched. A typical compressed file stores a single 16-color palette
  (32 bytes) with `szByte` 480, destined for slot 8 or 9.
  `Nclr::is_compressed` detects the case. Three further files have
  `szByte` **smaller** than the stored bytes (including two placeholders
  with `szByte = 0`); the game just consumes less than is stored, so the
  parser treats `szByte` as advisory (`Nclr::logical_size`) and never
  cross-checks it against the section size.
- **The `0x000A0004` fmt flag** (2,152 files, all in the follower-sprite
  archives `pbr/pokegra.narc`, `pbr/otherpoke.narc`, `a/0/0/4`, `a/1/1/4`):
  the high half of the fmt field carries `0x000A`. Every one of these
  stores an ordinary 16-color, 32-byte palette paired with a 4bpp NCGR —
  the low byte (4, `GX_TEXFMT_PLTT256`) does *not* apply. The game's own
  code only ever compares `fmt == GX_TEXFMT_PLTT256` exactly
  (pret's `obj_pltt_transfer.c:208`), so these take the 16-color path.
  The parser decodes them as 4bpp and keeps the raw field in
  `Nclr::fmt_raw`. What the `0x0A` half means is still unpinned.

The remaining 2,448 files store fmt 3 (4bpp) and 345 store fmt 4 (8bpp);
748 files have a PMCP section, which always pairs with the 0xBEEF pad and
the exact size formula `8 + 8 + 2×numPalette`. The table is the identity
(each stored palette *i* → slot *i*) on 605 of those 748; the remaining
143 redirect — the cache chunk keeps the indices verbatim, so the engine
applies the same patch load the game does.

## NSCR — screen maps (`RCSN` → SCRN `NRCS`)

A single section whose body is a serialized `NNSG2dScreenData`:

| Offset | Size | Field | Meaning |
|--------|------|-------|---------|
| +0x00 | 2 | `screenWidth` | in PIXELS (256 px, not 32 tiles) |
| +0x02 | 2 | `screenHeight` | in pixels |
| +0x04 | 2 | `colorMode` | `GX_BG_COLORMODE_16` (0) / `_256` (1) |
| +0x06 | 2 | `screenFormat` | 0 = text BG |
| +0x08 | 4 | `szByte` | map entry bytes that follow |

followed immediately by the entries — no offset field. 772 files are
standard text screens (`screenFormat` 0): `width/8 × height/8` little-
endian u16s, each `tile | palette << 12 | hflip 0x400 | vflip 0x800`.

Variants (accepted, meaning unpinned until Phase 3's hardware work):
`colorMode` 1 on 15 files (256-color) and 2 on 17 files with no SDK enum
counterpart; `screenFormat` 1 on 10 files and 2 on 9 files (rotation-
screen variants) — and notably the ten `screenFormat == 1` files carry
**u8** entries, half the u16 formula. `szByte` is therefore authoritative
over the dimensions; the parser validates `szByte == section size − 0x14`
and that the pixel dimensions are whole tiles, nothing more. A 256×192
full-screen text map (1,536 entry bytes) exists for the spot checks.

## Tools

```
cargo run -p apricorn-tools -- gfx hg_usa.nds a/0/0/4        # one line per member
cargo run -p apricorn-tools -- gfx hg_usa.nds a/0/0/4 4      # every field of member 4
```