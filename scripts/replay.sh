#!/usr/bin/env bash
# scripts/replay.sh — the Phase 2 exit demo: replay a corpus case on
# the oracle (or, with --engine, on the real engine) and print
# EQUIVALENT or the exact frame/region of the first divergence.
#
# usage: scripts/replay.sh <corpus-case> [--update] [--engine]
#   e.g. scripts/replay.sh corpus/boot-idle
#        scripts/replay.sh corpus/boot-idle --update   # regenerate expected.trace (oracle only)
#        scripts/replay.sh corpus/boot-idle --engine   # engine trace vs the oracle baseline
#
# Prerequisites (the script says so and exits, it never guesses):
#   - hg_usa.nds at the repo root (your own retail US dump)
#   - the oracle built by oracle/setup.sh into out/oracle/ (not needed
#     with --engine: the engine replays against the committed baseline)
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
case_dir="${1:?usage: scripts/replay.sh <corpus-case> [--update] [--engine]}"; shift || true

engine=0
for arg in "$@"; do
    [ "$arg" = "--engine" ] && engine=1
done

[ -f "$repo/hg_usa.nds" ] ||
    { echo "error: hg_usa.nds not found at the repo root — supply your own retail US dump" >&2; exit 1; }
if [ "$engine" -eq 0 ]; then
    [ -f "$repo/out/oracle/apricorn-oracle" ] || [ -f "$repo/out/oracle/apricorn-oracle.exe" ] ||
        { echo "error: out/oracle/apricorn-oracle not built — run oracle/setup.sh first" >&2; exit 1; }
fi

# Build the harness bin (quiet), then hand off to apricorn-replay —
# its exit code is the verdict: 0 EQUIVALENT, 1 diverged, 64 usage/gate.
cargo build --quiet -p apricorn-harness --bin apricorn-replay
exec "$repo/target/debug/apricorn-replay" "$@" "$case_dir"
