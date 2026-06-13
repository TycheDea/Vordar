param(
    [string]$OutFile = ""
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$templates = Join-Path $root "templates"
$trackers = Join-Path $root "trackers"

$statePath = Join-Path $trackers "SESSION-STATE.yaml"
$diagPath = Join-Path $trackers "CURRENT-DIAGNOSTIC.yaml"
$diffPath = Join-Path $trackers "CURRENT-DIFF.yaml"

foreach ($p in @($statePath, $diagPath, $diffPath)) {
    if (!(Test-Path $p)) {
        throw "Missing required tracker file: $p"
    }
}

$stateText = Get-Content -Path $statePath -Raw
$diagText = Get-Content -Path $diagPath -Raw
$diffText = Get-Content -Path $diffPath -Raw

$repeatMatch = [regex]::Match($stateText, 'repeated_error_count:\s*(\d+)')
$repeatCount = 0
if ($repeatMatch.Success) {
    $repeatCount = [int]$repeatMatch.Groups[1].Value
}

$templateName = "debugger-iteration.txt"
if ($repeatCount -ge 2) {
    $templateName = "escalation-2x.txt"
}

$templatePath = Join-Path $templates $templateName
if (!(Test-Path $templatePath)) {
    throw "Missing prompt template: $templatePath"
}

$header = Get-Content -Path $templatePath -Raw

$prompt = @"
$header

STATE:
$stateText

DIAGNOSTIC:
$diagText

DELTA:
$diffText
"@

if ($OutFile -ne "") {
    Set-Content -Path $OutFile -Value $prompt
    Write-Host "Prompt written to $OutFile"
} else {
    Write-Output $prompt
}
