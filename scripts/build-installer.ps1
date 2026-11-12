# Build the Rift Coach (Rust/Tauri) NSIS installer without MSVC (GNU toolchain).
#   powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1 [-SkipUi] [-SkipNpmCi]
# Output: rust\target\release\bundle\nsis\Rift Coach_<version>_x64-setup.exe
# Prints the installer path as the last line. Non-zero exit code on failure.
param(
  [switch]$SkipUi,      # dist\renderer already built
  [switch]$SkipNpmCi,   # node_modules already installed
  [string]$TauriCli = '2.12.4'
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

function Step($msg) { Write-Host "==> $msg" }
function Run($exe, [string[]]$argv) {
  & $exe @argv
  if ($LASTEXITCODE -ne 0) { throw "$exe $($argv -join ' ') failed with exit code $LASTEXITCODE" }
}

Push-Location $repo
try {
  if (-not $SkipUi) {
    if (-not $SkipNpmCi -or -not (Test-Path node_modules)) { Step 'npm ci'; Run npm @('ci', '--no-audit', '--no-fund') }
    Step 'npm run build (UI -> dist\renderer)'; Run npm @('run', 'build')
  }
  if (-not (Test-Path 'dist\renderer\control.html')) { throw 'dist\renderer is missing - run without -SkipUi' }

  # dlltool replacement for the GNU toolchain (cargo-gnu.ps1 builds it once into rust\target\gnu-tools).
  Step 'GNU helper tools'
  Run powershell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "$repo\rust\app\cargo-gnu.ps1", '--version')
  $env:PATH = "$repo\rust\target\gnu-tools;$env:PATH"

  # tauri-cli: the prebuilt npm binary (pinned to match rust\packaging\nsis\installer.nsi).
  # `cargo install tauri-cli` does NOT work here: its `ring` dependency needs a C compiler
  # (gcc/MSVC), which this GNU setup does not have. The CLI only drives cargo (GNU toolchain via
  # rust\rust-toolchain.toml) and downloads NSIS + nsis_tauri_utils into %LOCALAPPDATA%\tauri.
  Step "tauri build --bundles nsis (@tauri-apps/cli@$TauriCli)"
  Push-Location "$repo\rust\app"
  try { Run npx @('--yes', "@tauri-apps/cli@$TauriCli", 'build', '--bundles', 'nsis') } finally { Pop-Location }

  $out = Get-ChildItem "$repo\rust\target\release\bundle\nsis\*-setup.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if (-not $out) { throw 'installer not found in rust\target\release\bundle\nsis' }
  Step ("installer: {0} ({1:N1} MB)" -f $out.FullName, ($out.Length / 1MB))
  $out.FullName
} finally { Pop-Location }
