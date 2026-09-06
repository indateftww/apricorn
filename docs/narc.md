# NARC — Nitro's archive format, as HeartGold uses it

Reference for `apricorn-core::formats::narc`, established in Phase 1 by
scanning every NARC in the retail US dump (`hg_usa.nds`). HeartGold keeps
almost all of its content in NARCs: the NitroFS's 43 `.narc` files, plus
**265 extensionless files** (`a/…`, `fielddata/…`) that are NARCs without
the name — 308 archives holding **56,689 members** in total. (One trap:
`poketool/personal/pms.narc` is *not* a NARC despite its extension —
always check the magic, which is what `formats::is_narc` is for.)

The member census is the game's content map, roughly:

| Member magic | Format | Count |
|--------------|--------|-------|
| `RGCN` | NCGR — character/tile graphics | 7,937 |
| `RLCN` | NCLR — palettes | 4,945 |
| `BMD0` | NSBMD — 3D models | 1,540 |
| `BTX0` | NSBTX — 3D textures | 1,146 |
| `RCSN` | NSCR — BG screen maps | 791 |
| `RECN` | NCER — cell (sprite frame) data | 608 |
| `RNAN` | NANR — sprite animations | 591 |
| `BCA0` | NSBCA — model animations | 285 |
| (raw) | binary tables (stats, encounters, text…) | ~10,000 |

699 members are zero-length.

## Container layout

Like every Nitro container, a NARC opens with a 0x10-byte header:

| Offset | Size | Field | HeartGold value |
|--------|------|-------|-----------------|
| 0x00 | 4 | magic `NARC` | `NARC` |
| 0x04 | 2 | byte-order mark | 0xFFFE |
| 0x06 | 2 | version | 0x0100 |
| 0x08 | 4 | total file size | always equals the file's length |
| 0x0C | 2 | header size | 0x10 |
| 0x0E | 2 | chunk count | 3 (a rare FNTI fourth chunk exists in some games; not here) |

Then exactly three chunks, in order, each `magic + u32 chunk_size` where
the size counts the chunk *including* its 8-byte header:

### BTAF — the member allocation table

```text
u32 magic 'BTAF'
u32 chunk_size
u16 member_count
u16 reserved (0)
(member_count × ) u32 start, u32 end    // relative to the GMIF body
```

**Do not assume contiguity or alignment.** 176 of HeartGold's NARCs
have gaps between members (alignment padding to 4 bytes) and 185 have
members with odd ends — `pbr/msg.narc`'s 624 text banks are packed
back-to-back with no padding at all. Treat BTAF as plain ranges: member
`i` is `gmif_body[start[i]..end[i]]`.

### BTNF — the filename table

Structurally the same directory + name-subtable scheme as the NitroFS
FNT (see `docs/nds-container.md`): an array of 8-byte directory records
(record 0 is the root; the root's third field holds the directory
count), with length-prefixed name records (`0x80` bit = directory)
referenced by offsets relative to the record-array base.

**Every HeartGold NARC has a root-only, empty BTNF (exactly 0x10
bytes)** — members are addressed by index alone, which is why the parser
exposes `file(id)` first. The empty-table convention is quirky: the
root's subtable offset is 4, pointing *inside* the root record itself,
where the top-file-id byte 0x00 conveniently reads as the end-of-table
marker. The parser implements the full named path (validated by
synthetic tests) but retail never exercises it.

### GMIF — the member data

`u32 magic 'GMIF' + u32 chunk_size` followed by the members, padded to
4-byte alignment with 0xFF *where the writer bothered to align at all*
(see BTAF above).

## Tools

```text
cargo run -p apricorn-tools -- narc hg_usa.nds pbr/growtbl.narc
```

prints each member's id, size, name (BTNF archives only), and the
ASCII of its first four bytes — usually the contained Nitro format's
reversed magic (`RGCN` = NCGR, `RLCN` = NCLR, `BMD0` = NSBMD, …).