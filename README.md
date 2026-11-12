# Rift Coach

Prywatny overlay‑coach do League of Legends (Windows, Rust/Tauri). Dwa systemy:

1. **Lokalny tracker** — czyta minimapę z ekranu, wykrywa portrety, mgłę wojny, obozy w dżungli i paski HP; przewiduje, gdzie są wrogowie we mgle (półprzezroczyste kółka), ostrzega przed invadem i gankiem, uczy się z własnych meczów i osi czasu z Riot API.
2. **Coach AI** — przez twoje subskrypcje: **Claude Code** (`claude -p`) i/lub **Codex CLI**. Krótka porada na górnym pasku, analiza po śmierci, wezwania do obiektów, build, wardy, runy w wyborze postaci, podsumowanie po meczu.

Dane: Live Client API, Replay API, klient LoL (LCU), Riot Web API (własny klucz deweloperski), Data Dragon.

> Projekt fanowski, niepowiązany z Riot Games. Nie wysyła żadnych klawiszy do gry i nie czyta jej pamięci — tylko obraz z ekranu i oficjalne lokalne API.

## Uruchomienie

Instalator NSIS (także przez agentów AI): [INSTALL.md](INSTALL.md).

Ze źródeł: `npm install` → `npm run build` (frontend do `dist/renderer`) → `npm run app` (Rust, toolchain GNU) → `rust/target/release/rift-coach.exe`.

Wymagania: Windows, Node 22+, Rust, League of Legends. Coach AI potrzebuje zalogowanego Claude Code (`claude auth login`) lub aplikacji Codex. Klucz Riot API wpisujesz w Ustawieniach (zapisywany lokalnie w `%APPDATA%\Rift Coach`).

Replay API (zbieranie danych z powtórek): w `Config\game.cfg` dodaj `EnableReplayApi=1` w sekcji `[General]`.

## Dokumentacja

- [SPEC.md](SPEC.md) — architektura i plan
- [docs/MODELE.md](docs/MODELE.md) — które modele i poziomy rozumowania (Codex / Claude Code / oba)

## Testy

```bash
npm test
npm run typecheck
```
