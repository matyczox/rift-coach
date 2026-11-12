# Installing Rift Coach (Rust / Tauri build)

This file is written for coding agents (Claude Code, Codex) installing Rift Coach on a user's PC.
Every command is non-interactive. Run them in PowerShell from the repository root unless noted.

Rift Coach is a League of Legends overlay coach for Windows. It reads the game only through the
screen (minimap) and Riot's official local APIs; it never sends input to the game or reads its memory.

## 1. Prerequisites

| Requirement | Check | Required? |
|---|---|---|
| Windows 10/11, x64 | `[Environment]::OSVersion.Version`, `[Environment]::Is64BitOperatingSystem` | yes |
| WebView2 runtime | installed on all current Windows 11 / most Windows 10; the installer adds it if missing (internet) | yes (auto) |
| League of Legends | folder with `LeagueClient.exe`, usually `C:\Riot Games\League of Legends` | yes |
| `Config\game.cfg` with `EnableReplayApi=1` in `[General]` | `install.ps1` reports `replayApi` | only for learning from replays |
| Claude Code CLI, logged in | `claude auth status` → `"loggedIn": true` | one AI backend needed |
| Codex app | `Get-AppxPackage OpenAI.Codex` | alternative AI backend |
| Riot API developer key | user gets it at https://developer.riotgames.com | strongly recommended (see section 6) |

Only to build from source (not needed with a ready installer): Node.js 22+ with npm 10+ (tested Node 24 / npm 11; with an older npm `npm ci` may report the lock file out of sync - use `npm install`, the build script falls back to it automatically), `npm`, Rust via rustup with
the `stable-x86_64-pc-windows-gnu` toolchain (`rustup toolchain install stable-x86_64-pc-windows-gnu`),
internet access. MSVC / Visual Studio is **not** needed.

## 2. Install (one command)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1
```

What it does, in order:
1. Checks Windows version, WebView2, League path and `game.cfg`, Claude Code login, Codex app.
2. Uses `-Installer <path>` if given; otherwise the newest `rust\target\release\bundle\nsis\*-setup.exe`;
   otherwise builds it from source (`scripts\build-installer.ps1`: `npm ci`, `npm run build`, cargo GNU build, NSIS).
3. Installs silently, per user, no admin: `setup.exe /S [/D=<dir>]` → `%LOCALAPPDATA%\Rift Coach`.
4. Starts the installed exe with `--smoke` (hidden window, coach off, exits by itself after a few seconds).
5. Prints one JSON line as the **last line** of output and exits with a status code.

Useful options (all optional; the script never prompts):

```powershell
# use a downloaded/copied installer
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1 -Installer "C:\path\Rift Coach_0.1.0_x64-setup.exe"
# force a fresh build from this repo
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1 -FromSource
# turn on the Replay API (backs up game.cfg to game.cfg.bak-<timestamp>; refuses while a game is running)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1 -EnableReplayApi
# League in a non-standard folder / custom install folder
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1 -LeaguePath "D:\Games\League of Legends" -InstallDir "D:\Apps\Rift Coach"
# store the user's Riot key (written to %APPDATA%\Rift Coach\settings.json, never printed)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1 -RiotKey $env:RIOT_KEY
# only check prerequisites
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1 -CheckOnly
```

Do not put the Riot key in a command line that gets logged or echoed back to the user; pass it from an
environment variable or let the user paste it into the app (Settings) later.

### Result line

```json
{"ok":true,"installedTo":"C:\\Users\\me\\AppData\\Local\\Rift Coach","webview2":true,"claude":"logged-in","codex":true,"replayApi":true,"problems":[]}
```

- `claude`: `logged-in` | `not-logged-in` | `missing`.
- `problems`: human-readable strings. Some are warnings that do not fail the install
  (League not found, `game.cfg` missing, no AI backend, game running) — relay them to the user.

Exit codes: `0` installed and smoke test passed (or `-CheckOnly` / `-Uninstall` succeeded),
`1` failed (see `problems`), `2` unsupported system.

## 3. Raw installer (without the script)

```powershell
& ".\Rift Coach_0.1.0_x64-setup.exe" /S                        # silent, default folder
& ".\Rift Coach_0.1.0_x64-setup.exe" /S /D=D:\Apps\RiftCoach    # /D= must be last, no quotes
```

NSIS exit codes: `0` success, `1` aborted, `2` script error. `/S` closes a running copy installed in the
same folder. The install is registered in `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Rift Coach`
(`InstallLocation`, `MainBinaryName`). Shortcuts: "Rift Coach (Rust)" in the Start menu and on the desktop.

## 4. Verify

```powershell
$dir = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Rift Coach').InstallLocation.Trim('"')
$p = Start-Process "$dir\rift-coach.exe" -ArgumentList '--smoke','--smoke-seconds=4' -RedirectStandardOutput "$env:TEMP\rc-smoke.txt" -WindowStyle Hidden -PassThru -Wait
$p.ExitCode                        # 0
Select-String SMOKE- "$env:TEMP\rc-smoke.txt"   # SMOKE-VISION / SMOKE-HUD / SMOKE-HOTKEYS lines
Test-Path "$dir\scripts\replay-fog.ps1"         # True
```

`--smoke` runs next to a normal instance and never shows a window. Do not start the app without
`--smoke` on the user's behalf while they are playing.

## 5. Uninstall

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install.ps1 -Uninstall
```

or directly `& "$env:LOCALAPPDATA\Rift Coach\uninstall.exe" /S` (it re-launches itself from `%TEMP%`
and returns immediately; wait until the folder is gone). User data in `%APPDATA%\Rift Coach`
(settings, recordings, learned model) is kept; delete that folder only if the user asks.

## 6. What the user must do themselves

An agent cannot (and must not) do these:
- **Log in to an AI backend**: run `claude auth login` in a terminal (Claude subscription), or install and
  sign in to the Codex app. Without one, the tracker works but the AI coach is silent.
- **Riot API key**: create a development key at https://developer.riotgames.com (free, **expires every 24 h**,
  must be renewed) and paste it into Rift Coach → Settings. Never ask for it in chat logs.
  **Tell the user explicitly what does NOT work without a valid key:**
  - enemy profiles on the loading screen (rank, form, aggression, jungle pathing) and their use by the coach,
  - the player profile (own habits and weak spots the coach watches),
  - self-training after each game (true positions from Riot timelines).
  Without the key the tracker, minimap ghosts, AI coach calls, death/post-game reviews and runes still work.
- **Play Summoner's Rift** (Ranked / Normal / Swiftplay) for coaching and learning; ARAM and Arena are ignored.
- Start League once so `Config\game.cfg` exists; for replay learning restart the client after `-EnableReplayApi`.

---

## Po polsku (dla ludzi)

1. Pobierz `Rift Coach_<wersja>_x64-setup.exe` i uruchom go (bez uprawnień administratora). Skrót „Rift Coach (Rust)” pojawi się na pulpicie i w menu Start.
2. Albo w terminalu w folderze repozytorium: `powershell -ExecutionPolicy Bypass -File scripts\install.ps1` — sprawdzi wymagania, zbuduje i zainstaluje aplikację, a na końcu wypisze podsumowanie.
3. Zaloguj się do Claude Code (`claude auth login`) albo zainstaluj aplikację Codex — bez tego coach AI milczy.
4. Opcjonalnie: klucz Riot API (https://developer.riotgames.com, ważny 24 h) wklej w Ustawieniach aplikacji.
5. Żeby aplikacja uczyła się z powtórek: `scripts\install.ps1 -EnableReplayApi` (dopisze `EnableReplayApi=1` do `game.cfg`, zrobi kopię zapasową).
6. Graj na Summoner's Rift — ARAM i Arena są pomijane.
7. Odinstalowanie: Ustawienia → Aplikacje → Rift Coach, albo `scripts\install.ps1 -Uninstall`. Twoje dane w `%APPDATA%\Rift Coach` zostają.

Szczegóły budowania instalatora: [rust/packaging/README.md](rust/packaging/README.md).
