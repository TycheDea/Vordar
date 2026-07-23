# Regression gate for one criterion bench target vs its saved "main" baseline.
#
#   powershell scripts/bench-gate.ps1 [-Bench snapshot] [-Threshold 0.10]
#
# Requires a baseline already recorded via:
#   cargo bench -p vordar-benches --bench <Bench> -- --save-baseline main
#
# Runs `cargo bench -p vordar-benches --bench <Bench> -- --baseline main`, then reads
# criterion's on-disk comparison output (target/criterion/**/change/estimates.json,
# mean.point_estimate as the relative change vs "main") for every bench instance whose
# files were written by this run (matched by timestamp, since one bench target's groups
# share target/criterion with all ten others). Appends one line to
# docs/benchmarks/gate-log.txt. Exits 1 if any bench's mean regressed by more than
# Threshold, or if the run produced no comparison data (no baseline saved yet).

param(
    [string]$Bench = 'snapshot',
    [double]$Threshold = 0.10
)

$repoRoot = Split-Path -Parent $PSScriptRoot
$criterionDir = Join-Path $repoRoot 'target\criterion'
$gateLog = Join-Path $repoRoot 'docs\benchmarks\gate-log.txt'
$inv = [System.Globalization.CultureInfo]::InvariantCulture

$runStart = Get-Date

Push-Location $repoRoot
try {
    cargo bench -p vordar-benches --bench $Bench -- --baseline main
    $benchExit = $LASTEXITCODE
}
finally {
    Pop-Location
}

if ($benchExit -ne 0) {
    Write-Error "cargo bench exited with code $benchExit"
    exit $benchExit
}

$changeFiles = @(Get-ChildItem -Path $criterionDir -Recurse -Filter 'estimates.json' -ErrorAction SilentlyContinue |
    Where-Object { $_.Directory.Name -eq 'change' -and $_.LastWriteTime -ge $runStart })

if ($changeFiles.Count -eq 0) {
    Write-Error "No criterion comparison data found for bench '$Bench' after this run. Save a baseline first: cargo bench -p vordar-benches --bench $Bench -- --save-baseline main"
    exit 1
}

$results = @()
foreach ($f in $changeFiles) {
    $instanceDir = $f.Directory.Parent
    $benchmarkJsonPath = Join-Path $instanceDir.FullName 'new\benchmark.json'
    $fullId = $instanceDir.Name
    if (Test-Path $benchmarkJsonPath) {
        $bj = Get-Content $benchmarkJsonPath -Raw | ConvertFrom-Json
        if ($bj.full_id) { $fullId = $bj.full_id }
    }
    $est = Get-Content $f.FullName -Raw | ConvertFrom-Json
    $results += [PSCustomObject]@{ Bench = $fullId; Change = $est.mean.point_estimate }
}
$results = $results | Sort-Object Bench

Write-Host ''
Write-Host ('{0,-45} {1,10}  {2}' -f 'Bench', 'Change', 'Verdict')
Write-Host ('-' * 70)
foreach ($r in $results) {
    $pct = $r.Change * 100
    $sign = '+'
    if ($pct -lt 0) { $sign = '' }
    $pctStr = $sign + $pct.ToString('0.00', $inv) + '%'
    $verdict = 'ok'
    if ($r.Change -gt $Threshold) { $verdict = 'REGRESSION' }
    Write-Host ('{0,-45} {1,10}  {2}' -f $r.Bench, $pctStr, $verdict)
}
Write-Host ''

$maxDelta = ($results | Measure-Object -Property Change -Maximum).Maximum
$regressions = @($results | Where-Object { $_.Change -gt $Threshold })
$fired = $false
if ($regressions.Count -gt 0) { $fired = $true }

$firedStr = 'n'
if ($fired) { $firedStr = 'y' }
$dateStr = Get-Date -Format 'yyyy-MM-dd'
Add-Content -Path $gateLog -Value "$dateStr, $Bench, max-delta $($maxDelta.ToString('0.000', $inv)), fired $firedStr"

if ($fired) {
    $worst = $regressions | Sort-Object Change -Descending | Select-Object -First 1
    Write-Host "Regression gate FIRED: $($worst.Bench) regressed $($worst.Change.ToString('0.000', $inv)) (threshold $Threshold)." -ForegroundColor Red
    exit 1
}

Write-Host "Regression gate passed (max delta $($maxDelta.ToString('0.000', $inv)), threshold $Threshold)."
exit 0
