#!/bin/sh
# Builds the apricorn differential-testing oracle: melonDS 1.1 with the
# committed patch set, configured core-only (no Qt/SDL frontend, no JIT,
# no OpenGL, no GDB stub) into a deterministic software interpreter.
#
# Everything this script produces is gitignored: the clone lives in
# refs/melonds, build artifacts in out/oracle. Nothing ROM-derived is
# committed; the oracle reads hg_usa.nds at run time like everything else.
#
# Usage:  ./oracle/setup.sh
# Result: out/oracle/apricorn-oracle

set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
clone="$repo/refs/melonds"
build="$repo/out/oracle"

# 1. Shallow clone of the pinned tag (never committed; refs/ is ignored).
if [ ! -d "$clone" ]; then
    git clone --depth 1 --branch 1.1 \
        https://github.com/melonDS-emu/melonDS.git "$clone"
fi
git -C "$clone" checkout -- .
git -C "$clone" clean -fd

# 2. Apply the committed patch set (LF-only).
for patch in "$repo"/oracle/patches/*.patch; do
    git -C "$clone" apply --whitespace=nowarn "$patch"
done

# 3. Configure + build core-only. The flags are the determinism contract:
#    interpreter only (JIT off), software renderer (OpenGL off), no
#    debugger hooks, no frontend.
#
#    The MinGW bin dir must be on PATH for the build (cc1.exe needs its
#    runtime DLLs from there) and for running the oracle (libstdc++-6.dll
#    & friends are loaded from it).
: "${MinGW:=/c/msys64/mingw64}"
export PATH="$MinGW/bin:$PATH"

cmake -S "$clone" -B "$build" -G Ninja \
    -DCMAKE_C_COMPILER="$MinGW/bin/gcc.exe" \
    -DCMAKE_CXX_COMPILER="$MinGW/bin/g++.exe" \
    -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_QT_SDL=OFF \
    -DENABLE_JIT=OFF \
    -DENABLE_OGLRENDERER=OFF \
    -DENABLE_GDBSTUB=OFF \
    -DMELONDS_EMBED_BUILD_INFO=OFF

cmake --build "$build" --target apricorn-oracle

echo "oracle built: $build/apricorn-oracle"