param(
    [Parameter(Mandatory = $true)]
    [string]$ResponseFile
)

$ErrorActionPreference = "Stop"

if (!(Test-Path $ResponseFile)) {
    throw "Response file not found: $ResponseFile"
}

$text = Get-Content -Path $ResponseFile -Raw
$hasCause = $text -match '(?m)^\s*CAUSE:'
$hasFix = $text -match '(?m)^\s*FIX:'

if (-not $hasCause -or -not $hasFix) {
    Write-Error "Invalid response format. Required sections: CAUSE and FIX."
    exit 1
}

Write-Host "Response format valid."
