# Rift Coach (Rust/Tauri) - one-command, non-interactive install for Windows 10/11 x64.
# Written to be run by coding agents (Claude Code / Codex) as well as people.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1 [options]
#
# Options (all optional, never prompts):
#   -Installer <path>    use this "Rift Coach_<ver>_x64-setup.exe" instead of building
#   -FromSource          build the installer from this repo even if one is already built
#   -InstallDir <dir>    install there (default: %LOCALAPPDATA%\Rift Coach; per-user, no admin)
#   -LeaguePath <dir>    League of Legends folder (default: auto-detect)
#   -EnableReplayApi     add EnableReplayApi=1 to [General] in <League>\Config\game.cfg (backup first)
#   -RiotKey <key>       store the Riot API key in %APPDATA%\Rift Coach\settings.json (never printed)
#   -SkipSmoke           do not start the installed exe with --smoke
#   -CheckOnly           only check prerequisites, install nothing
#   -Uninstall           silently uninstall (user data in %APPDATA%\Rift Coach is kept)
#
# Output: progress lines, then ONE final JSON line, e.g.
#   {"ok":true,"installedTo":"C:\\Users\\me\\AppData\\Local\\Rift Coach","webview2":true,
#    "claude":"logged-in","codex":true,"replayApi":true,"problems":[]}
# Exit code: 0 = ok, 1 = failed (see "problems"), 2 = unsupported system.
[CmdletBinding()]
param(
  [string]$Installer,
  [switch]$FromSource,
  [string]$InstallDir,
  [string]$LeaguePath,
  [switch]$EnableReplayApi,
  [string]$RiotKey,
  [switch]$SkipSmoke,
  [switch]$CheckOnly,
  [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repo = Split-Path -Parent $PSScriptRoot
$uninstKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Rift Coach'
$dataDir = Join-Path $env:APPDATA 'Rift Coach'

$problems = New-Object System.Collections.Generic.List[string]
$result = [ordered]@{ ok = $false; installedTo = $null; webview2 = $false; claude = 'missing'; codex = $false; replayApi = $false; problems = $problems }
function Say($msg) { Write-Host "[rift-coach] $msg" }
function Problem($msg) { $problems.Add($msg); Write-Host "[rift-coach] PROBLEM: $msg" }
function Finish([int]$code) {
  $result.ok = ($code -eq 0)
  Write-Output ($result | ConvertTo-Json -Compress -Depth 4)
  exit $code
}

function Get-InstallInfo {
  $p = Get-ItemProperty $uninstKey -ErrorAction SilentlyContinue
  if (-not $p) { return $null }
  $dir = "$($p.InstallLocation)".Trim('"')
  $exe = if ($p.MainBinaryName) { $p.MainBinaryName } else { 'rift-coach.exe' }
  [pscustomobject]@{ Dir = $dir; Exe = (Join-Path $dir $exe); Uninstaller = (Join-Path $dir 'uninstall.exe'); Version = $p.DisplayVersion }
}

# Waits for a process (and the uninstaller's temp copy, which NSIS re-launches) to finish.
function Wait-Gone([string]$path, [int]$seconds) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Test-Path $path) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
  -not (Test-Path $path)
}

# ---------------------------------------------------------------- uninstall
if ($Uninstall) {
  # Prerequisites are not checked in this mode.
  $result.webview2 = $null; $result.claude = $null; $result.codex = $null; $result.replayApi = $null
  $info = Get-InstallInfo
  if (-not $info) { Say 'not installed'; $result.installedTo = $null; Finish 0 }
  Say "uninstalling from $($info.Dir)"
  $p = Start-Process -FilePath $info.Uninstaller -ArgumentList '/S' -PassThru -Wait
  # uninstall.exe copies itself to %TEMP% and returns at once; wait for the files to go.
  if (-not (Wait-Gone $info.Exe 120) -or -not (Wait-Gone $info.Uninstaller 60)) { Problem "uninstaller (exit $($p.ExitCode)) left $($info.Dir)"; Finish 1 }
  Start-Sleep -Seconds 1
  if (Get-ItemProperty $uninstKey -ErrorAction SilentlyContinue) { Problem 'uninstall registry entry still present'; Finish 1 }
  Say "uninstalled (user data kept in $dataDir)"
  Finish 0
}

# ---------------------------------------------------------------- system
$os = [Environment]::OSVersion.Version
if (-not [Environment]::Is64BitOperatingSystem -or $os.Major -lt 10) {
  Problem "unsupported system: Windows $os, 64-bit=$([Environment]::Is64BitOperatingSystem) (need Windows 10/11 x64)"
  Finish 2
}
Say "Windows $os x64"

# ---------------------------------------------------------------- WebView2
function Test-WebView2 {
  $guid = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
  foreach ($k in "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$guid", "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$guid", "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$guid") {
    $pv = (Get-ItemProperty $k -ErrorAction SilentlyContinue).pv
    if ($pv -and $pv -ne '0.0.0.0') { return $pv }
  }
  $null
}
$wv = Test-WebView2
$result.webview2 = [bool]$wv
if ($wv) { Say "WebView2 runtime $wv" } else { Say 'WebView2 runtime missing - the installer will install it (needs internet)' }

# ---------------------------------------------------------------- League of Legends
function Find-League {
  if ($LeaguePath) { return $LeaguePath }
  $c = @()
  $json = Join-Path $env:ProgramData 'Riot Games\RiotClientInstalls.json'
  if (Test-Path $json) {
    try {
      $j = Get-Content $json -Raw | ConvertFrom-Json
      foreach ($prop in @($j.associated_client.PSObject.Properties)) { if ($prop.Name -match 'League of Legends') { $c += $prop.Name } }
    } catch { }
  }
  foreach ($root in 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall') {
    Get-ChildItem $root -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -like 'Riot Game league_of_legends*' } | ForEach-Object {
      $loc = (Get-ItemProperty $_.PSPath).InstallLocation; if ($loc) { $c += $loc }
    }
  }
  foreach ($d in 'C', 'D', 'E', 'F') { $c += "${d}:\Riot Games\League of Legends" }
  foreach ($p in $c) { $p = $p.TrimEnd('\', '/'); if (Test-Path (Join-Path $p 'LeagueClient.exe')) { return $p } }
  foreach ($p in $c) { $p = $p.TrimEnd('\', '/'); if (Test-Path (Join-Path $p 'Config')) { return $p } }
  $null
}

# EnableReplayApi=1 inside the [General] section?
function Test-ReplayApi([string[]]$lines) {
  $inGeneral = $false
  foreach ($l in $lines) {
    $t = $l.Trim()
    if ($t -match '^\[(.+)\]$') { $inGeneral = ($Matches[1] -eq 'General'); continue }
    if ($inGeneral -and $t -match '^EnableReplayApi\s*=\s*1\s*$') { return $true }
  }
  $false
}

$league = Find-League
if (-not $league) {
  Problem 'League of Legends not found (pass -LeaguePath "<folder with LeagueClient.exe>")'
} else {
  Say "League of Legends: $league"
  $cfg = Join-Path $league 'Config\game.cfg'
  if (-not (Test-Path $cfg)) {
    Problem "missing $cfg (start League once so it creates it)"
  } else {
    $lines = @(Get-Content -LiteralPath $cfg)
    $result.replayApi = Test-ReplayApi $lines
    if (-not $result.replayApi -and $EnableReplayApi) {
      if (Get-Process -Name 'League of Legends' -ErrorAction SilentlyContinue) {
        Problem 'League of Legends game is running - game.cfg not changed (run again after the game, -EnableReplayApi)'
      } else {
        $backup = "$cfg.bak-$(Get-Date -Format yyyyMMdd-HHmmss)"
        Copy-Item -LiteralPath $cfg -Destination $backup
        $out = New-Object System.Collections.Generic.List[string]
        $inGeneral = $false; $done = $false; $sawGeneral = $false
        foreach ($l in $lines) {
          $t = $l.Trim()
          if ($t -match '^\[(.+)\]$') {
            if ($inGeneral -and -not $done) { $out.Add('EnableReplayApi=1'); $done = $true }
            $inGeneral = ($Matches[1] -eq 'General'); if ($inGeneral) { $sawGeneral = $true }
            $out.Add($l); continue
          }
          if ($inGeneral -and $t -match '^EnableReplayApi\s*=') { if (-not $done) { $out.Add('EnableReplayApi=1'); $done = $true }; continue }
          $out.Add($l)
        }
        if ($inGeneral -and -not $done) { $out.Add('EnableReplayApi=1'); $done = $true }
        if (-not $sawGeneral) { $out.Insert(0, 'EnableReplayApi=1'); $out.Insert(0, '[General]') }
        [IO.File]::WriteAllLines($cfg, $out.ToArray(), (New-Object Text.UTF8Encoding $false))
        $result.replayApi = Test-ReplayApi @(Get-Content -LiteralPath $cfg)
        Say "EnableReplayApi=1 written to $cfg (backup: $backup)"
      }
    }
    if (-not $result.replayApi) { Say 'Replay API disabled (optional; needed only to learn from replays - use -EnableReplayApi)' }
  }
}

# ---------------------------------------------------------------- Claude Code / Codex
$claudeCmd = Get-Command claude -ErrorAction SilentlyContinue
if ($claudeCmd) {
  try {
    $raw = (& claude auth status 2>$null | Out-String)
    $st = $raw | ConvertFrom-Json
    $result.claude = if ($st.loggedIn) { 'logged-in' } else { 'not-logged-in' }
  } catch { $result.claude = 'not-logged-in' }
} else { $result.claude = 'missing' }
Say "Claude Code CLI: $($result.claude)"
$result.codex = [bool](Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue)
Say "Codex app: $(if ($result.codex) { 'installed' } else { 'missing' })"
if ($result.claude -ne 'logged-in' -and -not $result.codex) {
  Problem 'no AI coach backend: install and log in to Claude Code (claude auth login) or install the Codex app - the user must do this'
}

if ($CheckOnly) {
  $info = Get-InstallInfo
  if ($info -and (Test-Path $info.Exe)) { $result.installedTo = $info.Dir }
  # Only blocking problems fail -CheckOnly: the system check already exited above.
  Finish 0
}

# ---------------------------------------------------------------- installer
if (-not $Installer) {
  $built = Get-ChildItem "$repo\rust\target\release\bundle\nsis\*-setup.exe" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if ($built -and -not $FromSource) {
    $Installer = $built.FullName
  } elseif (Test-Path "$repo\rust\app\tauri.conf.json") {
    Say 'building the installer from source (npm ci, npm run build, cargo, NSIS) - takes several minutes'
    $log = Join-Path $env:TEMP 'rift-coach-build.log'
    & powershell -NoProfile -ExecutionPolicy Bypass -File "$repo\scripts\build-installer.ps1" *> $log
    if ($LASTEXITCODE -ne 0) { Problem "build failed (exit $LASTEXITCODE), log: $log"; Finish 1 }
    $built = Get-ChildItem "$repo\rust\target\release\bundle\nsis\*-setup.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    $Installer = $built.FullName
  } else {
    Problem 'no installer: pass -Installer <path to Rift Coach_*_x64-setup.exe>'; Finish 1
  }
}
if (-not (Test-Path -LiteralPath $Installer)) { Problem "installer not found: $Installer"; Finish 1 }
$Installer = (Resolve-Path -LiteralPath $Installer).Path
Say "installer: $Installer"

# ---------------------------------------------------------------- install (silent, per-user)
# NSIS: /S = silent; /D=<dir> must be last and unquoted. A running copy in the target folder is closed.
$argLine = '/S'
if ($InstallDir) { $argLine += " /D=$([IO.Path]::GetFullPath($InstallDir))" }
$p = Start-Process -FilePath $Installer -ArgumentList $argLine -PassThru -Wait
if ($p.ExitCode -ne 0) { Problem "installer exit code $($p.ExitCode) (1 = aborted, 2 = script error)"; Finish 1 }
$info = Get-InstallInfo
if (-not $info -or -not (Test-Path $info.Exe)) { Problem 'installer finished but the app was not found (no uninstall entry / exe)'; Finish 1 }
$result.installedTo = $info.Dir
Say "installed $($info.Version) to $($info.Dir)"
if (-not (Test-Path (Join-Path $info.Dir 'scripts\replay-fog.ps1'))) { Problem 'scripts\replay-fog.ps1 missing in the install folder' }
$result.webview2 = [bool](Test-WebView2)
if (-not $result.webview2) { Problem 'WebView2 runtime still missing after install' }

# ---------------------------------------------------------------- Riot API key (never printed)
if ($RiotKey) {
  $settingsPath = Join-Path $dataDir 'settings.json'
  try {
    New-Item -ItemType Directory -Force $dataDir | Out-Null
    $s = if (Test-Path $settingsPath) { Get-Content $settingsPath -Raw -Encoding UTF8 | ConvertFrom-Json } else { [pscustomobject]@{} }
    if (-not $s.PSObject.Properties['riot']) { $s | Add-Member -NotePropertyName riot -NotePropertyValue ([pscustomobject]@{ apiKey = ''; platform = 'eun1' }) }
    if (-not $s.riot.PSObject.Properties['apiKey']) { $s.riot | Add-Member -NotePropertyName apiKey -NotePropertyValue '' }
    $s.riot.apiKey = $RiotKey.Trim()
    [IO.File]::WriteAllText($settingsPath, ($s | ConvertTo-Json -Depth 20), (New-Object Text.UTF8Encoding $false))
    Say "Riot API key saved to $settingsPath (value not shown)"
    if (Get-Process | Where-Object { $_.Path -eq $info.Exe }) { Problem 'Rift Coach was running while the key was saved - restart it so it is used' }
  } catch { Problem "could not save the Riot API key: $($_.Exception.GetType().Name)" }
}

# ---------------------------------------------------------------- Riot key present? (value never printed)
try {
  $sp = Join-Path $dataDir 'settings.json'
  $hasKey = (Test-Path $sp) -and [bool]((Get-Content $sp -Raw -Encoding UTF8 | ConvertFrom-Json).riot.apiKey)
} catch { $hasKey = $false }
$result.riotKey = $hasKey
if (-not $hasKey) { Problem 'no Riot API key: enemy profiles, player profile and self-training are OFF until the user pastes a dev key (https://developer.riotgames.com, renew every 24 h) in Settings - tell the user' }

# ---------------------------------------------------------------- smoke test
# --smoke: hidden window, coach off, no autostart/hotkeys, prints SMOKE-* lines and exits by itself.
if (-not $SkipSmoke) {
  $out = Join-Path $env:TEMP "rift-coach-smoke-$PID.txt"
  $err = Join-Path $env:TEMP "rift-coach-smoke-$PID.err.txt"
  $sp = Start-Process -FilePath $info.Exe -ArgumentList '--smoke', '--smoke-seconds=4' -WorkingDirectory $info.Dir -RedirectStandardOutput $out -RedirectStandardError $err -WindowStyle Hidden -PassThru
  $null = $sp.Handle   # keep the handle open, otherwise ExitCode is empty after WaitForExit (PS 5.1)
  if (-not $sp.WaitForExit(90000)) {
    try { $sp.Kill() } catch { }
    Problem 'smoke test did not exit within 90 s (killed)'
  } else {
    $text = if (Test-Path $out) { Get-Content $out -Raw } else { '' }
    if ($sp.ExitCode -eq 0 -and $text -match 'SMOKE-') { Say "smoke test ok: $(([regex]::Matches($text, 'SMOKE-[A-Z]+')).Count) SMOKE lines, exit 0" }
    else { Problem "smoke test failed (exit $($sp.ExitCode)); output: $out" }
  }
}

if ($problems | Where-Object { $_ -notmatch '^(League of Legends not found|missing .*game\.cfg|no AI coach backend|League of Legends game is running|no Riot API key)' }) { Finish 1 }
Finish 0
