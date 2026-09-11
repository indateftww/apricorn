# scripts/shots.ps1 — screenshot a corpus case on the oracle: both LCDs
# as 256x192 RGB8 PNGs at the listed frames, written to a review
# directory under out/ (gitignored — never into the corpus).
#
# usage: scripts/shots.ps1 <corpus-case> <frames> [-Dir <dir>]
#   e.g. scripts/shots.ps1 corpus/new-game "300,1200-1210"
#        scripts/shots.ps1 corpus/boot-idle 599 -Dir out/oracle-shots/idle
#
# <frames> is a comma list of frame indices or inclusive first-last
# ranges. The default -Dir is out/oracle-shots/<case name>. Files are
# named frame_NNNNNN_top.png / frame_NNNNNN_bottom.png.
#
# The run is a normal replay: the case's expected.trace is compared
# afterwards and the exit code is apricorn-replay's (0 EQUIVALENT,
# 1 diverged, 64 usage/gate — including "no expected.trace yet", in
# which case the PNGs are still written).
#
# Prerequisites (the script says so and exits, it never guesses):
#   - hg_usa.nds at the repo root (your own retail US dump)
#   - the oracle built by oracle/setup.ps1 into out/oracle/
#     (or APRICORN_ORACLE pointing at a scratch build)
param(
    [Parameter(Position = 0, Mandatory = $true)]
    [string]$Case,

    [Parameter(Position = 1, Mandatory = $true)]
    [string]$Frames,

    # Output directory; default out/oracle-shots/<case name>.
    [string]$Dir
)

$ErrorActionPreference = 'Stop'
$repo = (Split-Path $PSScriptRoot -Parent)

if (-not (Test-Path (Join-Path $repo 'hg_usa.nds'))) {
    Write-Error "hg_usa.nds not found at the repo root - supply your own retail US dump"
}
$oracle = @('out/oracle/apricorn-oracle.exe', 'out/oracle/apricorn-oracle') |
    Where-Object { Test-Path (Join-Path $repo $_) }
if (-not $oracle -and -not ($env:APRICORN_ORACLE -and (Test-Path $env:APRICORN_ORACLE))) {
    Write-Error "out/oracle/apricorn-oracle not built - run oracle/setup.ps1 first"
}

if (-not $Dir) {
    $Dir = Join-Path $repo ('out/oracle-shots/' + (Split-Path $Case -Leaf))
}

# Build the harness bin (quiet), then hand off to apricorn-replay.
Push-Location $repo
try {
    cargo build --quiet -p apricorn-harness --bin apricorn-replay
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally { Pop-Location }

$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$bin = if ($IsWindows -or $env:OS -eq 'Windows_NT') {
    Join-Path $target 'debug\apricorn-replay.exe'
} else {
    Join-Path $target 'debug/apricorn-replay'
}
& $bin --shots $Frames --shots-dir $Dir $Case
exit $LASTEXITCODE
