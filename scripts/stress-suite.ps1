# Runs the workspace test suite under CPU oversubscription to catch control
# laws and timing-sensitive tests that only fail when starved for cycles.
#
#   powershell scripts/stress-suite.ps1 [-Load 3.0] [-Runs 1] [-Filter '<nextest -E expr>']
#
# Spinners are busy-loop powershell processes, always cleaned up via
# try/finally even on Ctrl-C or a failed run. Exits non-zero if any run
# fails, so it can gate.

param(
    [double]$Load = 3.0,
    [int]$Runs = 1,
    [string]$Filter = ''
)

$processorCount = [Environment]::ProcessorCount
$spinnerCount = [int][Math]::Round($Load * $processorCount)

Write-Host "Spinning up $spinnerCount busy-loop processes (Load=$Load x $processorCount cores)..."

$spinners = @()
$allPassed = $true

try {
    for ($i = 0; $i -lt $spinnerCount; $i++) {
        $spinners += Start-Process powershell -ArgumentList '-NoProfile','-Command','while($true){}' -WindowStyle Hidden -PassThru
    }

    for ($run = 1; $run -le $Runs; $run++) {
        Write-Host "=== Run $run/$Runs ==="
        if ($Filter) {
            & cargo nextest run --workspace -E $Filter
        } else {
            & cargo nextest run --workspace
        }
        $exitCode = $LASTEXITCODE
        if ($exitCode -eq 0) {
            Write-Host "Run $run/$Runs PASSED"
        } else {
            Write-Host "Run $run/$Runs FAILED (exit $exitCode)"
            $allPassed = $false
        }
    }
}
finally {
    Write-Host "Cleaning up $($spinners.Count) spinner processes..."
    $spinners | Stop-Process -Force -ErrorAction SilentlyContinue
}

if (-not $allPassed) {
    exit 1
}
exit 0
