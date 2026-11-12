# Run cargo for the GNU toolchain without MinGW installed.
#   powershell -ExecutionPolicy Bypass -File app\cargo-gnu.ps1 build -p rift-coach
#   powershell -ExecutionPolicy Bypass -File app\cargo-gnu.ps1 test -p rc-core
# rustc needs `dlltool.exe` for raw-dylib imports (windows / windows-sys crates); the real
# one needs the GNU assembler, so we build tools\dlltool.rs (std only) once and put it in PATH.
# The Windows resource (icon + manifest) is handled by app\build.rs when `windres` is missing.
$ErrorActionPreference = 'Stop'
$rustDir = Split-Path -Parent $PSScriptRoot
$toolDir = Join-Path $rustDir 'target\gnu-tools'
$tool = Join-Path $toolDir 'dlltool.exe'
$src = Join-Path $PSScriptRoot 'tools\dlltool.rs'
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
if (-not (Test-Path $tool) -or (Get-Item $src).LastWriteTime -gt (Get-Item $tool).LastWriteTime) {
  New-Item -ItemType Directory -Force $toolDir | Out-Null
  Push-Location $rustDir
  try { rustc -O --edition 2021 $src -o $tool; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE } } finally { Pop-Location }
}
$env:PATH = "$toolDir;$env:PATH"
Push-Location $rustDir
try { cargo @args; exit $LASTEXITCODE } finally { Pop-Location }
