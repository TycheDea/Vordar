param(
    [Parameter(Mandatory = $true)]
    [string]$Task
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$statePath = Join-Path $root "trackers\SESSION-STATE.yaml"

if (!(Test-Path $statePath)) {
    throw "Missing SESSION-STATE.yaml. Run init-integration.ps1 first."
}

$content = Get-Content -Path $statePath -Raw

$content = [regex]::Replace($content, 'task:\s*".*"', "task: `"$Task`"")
$content = [regex]::Replace($content, 'iteration:\s*\d+', "iteration: 0")
$content = [regex]::Replace($content, 'repeated_error_count:\s*\d+', "repeated_error_count: 0")
$content = [regex]::Replace($content, 'current_status:\s*".*"', "current_status: `"failing`"")
$content = [regex]::Replace($content, 'last_error_code:\s*".*"', "last_error_code: `"`"")

Set-Content -Path $statePath -Value $content
Write-Host "Task started: $Task"
