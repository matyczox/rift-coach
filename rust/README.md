# Rift Coach — wersja Rust (Tauri)

Port aplikacji z Electrona na Rust + Tauri v2. Interfejs (React) zostaje ten sam: `src/renderer` budowany przez `npm run build` do `dist/renderer`.

- `app/` — aplikacja Tauri: okna, most `window.rc`, źródła danych (Live API, LCU, Replay API), ustawienia, coach (Claude Code / Codex CLI).
- `crates/rc-vision/` — przechwytywanie okna gry, minimapa, wykrywanie portretów, mgła, obozy, paski HP, tracker mgły.

Budowanie: `cargo build --release` w katalogu `rust/` (wymaga MSVC Build Tools z C++), albo bez MSVC: `powershell -ExecutionPolicy Bypass -File app\cargo-gnu.ps1 build --release -p rift-coach`.

## Aplikacja: obraz gry i HUD

- `app/src/vision.rs` — przechwytywanie okna gry (WGC, rc-vision), podgląd minimapy dla zakładki Minimapa (≤10 kl./s, tylko gdy okno sterowania jest widoczne), VisionService (≤5 kl./s, najnowsza klatka wygrywa, wykrywacz na 2 wątkach), tracker mgły (duchy, alerty invade/gank/missing, obozy → obserwacje junglera, powroty po zakupie), paski HP → IntelTracker. Nauka modelu mgły tylko w CLASSIC/SWIFTPLAY na mapie 11. Kalibracja: `grabScene` / `autoCalibrate` / `setCalibration`.
- `app/src/hud.rs` + `hud_state.rs` — JEDNO przezroczyste okno HUD nad obszarem klienta gry (WS_EX_NOACTIVATE | TRANSPARENT | LAYERED | TOOLWINDOW, zawsze na wierzchu, pokazywane bez aktywacji, tylko gdy gra jest aktywnym oknem i jakiś panel ma treść). Strona `src/renderer/static/hud-all.html` + `src/renderer/hud/all.ts` składa istniejące panele HUD bez zmian w ich kodzie.
- `app/src/hotkeys.rs` — globalne skróty z ustawień (przełącz HUD, Flash wroga; nagrywanie jeszcze nieprzeniesione) i wpis autostartu w HKCU\…\Run (tylko build release, nie w `--smoke`).

Flagi testowe: `--smoke` (ukryte okno, coach wyłączony, wypisuje stan po `--smoke-seconds=N`, `--smoke-frame=plik.png` zapisuje wycinek minimapy), `--allow-replay` (powtórka traktowana jak mecz na żywo), `--hud-test` (HUD z ramką minimapy, także gdy gra nie jest aktywna), `--smoke-hotkeys` (rejestruj skróty także w `--smoke`).
