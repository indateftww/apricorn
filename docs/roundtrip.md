# Round-trip tests — re-serialize and byte-compare

Phase 1's parser guard: every format parser pairs with a `to_bytes()`
serializer that rebuilds the file from the parsed fields, and
`tests/roundtrip_hg.rs` re-serializes **every instance of every format
in the retail ROM and byte-compares against the original bytes**.

A parser can be subtly wrong in ways a "does it parse" test never
catches: a field read from the wrong offset, a section dropped, an entry
decoded with an off-by-one. The round trip catches all of them — if the
parser misunderstood even one byte it read, the rebuilt file differs
from the original and the test fails. A byte-identical rebuild is proof
the parse consumed and understood the whole file.

`to_bytes()` is deliberately infallible: it cannot *validate*, only
re-derive. All validation lives in `parse()`, so the pair is a genuine
two-way guard.

## What is re-derived vs. retained verbatim

The design rule: **re-derive everything the writer derived** (so the
round trip genuinely exercises the parser's understanding), and **retain
verbatim only true writer artifacts** — bytes the SDK's writer chose in
a way no parser could reconstruct:

- **NCER / NANR LBAL label tables** — re-derived. The offset table is a
  running sum of label lengths; the pinned retail scan confirmed every
  file's strings tile exactly to the last label's NUL.
- **NCER OAM padding** — re-derived as zeros. All 162 padded retail
  files (and every synthetic case) have zero padding after the OAM
  array.
- **NANR KNBA offsets / UAAT placement** — re-derived (`resultOffset ==
  frameOffset + 8·nFrames`, per-sequence `frameDataOffset ==
  8·(cumulative frame start)`, and the UAAT always ends the pool).
- **NCLR** — the raw `fmt` (including the follower-sprite `0x000A0004`
  flag bits), `bExt`, logical `szByte`, and the whole PMCP table are
  retained raw and re-emitted; the section sizes and PMCP table offset
  are re-derived.
- **MAT** — the round trip is a *re-encryption*: the packed entry table
  is rebuilt from the parsed message spans and re-XORed with the
  Decrypt1 seeds, every message's units re-XORed with the Decrypt2
  rolling seed. Because the two XOR layers are independent, this
  exercises both decrypt paths in both directions.
- **BTX** — every dictionary offset, `sizeTex`/`sizePltt`, and both
  `TEXIMAGE_PARAM` words are re-derived. Two writer artifacts are copied
  verbatim (see below).
- **NARC** — the header version and BTAF's reserved field are retained
  raw; the BTAF ranges and all chunk sizes are re-derived; the BTNF
  chunk and GMIF body are copied verbatim (see below).

### BTX's verbatim artifacts

- **The two dictionary trie node arrays.** They are a binary-search trie
  the NitroSDK writer built over the names; the engine looks names up
  linearly and never touches the trie. Reproducing it would mean
  re-running the SDK's construction algorithm (ordering choices
  included), which is exactly the kind of writer detail a parser should
  not depend on — so the parse retains the nodes and the serializer
  copies them. `round_trips_verbatim_writer_artifacts` patches nonzero
  node bytes into a fixture to prove the copy is genuine.
- **The `tex4x4Info` data pointers** (`ofsTex`, `ofsTexPlttIdx`).
  Retail ships no COMP4x4 textures, so these two `u32`s are garbage on
  every one of the 1,157 files — never read by the engine, never
  validated by the parser, kept verbatim so the round trip covers them.

Name-slot padding (after each dictionary name to the fixed 16 bytes) is
re-written as zeros — the pinned scan confirmed zero padding on both
dictionaries of every retail file.

### NARC's verbatim BTNF

The BTNF filename table is a directory tree with writer-chosen subtable
ordering and no derived layout; the engine reads the parsed `dir/file`
paths, so re-serializing the tree from paths would be a second, parallel
implementation with nothing to check it. The chunk is retained as a raw
slice and copied — while BTAF, the chunk sizes, and the header are
re-derived around it, so a mis-parsed archive still cannot round-trip.

## SDAT is excluded

SDAT does not byte round-trip: the parsed struct does not retain the
writer's SYMB string-pool packing, INFO record interleaving, or block
padding, and STRMPLAYER records are not exposed at all. Reproducing the
file would mean re-running the SDK writer's layout choices rather than
testing the parse. The existing `tests/sdat_hg.rs` census checks the
parse; Phase 7 revisits SDAT with the audio work.

## The retail census (`tests/roundtrip_hg.rs`)

Walks every loose NitroFS file plus every NARC member (the same walk as
the format-specific tests), sniffs the magic, parses, re-serializes,
byte-compares — and pins the per-format counts:

| Format | Files | Notes |
|--------|-------|-------|
| NCGR   | 7,949 | plus the corrupt `data/dp_areawindow.NCGR` (a DP leftover with a zero BOM and under-declared sizes — the sniff never reaches it, the strict parser rejects it) |
| NCLR   | 4,953 | |
| NSCR   |   793 | |
| NCER   |   612 | |
| NANR   |   596 | 146 with a UAAT block |
| BTX    | 1,157 | |
| NARC   |   308 | |
| MAT    | 1,453 | 829 in `a/0/2/7`, 624 in `pbr/msg.narc` — the only two MAT archives, so they are walked directly (MAT has no magic to sniff) |

Every one of them re-serializes byte-exact.

The per-format synthetic-fixture tests (in each format module) also
assert `to_bytes()` byte-exact on every builder variant, so the round
trip runs in the unit tests without a ROM too.