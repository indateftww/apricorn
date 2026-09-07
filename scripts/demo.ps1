# scripts/demo.ps1 — the Phase 3 headless demo: run the boot chain,
# rasterize the dump points, and write top/bottom PNGs + SHA-1s into
# out/frames/ — the testable manual check without a window.
#
# usage: scripts/demo.ps1
#
# Prerequisites (the script says so and exits, it never guesses):
#   - hg_usa.nds at the repo root (your own retail US dump)
#
# The printed hashes are the same digests tests/raster_hg.rs pins; the
# PNGs are for your eyes (review them against the real game's intro and
# title — the Phase 3 exit criterion).
$ErrorActionPreference = 'Stop'
$repo = (Split-Path $PSScriptRoot -Parent)

if (-not (Test-Path (Join-Path $repo 'hg_usa.nds'))) {
    Write-Error "hg_usa.nds not found at the repo root - supply your own retail US dump"
}

# Build the dump CLI (quiet), then run it by path — its exit code is
# the verdict: 0 dumped, 1 ROM/store failure, 64 usage.
Push-Location $repo
try {
    cargo build --quiet -p apricorn-gfx --bin apricorn-gfx-dump
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally { Pop-Location }

$bin = if ($IsWindows -or $env:OS -eq 'Windows_NT') {
    Join-Path $repo 'target\debug\apricorn-gfx-dump.exe'
} else {
    Join-Path $repo 'target/debug/apricorn-gfx-dump'
}

# The dump points of tests/raster_hg.rs: the copyright beat (the
# hold, the mid-fade identity, black, the Game Freak logo and its
# hold) and the title screen's settled statics.
$frames = '0,45,100,135,200,320,420,1000'
& $bin --rom (Join-Path $repo 'hg_usa.nds') --frame $frames --out (Join-Path $repo 'out/frames')
exit $LASTEXITCODE