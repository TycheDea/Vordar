<#
Weekly Task Scheduler job (VordarTokenReport, Sun 09:00). Pulls ccusage's
current-week totals and the claude-memory token-insights ingest summary,
then writes docs/tokens/<run-date>.md. Exit 0 once the report is written;
exit 1 only when ccusage itself fails (the ingest step degrades to an
in-report error line instead of failing the run).
#>

$repoRoot = Split-Path -Parent $PSScriptRoot
$today = Get-Date -Format 'yyyy-MM-dd'
$tokensDir = Join-Path $repoRoot 'docs\tokens'
New-Item -ItemType Directory -Force -Path $tokensDir | Out-Null
$reportPath = Join-Path $tokensDir "$today.md"

$ccusageErrFile = Join-Path $env:TEMP 'vordar-token-report-ccusage-err.txt'
$ccusageLines = & ccusage weekly --json 2> $ccusageErrFile
$ccusageExit = $LASTEXITCODE
if ($ccusageExit -ne 0 -or -not $ccusageLines) {
    $errText = if (Test-Path $ccusageErrFile) { Get-Content $ccusageErrFile -Raw } else { '' }
    Write-Error "ccusage weekly --json failed (exit $ccusageExit): $errText"
    exit 1
}
$ccusage = ($ccusageLines | Out-String) | ConvertFrom-Json
$week = $ccusage.weekly | Select-Object -Last 1

$ingestBase = Join-Path $HOME '.claude\plugins\cache\Claudest\claude-memory'
$ingestScript = $null
$versionDirs = Get-ChildItem -Path $ingestBase -Directory -ErrorAction SilentlyContinue
if ($versionDirs) {
    $latestDir = $versionDirs | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    $candidate = Join-Path $latestDir.FullName 'skills\get-token-insights\scripts\ingest_token_data.py'
    if (Test-Path $candidate) { $ingestScript = $candidate }
}

$pythonExe = $null
foreach ($candidateExe in @('python3', 'python')) {
    if (Get-Command $candidateExe -ErrorAction SilentlyContinue) { $pythonExe = $candidateExe; break }
}

$ingestSummary = $null
$ingestErrorText = $null
if (-not $ingestScript) {
    $ingestErrorText = "ingest_token_data.py not found under $ingestBase"
} elseif (-not $pythonExe) {
    $ingestErrorText = 'no python interpreter found (tried python3, python)'
} else {
    $ingestErrFile = Join-Path $env:TEMP 'vordar-token-report-ingest-err.txt'
    try {
        $ingestLines = & $pythonExe $ingestScript 2> $ingestErrFile
        $ingestExit = $LASTEXITCODE
        if ($ingestExit -ne 0 -or -not $ingestLines) {
            $ingestErrorText = "ingest_token_data.py exited $ingestExit`: " + (Get-Content $ingestErrFile -Raw -ErrorAction SilentlyContinue)
        } else {
            $ingestSummary = ($ingestLines | Out-String) | ConvertFrom-Json
        }
    } catch {
        $ingestErrorText = $_.Exception.Message
    }
}

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("# Token report - $today")
$lines.Add('')
$lines.Add('Source: ccusage weekly --json (v20)')
$lines.Add('')
$lines.Add("## Current week ($($week.period))")
$lines.Add("- Total cost: `$$([math]::Round($week.totalCost, 2))")
$lines.Add("- Total tokens: $($week.totalTokens)")
$lines.Add("- Input tokens: $($week.inputTokens)")
$lines.Add("- Output tokens: $($week.outputTokens)")
$lines.Add("- Cache creation tokens: $($week.cacheCreationTokens)")
$lines.Add("- Cache read tokens: $($week.cacheReadTokens)")
$lines.Add('')
$lines.Add('### Per-model breakdown')
foreach ($m in $week.modelBreakdowns) {
    $lines.Add("- $($m.modelName): `$$([math]::Round($m.cost, 2)) cost, $($m.inputTokens) in / $($m.outputTokens) out, cache read $($m.cacheReadTokens), cache creation $($m.cacheCreationTokens)")
}
$lines.Add('')
$lines.Add('## claude-memory ingest summary')
if ($ingestSummary) {
    $k = $ingestSummary.kpis
    $lines.Add("- Sessions analyzed: $($ingestSummary.total_sessions) (range $($ingestSummary.date_range.earliest) to $($ingestSummary.date_range.latest))")
    $lines.Add("- Global cache hit rate: $($k.global_cache_ratio)")
    $lines.Add("- Tracked total cost: `$$($k.total_cost_usd)")
    $lines.Add("- Tool error rate: $($k.tool_error_rate)")
    $lines.Add("- Cache cliffs: $($k.cache_cliffs)")
    $lines.Add("- Max-token stops: $($k.max_token_stops)")
    $lines.Add('')
    $lines.Add('### Top waste insights')
    if ($ingestSummary.insights -and $ingestSummary.insights.Count -gt 0) {
        foreach ($ins in ($ingestSummary.insights | Select-Object -First 5)) {
            $lines.Add("- [$($ins.severity)] $($ins.title): $($ins.finding)")
        }
    } else {
        $lines.Add('- (none reported)')
    }
} else {
    $lines.Add("- INGEST FAILED: $ingestErrorText")
}

Set-Content -Path $reportPath -Value ($lines -join "`n") -Encoding UTF8
exit 0
