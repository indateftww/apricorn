# Extraction — ROM → unpacked tree + manifest

`apricorn-tools extract <rom.nds> <out-dir>` unpacks a ROM into a flat,
verifiable asset tree plus a JSON manifest. It is the Phase 1 exit
criterion: `apricorn-tools extract hg_usa.nds out/` produces a full,
verified, engine-loadable asset tree.

## Tree layout

```text
out/
├─ manifest.json              # the index below
├─ header.bin                 # raw 0x4000-byte cartridge header
├─ arm9.bin                   # header.arm9 region, as stored
├─ arm7.bin                   # header.arm7 region, as stored
├─ overlay/
│  └─ arm9/
│     ├─ overlay_0000.bin     # raw overlay bytes, one file per table entry
│     └─ …
└─ nitrofs/                   # the FNT tree at its original paths
   ├─ a/…  data/…  dwc/…  fielddata/…  msgdata/…
   └─ pbr/…  poketool/…  tel/…
```

Notes:

- **Overlays are stored raw, exactly as in the ROM.** 127 of retail
  HeartGold's 129 overlays are **headerless LZ77** (no `0x10` magic —
  the compressed bytes begin with a flag byte, and the overlay table's
  `raw_size` is the decompressed length). The manifest's `compressed`
  flag says which; decompression belongs to the conversion step, not
  extraction.
- **The NitroFS tree is namespaced under `nitrofs/`** so overlay/ and the
  binaries can never collide with a game directory name.
- The NitroFS root's `""` path is the `nitrofs/` directory itself; empty
  directories (if any) are enumerated in the manifest's `directories`.
- NitroFS file contents are exactly the FAT-referenced bytes — zero for
  an empty FAT entry.
- The ARM7 overlay table is empty in retail HeartGold and not extracted
  (the parser exposes only the ARM9 table).

## Verification

Every file — binaries, overlays, NitroFS files, and the manifest itself —
is written, read back, and **byte-compared** before the tool reports
success; a mismatch aborts with `extract: <path>: readback differs from
the source bytes`. The manifest's SHA-256s are computed from the source
ROM slices, so a re-hash of the tree also verifies the tree against the
manifest.

## Manifest

```json
{
  "manifest_version": 1,
  "rom": {
    "title": "POKEMON HG",
    "game_code": "IPKE",
    "maker_code": "01",
    "unit_code": 0,
    "size": 134217728,
    "sha1": "4fcded0e2713dc03929845de631d0932ea2b5a37",
    "header_crc_ok": true,
    "logo_crc_ok": true
  },
  "binaries": [
    { "path": "header.bin", "offset": 0, "size": 16384, "sha256": "3fbd596a…" },
    { "path": "arm9.bin",   "offset": 16384, "size": 762644, "sha256": "5eeaa2dc…" },
    { "path": "arm7.bin",   "offset": 3109888, "size": 161496, "sha256": "746355ed…" }
  ],
  "directories": [ "", "a", "data", "…" ],
  "files": [
    { "path": "nitrofs/a/0/2/8", "fat_id": 157, "offset": 40134144,
      "size": 41524, "sha256": "f02b453a…" }
  ],
  "overlays": [
    { "id": 0, "path": "overlay/arm9/overlay_0000.bin",
      "ram_address": 35434240, "bss_size": 4992, "sinit_start": 35726328,
      "sinit_end": 35726332, "fat_id": 0, "compressed": true,
      "offset": 784384, "size": 129744, "raw_size": 216448,
      "sha256": "a3f1e031…" }
  ]
}
```

- `manifest_version` — bumped whenever the schema changes; loaders check
  it before reading. (All numeric fields are decimal — JSON has no hex
  literals; offsets that read naturally as hex, like RAM addresses, are
  still plain integers.)
- `rom` — identifies the source dump (the `sha1` pins the exact image),
  which is the "versioning" of the tree: two dumps that differ produce
  different manifests, and the conversion step can key its cache on
  `rom.sha1`.
- `binaries` / `files` / `overlays` — one entry per extracted file, each
  with the tree-relative `path`, the `offset` it came from in the ROM
  image, its `size`, and its lowercase-hex SHA-256. File and overlay
  entries also keep their `fat_id` (the FAT is the shared id space:
  overlays occupy 0..0x81, NitroFS files 0x81..513 in HeartGold).
- Overlay entries carry the full overlay-table record (`ram_address`,
  `bss_size`, static-init bounds, `compressed`, `raw_size`) so the loader
  never needs to re-read the ROM.

The manifest is emitted pretty-printed (2-space indent) in a fixed
field order, so it diffs cleanly between runs and dumps.

## Retail HeartGold census (pinned in `tests/extract_hg.rs`)

- 46 NitroFS directories, 384 files; 129 ARM9 overlays (127 LZ-compressed,
  35 and 124 plain); 3 binaries. The tree totals 126,490,353 bytes
  (120.6 MiB).
- 513 FAT entries = 129 overlays + 384 files; the NitroFS tree is
  rooted at the eight root directories `a`, `data`, `dwc`, `fielddata`,
  `msgdata`, `pbr`, `poketool`, `tel`.
- The two SDATs land at
  `nitrofs/data/sound/gs_sound_data.sdat` (8,660,096 bytes) and
  `nitrofs/pbr/sound_data.sdat` (7,484,768 bytes).