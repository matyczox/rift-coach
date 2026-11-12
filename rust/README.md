# Rift Coach — wersja Rust (Tauri)

Aplikacja Rust + Tauri v2 (jedyna wersja). Interfejs (React) zostaje ten sam: `src/renderer` budowany przez `npm run build` do `dist/renderer`.

- `app/` — aplikacja Tauri: okna, most `window.rc`, źródła danych (Live API, LCU, Replay API), ustawienia, coach (Claude Code / Codex CLI).
- `crates/rc-vision/` — przechwytywanie okna gry, minimapa, wykrywanie portretów, mgła, obozy, paski HP, tracker mgły.

Budowanie: `cargo build --release` w katalogu `rust/` (wymaga MSVC Build Tools z C++), albo bez MSVC: `powershell -ExecutionPolicy Bypass -File app\cargo-gnu.ps1 build --release -p rift-coach`.

## Aplikacja: obraz gry i HUD

- `app/src/vision.rs` — przechwytywanie okna gry (WGC, rc-vision), podgląd minimapy dla zakładki Minimapa (≤10 kl./s, tylko gdy okno sterowania jest widoczne), VisionService (≤5 kl./s, najnowsza klatka wygrywa, wykrywacz na 2 wątkach), tracker mgły (duchy, alerty invade/gank/missing, obozy → obserwacje junglera, powroty po zakupie), paski HP → IntelTracker. Nauka modelu mgły tylko w CLASSIC/SWIFTPLAY na mapie 11. Kalibracja: `grabScene` / `autoCalibrate` / `setCalibration`.
- `app/src/hud.rs` + `hud_state.rs` — JEDNO przezroczyste okno HUD nad obszarem klienta gry (WS_EX_NOACTIVATE | TRANSPARENT | LAYERED | TOOLWINDOW, zawsze na wierzchu, pokazywane bez aktywacji, tylko gdy gra jest aktywnym oknem i jakiś panel ma treść). Strona `src/renderer/static/hud-all.html` + `src/renderer/hud/all.ts` składa istniejące panele HUD bez zmian w ich kodzie.
- `app/src/hotkeys.rs` — globalne skróty z ustawień (przełącz HUD, nagrywanie, Flash wroga) i wpis autostartu w HKCU\…\Run (tylko build release, nie w `--smoke`; nigdy nie nadpisuje wpisu „Rift Coach” wskazującego na inny program, np. wersję Electron).

## Aplikacja: wiedza, po meczu, nagrania

- `app/src/knowledge.rs` — profile wrogów z Riot API (raz na mecz, klucz z ustawień, nigdy nie logowany), brief patcha (raz na patch), matchup linii (raz na parę postaci), panel buildu, wywołania sytuacyjne (fed / zagrożenie / obiekt) dla HUD, `applyModels` (modele researchu, tryb fast Codexa). Coach tylko na mapie 11.
- `app/src/postgame.rs` — analiza po meczu (tylko CLASSIC/SWIFTPLAY/PRACTICETOOL, ≥ 5 min; zapis do `reviews/`, zdarzenie `review`) i samouczenie modelu mgły z obserwacji + osi czasu Riot API (`ObsTracker` dla trackera z rc-vision). Nauka nigdy z powtórek.
- `app/src/recorder.rs` — nagrywanie meczu (ten sam układ folderów co TS; sceny jako PNG w połowie rozdzielczości), auto-nagrywanie, sprzątanie po 30 s od startu (5 najnowszych / 14 dni).
- `app/src/harvest.rs` — zbieranie z powtórek (kolejka przez klienta League, mgła przełączana przez rc-core), `app/src/playback.rs` — odtwarzanie własnych nagrań.

Flagi testowe: `--smoke` (ukryte okno, HUD nigdy nie pokazywany, coach wyłączony, wypisuje stan po `--smoke-seconds=N`, `--smoke-frame=plik.png` zapisuje wycinek minimapy), `--allow-replay` (powtórka traktowana jak mecz na żywo, bez nauki), `--hud-test` (HUD z ramką minimapy, także gdy gra nie jest aktywna), `--smoke-hotkeys` (rejestruj skróty także w `--smoke`), `--smoke-show-hud`, `--smoke-record`, `--smoke-harvest` (jedno przejście „all” bez klikania w HUD powtórki), `--recordings-root=<dir>` (nagrania i zbieranie poza prawdziwym folderem), `--reviews-dir=<dir>`.
