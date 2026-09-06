# arm-runner — the per-function oracle

`arm-runner` is a test-only ARMv5TE interpreter for the ARM9 side,
living in `crates/apricorn-harness/src/arm/`. Where the melonDS oracle
proves the *whole machine* (boot, interrupts, both CPUs, real
timers/DMA), arm-runner proves *individual leaves*: it loads the
original retail ARM9 image into flat RAM and calls original functions
with controlled inputs, capturing the registers and memory they leave
behind. This is the differential counterpart to the oracle's probe
mode — the same entry state (r0–r3 = args, r12 = 0, a scratch stack,
`lr` = sentinel) on both machines, the same watched-region hashes out.

It is **never shipped in the game**: nothing under `arm/` is referenced
outside the harness crate.

## The CLI

```text
usage: arm-runner call --rom ROM.nds --fn PIN [--arg 0x1234]...
       arm-runner scan --rom ROM.nds --constant 0x41C64E6D
       arm-runner pins
```

* `call` verifies every committed pin against the loaded image (a wrong
  dump or a drifted address fails loudly — see below), runs the named
  leaf to the sentinel, and prints the return registers.
* `scan` reports where a 4-aligned word constant appears in the
  decompressed image — the discovery tool behind the pin table
  (e.g. the LCG multiplier `0x41C64E6D` finds `math_util`'s literal
  pools).
* `pins` lists the committed table.

Exit code 0 = success, 64 = usage or load error.

## Instruction scope

* **Implemented:** the ARMv5TE core — full data-processing with the
  barrel shifter; all single-register load/store addressing modes
  (`LDR`/`STR`/`LDRB`/`STRB`/`LDRH`/`STRH`/`LDRSB`/`LDRSH`,
  pre/post-indexing, writeback); `LDM`/`STM`; `MUL`/`MLA`/`UMULL`/
  `UMLAL`/`SMULL`/`SMLAL`; `CLZ`; the v5TE DSP saturating family
  (`QADD`/`QSUB`/`QDADD`/`QDSUB`); `B`/`BL`/`BX`/`BLX` in both
  instruction sets; `MCR`/`MRC` (CP15) as benign no-ops — the
  math_util leaves never touch them, but the SDK does.
* **Out of scope, loudly:** interrupts, timers, DMA, caches, the ARM7,
  IPC. `SWI` and any unimplemented encoding are errors, never silently
  skipped. arm-runner runs leaf functions to the sentinel `lr`; it is
  not the game loop — that's the oracle's job.

## Memory model

Flat address decode into owned buffers: main RAM (4 MiB @ 0x02000000),
ITCM (32 KiB @ 0x01000000), DTCM (16 KiB @ 0x0FF8000), plus scratch
windows for the stack and args. Unmapped access is a hard error.
The encrypted secure area (below the 0x02000800 entry point) is never
executed: arm-runner loads the BLZ-decompressed image as data and only
enters at pinned function addresses, all far above it.

## Pin verification

Every `call` and every `RetailArm9::load` verifies the committed pin
table (`crates/apricorn-harness/pins/arm9.tsv`) against the loaded
decompressed image — each pin's prologue SHA-1 must match, and data
pins past the image end must read as zeroed bss. A wrong ROM dump or a
drifted address fails loudly instead of executing garbage.

## Correctness — and the honesty clause

There is **no formal proof** of the interpreter. The correctness
strategy is layered:

1. per-encoding unit tests over the decoder and executor;
2. known-value tests derived from pret's C semantics — the LCG
   (`state*0x41C64E6D+0x6073`, seed `0x1234` → draw `0x4dcb`), the
   Mersenne Twister, CRC-16/CCITT of `"123456789"`;
3. differential probes against the oracle: the same function, same
   seeded state in, same registers and region hashes out
   (`tests/equivalence_hg.rs`).

The oracle is the proof. If arm-runner and the oracle ever disagree,
the comparator names the record and the interpreter loses the argument.