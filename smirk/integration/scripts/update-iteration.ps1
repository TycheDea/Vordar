param(
    [Parameter(Mandatory = $true)]
    [string]$ErrorCode,
    [string]$Action = "",
    [ValidateSet("pass", "fail")]
    [string]$Result = "fail"
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$statePath = Join-Path $root "trackers\SESSION-STATE.yaml"
$logPath = Join-Path $root "trackers\ITERATION-LOG.md"

if (!(Test-Path $statePath)) {
    throw "Missing SESSION-STATE.yaml. Run init-integration.ps1 first."
}

$content = Get-Content -Path $statePath -Raw

$iterationMatch = [regex]::Match($content, 'iteration:\s*(\d+)')
$currentIteration = 0
if ($iterationMatch.Success) {
    $currentIteration = [int]$iterationMatch.Groups[1].Value
}
$newIteration = $currentIteration + 1

$lastErrorMatch = [regex]::Match($content, 'last_error_code:\s*"(.*)"')
$lastError = ""
if ($lastErrorMatch.Success) {
    $lastError = $lastErrorMatch.Groups[1].Value
}

$repeatMatch = [regex]::Match($content, 'repeated_error_count:\s*(\d+)')
$repeatCount = 0
if ($repeatMatch.Success) {
    $repeatCount = [int]$repeatMatch.Groups[1].Value
}

if ($lastError -eq $ErrorCode) {
    $repeatCount += 1
} else {
    $repeatCount = 0
}

$status = "failing"
if ($Result -eq "pass") {
    $status = "ok"
    $repeatCount = 0
}

$content = [regex]::Replace($content, 'iteration:\s*\d+', "iteration: $newIteration")
$content = [regex]::Replace($content, 'repeated_error_count:\s*\d+', "repeated_error_count: $repeatCount")
$content = [regex]::Replace($content, 'current_status:\s*".*"', "current_status: `"$status`"")
$content = [regex]::Replace($content, 'last_error_code:\s*".*"', "last_error_code: `"$ErrorCode`"")

Set-Content -Path $statePath -Value $content

$taskMatch = [regex]::Match($content, 'task:\s*"(.*)"')
$task = "unset"
if ($taskMatch.Success) {
    $task = $taskMatch.Groups[1].Value
}

$ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
$line = "- [$ts] task=$task iteration=$newIteration error=$ErrorCode result=$Result action=""$Action"""
Add-Content -Path $logPath -Value $line

if ($repeatCount -ge 2) {
    Write-Warning "Same error repeated >= 2 iterations. Use escalation template."
}

Write-Host "Iteration updated: $newIteration (repeat_count=$repeatCount, status=$status)"
