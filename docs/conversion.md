# Conversion — raw formats → engine cache

`apricorn-tools convert <rom.nds> <out-dir>` is the Phase 1 conversion
step: it turns the ROM's raw Nitro formats into the engine's cached
formats, so the runtime loader stays fast and simple — a validated read
and a copy, no per-asset decoding. The raw formats are optimized for
the NDS's hardware (nibble-packed pixels, BGR555 colors, XOR-obfuscated
text, packed OAM registers); the cache carries exactly what the engine
consumes, decoded once at conversion time.

## Cache layout

```text
out/
└─ cache/
   ├─ cache.json                     # the manifest below
   ├─ overlay/
   │  └─ arm9/
   │     ├─ overlay_0000.bin         # all 129 overlays, the 127 BLZ
   │     └─ …                        #   images decompressed
   └─ nitrofs/                       # chunks at the source paths
      ├─ a/0/0/4/0.tiles             # NARC member: <narc>/<id>.<ext>
      ├─ a/0/2/7/829.text            # the script banks, decrypted
      ├─ data/dp_font.NCGR.tiles     # loose file: <path>.<ext>
      └─ …
```

- **Overlays** are BLZ-decompressed (backwards LZ77 — see below) when
  the overlay table's compstatic flag is set, copied as stored
  otherwise. Every decompressed overlay is exactly its table
  `raw_size` long.
- **NARC members** become `nitrofs/<narc path>/<id>.<ext>` — members
  are index-addressed, so the member id is the filename. A member whose
  first byte is the `0x10` magic is LZ77-10-compressed (see below): it
  is decompressed and the format sniffs run on the image. The
  manifest's `source` field records the original `path#id`.
- **Loose files** become `nitrofs/<path>.<ext>`.
- `<ext>` names the chunk kind: `tiles` / `pal` / `screen` / `cells` /
  `anim` / `text`.

## The chunk format

A chunk is a self-contained little-endian file:

```text
+0x00 4  b"APCH"
+0x04 2  format version (apricorn_core::cache::VERSION)
+0x06 2  chunk kind
+0x08 .. kind-specific payload
```

The six kinds and what they preserve:

| Kind | From | Payload |
|------|------|---------|
| `tiles` | NCGR | metadata (format, mapping, grid/CPOS, VRAM flag) and the pixels expanded 4bpp → 1 byte each |
| `pal` | NCLR | colors converted BGR555 → RGBA8 (`v<<3 \| v>>2` per channel), plus any PMCP patch indices verbatim |
| `screen` | NSCR | pixel dimensions, color mode, screen format, and the raw map entries |
| `cells` | NCER | per-OAM decoded records (signed X/Y, tile, palette, priority, shape, size, flips, mode, raw attrs) plus cell bounds, VRAM-transfer blocks, UCAT, labels |
| `anim` | NANR | sequences (label, loop point, play mode, element) with uniform 20-byte frame records |
| `text` | MAT | the bank's decrypted u16 units, message boundaries, and key |

The PMCP table is *not* a decompression: per `NNS_G2dLoadPaletteEx` it
is a partial-load patch — stored sub-palette *i* loads into VRAM slot
`indices[i]` of an already-loaded palette (see `docs/nitro-gfx.md`) —
so the chunk keeps the indices verbatim instead of inventing a
materialization.

Every chunk reader validates its payload structure exactly (the chunk
must tile to its last byte), so a truncated or corrupt cache is
rejected at load rather than misrendered.

## Verification

Every output — overlays, chunks, and the manifest itself — is written,
read back, and **byte-compared** before the run continues; every chunk
is additionally **re-parsed through the core cache reader**, so a cache
the tool produced is a cache the engine can load. The manifest is
written last and carries the ROM's SHA-1; re-running `convert` against
an existing cache with the same SHA-1 skips the rebuild, and because
the manifest lands only after every write has verified, a half-finished
cache is never mistaken for a complete one.

## Manifest

```json
{
  "cache_version": 1,
  "chunk_version": 1,
  "rom": { "sha1": "4fcded0e…", "size": 134217728 },
  "overlays": [
    { "id": 0, "path": "overlay/arm9/overlay_0000.bin", "compressed": true,
      "raw_size": 216448, "sha256": "b2f4e054…" }
  ],
  "chunks": [
    { "source": "a/0/0/4#0", "kind": "tiles",
      "path": "nitrofs/a/0/0/4/0.tiles", "sha256": "…" }
  ]
}
```

- `cache_version` — the manifest schema; `chunk_version` mirrors
  `apricorn_core::cache::VERSION` and bumps whenever any chunk layout
  changes. Loaders check both before reading.
- `rom` — identifies the source dump; the `sha1` is the cache's
  versioning key (rerunning against the same dump skips).
- `overlays` — one entry per overlay table record with the *output*
  bytes hashed (i.e. the decompressed image).
- `chunks` — one entry per chunk: the `source` (`path#id` for NARC
  members, the NitroFS path for loose files), the kind, the output
  `path`, and the SHA-256.

## Retail HeartGold census (pinned in `tests/convert_hg.rs`)

- 19,306 chunks totaling 110,492,231 bytes: **9,458 tiles, 4,953
  palettes, 1,500 screens, 979 cells, 963 anims, 1,453 text** — plus 129
  overlays, 127 of them BLZ-decompressed.
- Beyond the raw members these take in the LZ77-10 population (see
  below): 1,509 NCGRs, 707 NSCRs, 367 NCERs and 367 NANRs ship as
  compressed images behind the `0x10` magic (no NCLR ever does).
- The tiles figure is 24 short of the convertible total, for two
  reasons. `data/dp_areawindow.NCGR` is a DP leftover with a corrupt
  container header (zero BOM, file and section sizes each under-declared
  by 8 bytes — its tile data is complete, but nothing short of weakening
  three independent validations would let it through). And 23 LZ77-10
  images in `a/0/0/7` each omit the trailing CPOS section their own
  container header still lists — the image is the file minus its last
  16 bytes, a retail inconsistency the game tolerates because NNS's
  loaders never validate the container; the strict parsers skip them.
- The text figure is 829 banks from the script archive `a/0/2/7` plus
  624 further genuine MAT banks living in `pbr/msg.narc` (the PBR
  battle-subset message set). Banks whose message count is 0x0010 start
  with the `0x10` magic too; the converter's LZ77-10 sniff falls through
  to the MAT sniff when the stream path yields nothing, so all 21 of
  them still convert.
- Every source file behind these figures also re-serializes byte-exact
  through its parser's `to_bytes()` — the round-trip guard; see
  `docs/roundtrip.md`.

## BLZ, the ARM9 compression

BLZ (backwards LZ77, pret's `tools/compstatic` scheme) compresses the
overlays and the ARM9 binary itself, which pret builds as `main_lz` via
`$(COMPSTATIC) -9 -c -f`: a "compressed static" image whose plain
head (0x41BA bytes — the secure area plus the crt0 stub that
decompresses the rest at load time) runs straight into the payload,
with decompressed size 0x111EF8. The ROM header's `arm9.size`
(0xBA314) is the *stored* size.
A stored image of `L` bytes ends with an 8-byte footer:

```text
u32 (addLen << 24) | (L - srcOff)
u32 tailLen == S - L          # S = raw_size
```

The payload between `srcOff` and `payEnd = L - addLen` is LZ77 written
backwards: decoded from its end toward its start, writing the image
from its end toward its start. Full format reference:
`crates/apricorn-core/src/nds/blz.rs`.

Worked example — overlay 0 (`overlay/arm9/overlay_0000.bin`):

```text
stored:   L = 129744 bytes (0x1FAD0), at ROM offset 783872
raw_size: S = 216448 bytes (0x34D80)
footer:   w0 = 0x0B01FACF, w1 = 86704 (0x152B0)
```

`tailLen` checks out (216448 − 129744 = 86704); `w0 & 0xFFFFFF` =
0x1FACF, so `srcOff` = 1 — a single plain head byte (the stored image
starts `f8 8d 1d 90`, and 0xF8 copies verbatim to image byte 0);
`addLen` = 0x0B, so `payEnd` = 0x1FAC5, followed by three 0xFF padding
bytes and the 8-byte footer. The decoder walks the flags groups from
`payEnd` down to the head byte, producing the 216,448 raw bytes whose
SHA-256 the manifest records:
`b2f4e05409c8dd077aecc065cdc9da6b16bf41adc260ad8af45e013029f644ce`.

## LZ77-10, the asset compression

The *forward* LZ77-10 variant (the format the SDK's
`MI_UncompressLZ8`/`MI_UncompressLZ16` decoders accept) compresses
selected NARC members — those pret requests with `isCompressed=TRUE`.
HeartGold's examples live in the intro movie's `a/2/6/2`
(`gs_opening`): members 4–12 and 14 carry the `0x10` magic, covering
the copyright-beat NCGR/NSCR pair and the Game Freak logo's
char/screen files. A stream is a 4-byte header — the `0x10` magic plus
a u24 decompressed size — followed by groups of one flags byte and up
to 8 codes (flags consumed MSB-first: bit 7 is the first token):
flag 0 = literal, flag 1 = a two-byte match
`[b1][b2]` with `len = (b1 >> 4) + 3` (3..=18) and
`disp = ((b1 & 0xF) << 8 | b2) + 1` (1..=4096), copying from
`write - disp` forward so a match may read bytes it is itself writing
(RLE runs, `disp == 1`). Decoding stops when the declared image is
full; trailing padding is ignored. Full format reference:
`crates/apricorn-core/src/nds/lz10.rs`.

The converter sniffs the magic on each raw member and decompresses
once before the format sniffs run (a stream decompresses to strictly
more bytes than it stores, so a nested sniff cannot loop). The `0x10`
sniff runs *before* the MAT heuristic — a small LZ77-10 image can
mimic MAT header fields — and a decompression failure means "not this
format", the same convention the MAT sniff uses for its own misses.

The population is ROM-wide, not just the intro movie: 4,287 members
carry the `0x10` magic and 4,221 decode to convertible images. The
ambiguity cuts both ways — a genuine MAT bank whose message count is
0x0010 also starts with `0x10` — so when the stream path yields nothing
the sniff falls back to the MAT check on the original bytes. And a
sniff hit on a decoded image that then fails its parse is not a
corruption to abort on but a retail inconsistency the game tolerates
(NNS's loaders never validate the container): most notably 23 `a/0/0/7`
NCGRs whose image drops the trailing CPOS section the container header
still lists. Those members are skipped, census-pinned; parse failures
on members whose format magic is directly present still abort the
conversion.