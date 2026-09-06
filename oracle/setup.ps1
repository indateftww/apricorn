# Builds the apricorn differential-testing oracle: melonDS 1.1 with the
# committed patch set, configured core-only (no Qt/SDL frontend, no JIT,
# no OpenGL, no GDB stub) into a deterministic software interpreter.
#
# Everything this script produces is gitignored: the clone lives in
# refs/melonds, build artifacts in out/oracle. Nothing ROM-derived is
# committed; the oracle reads hg_usa.nds at run time like everything else.
#
# Usage:  ./oracle/setup.ps1
# Result: out/oracle/apricorn-oracle.exe

param(
    # MinGW toolchain root; the repo-standard MSYS2 install.
    [string]$MinGW = "C:/msys64/mingw64"
)

$ErrorActionPreference = 'Stop'
$repo = (Get-Item $PSScriptRoot).Parent.FullName
$clone = Join-Path $repo 'refs/melonds'
$build = Join-Path $repo 'out/oracle'

# 1. Shallow clone of the pinned tag (never committed; refs/ is ignored).
if (-not (Test-Path $clone)) {
    git clone --depth 1 --branch 1.1 `
        https://github.com/melonDS-emu/melonDS.git $clone
    if ($LASTEXITCODE -ne 0) { throw "clone failed" }
}
git -C $clone checkout -- .
git -C $clone clean -fd

# 2. Apply the committed patch set (LF-only, per .gitattributes).
Get-ChildItem "$PSScriptRoot/patches" -Filter *.patch | Sort-Object Name | ForEach-Object {
    git -C $clone apply --whitespace=nowarn $_.FullName
    if ($LASTEXITCODE -ne 0) {
        throw "patch $($_.Name) failed to apply (clone is not at tag 1.1?)"
    }
}

# 3. Configure + build core-only. The flags are the determinism contract:
#    interpreter only (JIT off), software renderer (OpenGL off), no
#    debugger hooks, no frontend.
#
#    $MinGW/bin must be on PATH for the *build* (cc1.exe can't find its
#    runtime DLLs otherwise and gcc dies silently — an absolute
#    -DCMAKE_C_COMPILER does not fix that) and for *running* the oracle
#    (it loads libstdc++-6.dll & friends from there).
$gcc = Join-Path $MinGW 'bin/gcc.exe'
if (-not (Test-Path $gcc)) { throw "MinGW gcc not found at $gcc" }
$gxx = Join-Path $MinGW 'bin/g++.exe'
$env:PATH = "$MinGW/bin;$env:PATH"

cmake -S $clone -B $build -G Ninja `
    -DCMAKE_C_COMPILER="$gcc" -DCMAKE_CXX_COMPILER="$gxx" `
    -DCMAKE_BUILD_TYPE=Release `
    -DBUILD_QT_SDL=OFF `
    -DENABLE_JIT=OFF `
    -DENABLE_OGLRENDERER=OFF `
    -DENABLE_GDBSTUB=OFF `
    -DMELONDS_EMBED_BUILD_INFO=OFF
if ($LASTEXITCODE -ne 0) { throw "cmake configure failed" }

cmake --build $build --target apricorn-oracle
if ($LASTEXITCODE -ne 0) { throw "build failed" }

Write-Host "oracle built: $build/apricorn-oracle.exe"