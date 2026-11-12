param(
    [Parameter(Mandatory = $true)][string]$DataAppData,
    [string]$RunDir = (Join-Path ([IO.Path]::GetTempPath()) ('rift-coach-astra-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))),
    [string]$PinnedEvaluator
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$runPath = [IO.Path]::GetFullPath($RunDir)
if (Test-Path -LiteralPath $runPath) { throw 'Choose a new RunDir to preserve earlier evidence.' }
New-Item -ItemType Directory -Path $runPath | Out-Null
$env:PYTHONDONTWRITEBYTECODE = '1'
Push-Location $repoRoot
try {
    if ($PinnedEvaluator) {
        New-Item -ItemType Directory -Path (Join-Path $runPath 'bin') | Out-Null
        Copy-Item -LiteralPath $PinnedEvaluator -Destination (Join-Path $runPath 'bin\eval_tracker.exe')
    } else {
        py -3 scripts/dev/astra_guard.py build --appdata $DataAppData --out $runPath
        if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    }
    py -3 scripts/dev/astra_guard.py baseline --appdata $DataAppData --out $runPath
    if ($LASTEXITCODE -ne 0) { throw 'Baseline failed.' }
    py -3 scripts/dev/astra_experiment.py --appdata $DataAppData --out $runPath --workers 2
    if ($LASTEXITCODE -ne 0) { throw 'Experiment failed.' }
    py -3 scripts/dev/astra_audit.py --appdata $DataAppData --out (Join-Path $runPath 'data-audit.json')
    if ($LASTEXITCODE -ne 0) { throw 'Data audit failed.' }
    py -3 scripts/dev/astra_verify.py --appdata $DataAppData --out $runPath
    if ($LASTEXITCODE -ne 0) { throw 'Verification failed.' }
    Write-Output "Evidence: $runPath"
    Write-Output 'Results: model/astra-results.json; selected weights: model/astra-best.json'
} finally {
    Pop-Location
}
