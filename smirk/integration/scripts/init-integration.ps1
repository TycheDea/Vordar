param()

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$templates = Join-Path $root "templates"
$trackers = Join-Path $root "trackers"

if (!(Test-Path $trackers)) {
    New-Item -ItemType Directory -Path $trackers | Out-Null
}

$pairs = @(
    @{ src = "state-header.yaml"; dst = "SESSION-STATE.yaml" },
    @{ src = "diagnostic-packet.yaml"; dst = "CURRENT-DIAGNOSTIC.yaml" },
    @{ src = "cursor-diff.yaml"; dst = "CURRENT-DIFF.yaml" }
)

foreach ($pair in $pairs) {
    $src = Join-Path $templates $pair.src
    $dst = Join-Path $trackers $pair.dst
    if (!(Test-Path $dst) -and (Test-Path $src)) {
        Copy-Item -Path $src -Destination $dst
    }
}

$decisionsPath = Join-Path $trackers "DECISIONS.md"
$logPath = Join-Path $trackers "ITERATION-LOG.md"
if (!(Test-Path $decisionsPath)) {
    Set-Content -Path $decisionsPath -Value "# Decisions Registry`n`n- D001: <decision> [status: frozen]"
}
if (!(Test-Path $logPath)) {
    Set-Content -Path $logPath -Value "# Iteration Log"
}

Write-Host "Integration toolkit initialized."
