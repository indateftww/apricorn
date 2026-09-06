# scripts/replay.ps1 — the Phase 2 exit demo: replay a corpus case on
# the oracle and print EQUIVALENT or the exact frame/region of the
# first divergence.
#
# usage: scripts/replay.ps1 <corpus-case> [-Update]
#   e.g. scripts/replay.ps1 corpus/boot-idle
#        scripts/replay.ps1 corpus/boot-idle -Update   # regenerate expected.trace
#
# Prerequisites (the script says so and exits, it never guesses):
#   - hg_usa.nds at the repo root (your own retail US dump)
#   - the oracle built by oracle/setup.ps1 into out/oracle/
param(
    [Parameter(Position = 0, Mandatory = $true)]
    [string]$Case,

    # Regenerate the case's expected.trace instead of comparing.
    [switch]$Update
)

$ErrorActionPreference = 'Stop'
$repo = (Split-Path $PSScriptRoot -Parent)

if (-not (Test-Path (Join-Path $repo 'hg_usa.nds'))) {
    Write-Error "hg_usa.nds not found at the repo root - supply your own retail US dump"
}
$oracle = @('out/oracle/apricorn-oracle.exe', 'out/oracle/apricorn-oracle') |
    Where-Object { Test-Path (Join-Path $repo $_) }
if (-not $oracle) {
    Write-Error "out/oracle/apricorn-oracle not built - run oracle/setup.ps1 first"
}

# Build the harness bin (quiet), then hand off to apricorn-replay —
# its exit code is the verdict: 0 EQUIVALENT, 1 diverged, 64 usage/gate.
Push-Location $repo
try {
    cargo build --quiet -p apricorn-harness --bin apricorn-replay
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally { Pop-Location }

$bin = if ($IsWindows -or $env:OS -eq 'Windows_NT') {
    Join-Path $repo 'target\debug\apricorn-replay.exe'
} else {
    Join-Path $repo 'target/debug/apricorn-replay'
}
$replayArgs = @($Case)
if ($Update) { $replayArgs += '--update' }
& $bin @replayArgs
exit $LASTEXITCODE