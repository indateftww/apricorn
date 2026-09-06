# The NDS container, as HeartGold uses it

Reference for `apricorn-core::nds`, established in Phase 1 by decoding the
retail US dump (`hg_usa.nds`, SHA-1 `4fcded0e…`) and reconciling it with
pret/pokeheartgold's build inputs. This is what our parser reads; the values
below are HeartGold's actual ones.

## Cartridge header (first 0x4000 bytes)

| Offset | Size | Field | HeartGold value |
|--------|------|-------|-----------------|
| 0x00 | 12 | game title | `POKEMON HG` |
| 0x0C | 4 | game code | `IPKE` |
| 0x10 | 2 | maker code | `01` |
| 0x20 | 4 | ARM9 ROM offset | 0x00004000 |
| 0x24 | 4 | ARM9 entry address | 0x02000800 |
| 0x28 | 4 | ARM9 RAM address | 0x02000000 |
| 0x2C | 4 | ARM9 size | 0x000BA314 |
| 0x30 | 4 | ARM7 ROM offset | 0x002F7400 |
| 0x34 | 4 | ARM7 entry address | 0x02380000 |
| 0x38 | 4 | ARM7 RAM address | 0x02380000 |
| 0x3C | 4 | ARM7 size | 0x000276D8 |
| 0x40 | 4 | FNT offset | 0x0031EC00 |
| 0x44 | 4 | FNT size | 0x00000B56 |
| 0x48 | 4 | FAT offset | 0x0031F800 |
| 0x4C | 4 | FAT size | 0x00001008 |
| 0x50 | 4 | ARM9 overlay table offset | 0x000BE400 |
| 0x54 | 4 | ARM9 overlay table size | 0x00001020 |
| 0x68 | 4 | banner offset | 0x00320A00 |
| 0x6C | 2 | secure-area CRC | — |
| 0x80 | 4 | application end offset (ROM size) | 0x078C763C |
| 0x84 | 4 | ROM header size | 0x00004000 |
| 0xC0 | 156 | Nintendo logo | — |
| 0x15C | 2 | logo CRC | — |
| 0x15E | 2 | header CRC | — |

Note the ARM9/ARM7 *size* fields at 0x2C/0x3C — easy to miss, and without
them the overlay-table offset (0xBE400) doesn't reconcile with the ARM9 end
(0x4000 + 0xBA314 = 0xBE314; 0xEC of alignment padding follows).

### CRCs

Both use ndstool's `CalcCrc16`: reflected polynomial 0xA001 (normal form
0x8005), init 0xFFFF, no final XOR — the CRC-16/MODBUS family (check value
for `"123456789"` is 0x4B37).

- **Logo CRC**: over `rom[0xC0..0x15C]` (the 156 logo bytes).
- **Header CRC**: over `rom[0x00..0x15E]` — i.e. everything up to and
  including the logo CRC, but excluding itself.

Both verify against the retail dump, confirming the ranges.

## Nitro filesystem

### FNT — File Name Table

Begins with an array of 8-byte directory records, indexed by directory ID
(`0xF000 | index`; the root is `0xF000`):

```text
u32 entry_start    // offset of this directory's name subtable, from FNT base
u16 top_file_id   // FAT id of the first file listed in the subtable
u16 parent         // parent directory ID — except in the root, where
                   // this field holds the total directory count (46)
```

HeartGold's FNT is 0xB56 bytes: 46 records (0x170 bytes) followed by the
name subtables. Name subtables are length-prefixed records:

- `0x00` — end of this subtable.
- `0x01..=0x7F` — file: name length, then the name; consumes one FAT id,
  starting at the record's `top_file_id`.
- `0x80..=0xFF` — directory: name length minus 0x80, the name, then a
  `u16` directory ID pointing at another record.

### FAT — File Allocation Table

Flat array of `(u32 start, u32 end)` pairs indexed by file ID. HeartGold:
0x1008 bytes = **513 entries**.

**Overlays occupy the first 129 FAT ids** (0..129), which is why the root
directory's `top_file_id` is 0x81 — NitroFS file ids start after the
overlays.

## ARM9 overlays

The overlay table (0x50/0x54 in the header) is an array of 0x20-byte
entries, in overlay-number order:

```text
u32 id                  // 0, 1, 2, … (HeartGold: 0..129)
u32 ram_address
u32 raw_size            // uncompressed (RAM) size
u32 bss_size
u32 sinit_start
u32 sinit_end
u32 fat_id             // index into the FAT (== the overlay id in HeartGold)
u32 compressed_size     // bit 24 = LZ77-compressed, bits 0..24 = compressed size,
                        // bit 25 = anti-piracy HMAC computed
```

(GBATEK describes the compression flag as bit 31 of the size word, but
retail HeartGold follows pret's `tools/compstatic` convention instead: the
flag is bit 24 of the compressed-size word, whose low 24 bits hold the
stored size. The ROM is the spec — 127 of the 129 overlays are compressed,
their compressed sizes all matching the flag convention and none setting
bit 31.)

Compressed overlays are **BLZ** (backwards LZ77, the `tools/blz` /
`compstatic` scheme pret uses): no `0x10` magic or length word — the
compressed bytes begin directly with a flag byte, and `raw_size` gives
the decompressed length. See `crates/apricorn-core/src/nds/blz.rs` and
`docs/conversion.md`.

HeartGold has 129 ARM9 overlays, only 35 and 124 (tiny) stored plain.
These are the game's modules: overlay 12 is battle (~64 KiB of it is ARM
asm in pret's decompilation), overlay 1 holds the script commands, etc.

## Scale: the ROM is a shell

The NitroFS looks tiny — 46 directories, 384 files — while pret's unpacked
`files/` tree counts 12,616 files. The difference is that the game's actual
content (maps, Pokémon data, graphics, scripts, text) lives inside **NARC
archives**: single NitroFS files that are themselves filesystems. The NARC
parser is done — see `docs/narc.md` (308 archives, 56,689 members). The
NCGR/NCLR/NSCR graphics formats inside the members are done too
(`docs/nitro-gfx.md`); NSBMD/SDAT and the remaining member formats are the
Phase 1 slices still open.

Relatedly, pret's `files/` tree is not a 1:1 image of the NitroFS: some
files are stored as rebuildable sources (message banks as JSON, NARCs
unpacked). 312 of HeartGold's 384 NitroFS files do have byte-identical
same-path counterparts there, which the integration test
(`crates/apricorn-core/tests/nds_hg.rs`) compares on every run.

## Tools

```text
cargo run -p apricorn-tools -- verify hg_usa.nds   # facts + CRC + SHA-1
cargo run -p apricorn-tools -- list hg_usa.nds     # every file with FAT id
```