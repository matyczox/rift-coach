# Rift Coach — wersja Rust (Tauri)

Port aplikacji z Electrona na Rust + Tauri v2. Interfejs (React) zostaje ten sam: `src/renderer` budowany przez `npm run build` do `dist/renderer`.

- `app/` — aplikacja Tauri: okna, most `window.rc`, źródła danych (Live API, LCU, Replay API), ustawienia, coach (Claude Code / Codex CLI).
- `crates/rc-vision/` — przechwytywanie okna gry, minimapa, wykrywanie portretów, mgła, obozy, paski HP, tracker mgły.

Budowanie: `cargo build --release` w katalogu `rust/` (wymaga MSVC Build Tools z C++).
