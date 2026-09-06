#!/usr/bin/env bash
# scripts/replay.sh — the Phase 2 exit demo: replay a corpus case on
# the oracle and print EQUIVALENT or the exact frame/region of the
# first divergence.
#
# usage: scripts/replay.sh <corpus-case> [--update]
#   e.g. scripts/replay.sh corpus/boot-idle
#        scripts/replay.sh corpus/boot-idle --update   # regenerate expected.trace
#
# Prerequisites (the script says so and exits, it never guesses):
#   - hg_usa.nds at the repo root (your own retail US dump)
#   - the oracle built by oracle/setup.sh into out/oracle/
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
case_dir="${1:?usage: scripts/replay.sh <corpus-case> [--update]}"; shift || true

[ -f "$repo/hg_usa.nds" ] ||
    { echo "error: hg_usa.nds not found at the repo root — supply your own retail US dump" >&2; exit 1; }
[ -f "$repo/out/oracle/apricorn-oracle" ] || [ -f "$repo/out/oracle/apricorn-oracle.exe" ] ||
    { echo "error: out/oracle/apricorn-oracle not built — run oracle/setup.sh first" >&2; exit 1; }

# Build the harness bin (quiet), then hand off to apricorn-replay —
# its exit code is the verdict: 0 EQUIVALENT, 1 diverged, 64 usage/gate.
cargo build --quiet -p apricorn-harness --bin apricorn-replay
exec "$repo/target/debug/apricorn-replay" "$case_dir" "$@"