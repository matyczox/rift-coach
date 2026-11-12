# Instalator Rift Coach (Rust / Tauri)

Instalator NSIS dla wersji Rust: `Rift Coach_<wersja>_x64-setup.exe`.

- instalacja na użytkownika (bez uprawnień administratora) do `%LOCALAPPDATA%\Rift Coach`,
- skróty „Rift Coach” w menu Start i na pulpicie (ikona aplikacji),
- wpis w Ustawienia → Aplikacje (dezinstalator `uninstall.exe`),
- WebView2: wbudowany bootstrapper Microsoftu instaluje środowisko, jeśli go brak (wymaga internetu),
- język polski (gdy Windows jest po polsku), w pozostałych przypadkach angielski,
- `scripts\replay-fog.ps1` trafia do `<folder instalacji>\scripts\` — aplikacja szuka `scripts\replay-fog.ps1` w folderach nadrzędnych exe (`rc-core/src/harvest.rs`, `replay_fog_script()`), więc działa bez zmian w kodzie.

## Budowanie

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-installer.ps1
```

Skrypt robi: `npm ci` → `npm run build` (UI do `dist\renderer`) → `cargo-gnu.ps1` (zamiennik `dlltool` dla toolchainu GNU) → `npx @tauri-apps/cli@2.12.4 build --bundles nsis` w `rust\app`.
Opcje: `-SkipUi` (UI już zbudowane), `-SkipNpmCi` (jest `node_modules`).

Wynik: `rust\target\release\bundle\nsis\Rift Coach_0.1.0_x64-setup.exe` (ostatnia linia wyjścia skryptu to ścieżka). Rozmiar: 5,68 MiB (5 956 055 B dla 0.1.0; zawiera bootstrapper WebView2); po instalacji ok. 11,7 MB (`rift-coach.exe` 11,4 MB, `WebView2Loader.dll`, `uninstall.exe`, `scripts\replay-fog.ps1`). Pierwsze budowanie: ok. 5 min kompilacji Rust.

Wymagania: Node 22+, Rust z toolchainem `stable-x86_64-pc-windows-gnu` (wybierany przez `rust\rust-toolchain.toml`), internet przy pierwszym budowaniu. MSVC **nie** jest potrzebne.

### Dlaczego tak

- `cargo install tauri-cli` nie działa bez MSVC/MinGW: zależność `ring` wymaga kompilatora C. Dlatego używamy gotowej binarki CLI z npm (`@tauri-apps/cli`, wersja przypięta w skrypcie). CLI tylko uruchamia `cargo` i bundler.
- NSIS i wtyczkę `nsis_tauri_utils.dll` Tauri pobiera sam do `%LOCALAPPDATA%\tauri\NSIS` (zwykły `makensis.exe`, bez MSVC). Bootstrapper WebView2 trafia do `%LOCALAPPDATA%\tauri\WebView2*`.
- `nsis\installer.nsi` to kopia szablonu z tauri-bundler 2.10.4 (z tauri-cli 2.12.4) z jedną zmianą: nazwa skrótów `SHORTCUTNAME` = „Rift Coach”. Przy aktualizacji `@tauri-apps/cli` trzeba skopiować nowy szablon i powtórzyć tę zmianę (zamiana `${PRODUCTNAME}.lnk` → `${SHORTCUTNAME}.lnk`).
- `nsis\Polish.nsh` — polskie teksty Tauri (Tauri nie ma własnego tłumaczenia; teksty NSIS/MUI są z NSIS).

## Instalacja z terminala (cicha)

```powershell
& ".\Rift Coach_0.1.0_x64-setup.exe" /S                       # domyślny folder
& ".\Rift Coach_0.1.0_x64-setup.exe" /S /D=C:\Gry\RiftCoach   # /D= na końcu, bez cudzysłowów
& "$env:LOCALAPPDATA\Rift Coach\uninstall.exe" /S             # dezinstalacja (dane w %APPDATA%\Rift Coach zostają)
```

Instalator `/S` zamyka działającą kopię aplikacji z folderu docelowego. Kody wyjścia NSIS: `0` OK, `1` przerwano, `2` błąd skryptu.
Dezinstalator kopiuje się do `%TEMP%` i wraca od razu — trzeba poczekać, aż zniknie folder (robi to `scripts\install.ps1 -Uninstall`).

Pełna, nieinteraktywna instalacja z kontrolą wymagań i testem `--smoke`: `scripts\install.ps1` — opis w [INSTALL.md](../../INSTALL.md).
