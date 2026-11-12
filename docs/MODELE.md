# Rift Coach — modele AI i poziomy rozumowania

Ten dokument mówi, **które modele i jakie poziomy rozumowania ustawić**, zależnie od tego, co masz na komputerze: tylko **Codex** (subskrypcja ChatGPT), tylko **Claude Code** (subskrypcja Claude) albo oba.

Aplikacja **sama wykrywa**, co jest dostępne: czy Claude Code jest zalogowany (`claude auth login`) i czy zainstalowana jest aplikacja Codex (Microsoft Store). Wybiera wtedy odpowiedni zestaw. Sprawdza to przy starcie i co 5 minut. Własne ustawienia w Ustawieniach mają pierwszeństwo, o ile dane narzędzie jest dostępne.

## Gotowe zestawy

### Oba: Codex + Claude Code (zalecane)

| Zadanie | Model | Rozumowanie | Czas | Dlaczego |
|---|---|---|---|---|
| Porada w grze (pasek u góry) | GPT-6.1-Terra | high | ~6–7 s | najszybszy bez błędów; „high” jest tak samo szybki jak „low”, a nie myli stanu gry |
| Analiza po śmierci | Opus 5.5 | low | ~9 s | najlepiej rozumie, dlaczego zginąłeś (np. „Yone ma Ignite, wymieniaj po tornado”) |
| Podsumowanie po meczu | Opus 5.5 | medium | ~15–20 s | czytasz po meczu, więc liczy się jakość |
| Build obok minimapy | GPT-6.1-Terra | high | ~11 s | stabilny, sensowny zestaw przedmiotów, poprawne nazwy |
| Brief patcha i matchup (internet) | Opus 5.5 | low | ~15–21 s | najbogatsze wskazówki; robione raz na patch lub mecz |
| Runy i role w drafcie (internet) | GPT-6.1-Terra | high | ~25–35 s | czyta prawdziwe statystyki patcha (op.gg, u.gg, Mobalytics); także runy pod przeciwnika |

### Tylko Claude Code

| Zadanie | Model | Rozumowanie | Czas |
|---|---|---|---|
| Porada w grze | Sonnet 5.5 | medium | ~7–8 s |
| Analiza po śmierci | Opus 5.5 | low | ~9 s |
| Podsumowanie po meczu | Opus 5.5 | medium | ~15–20 s |
| Build | Sonnet 5.5 | medium | ~9 s |
| Brief patcha i matchup | Opus 5.5 | low | ~15–21 s |
| Runy i role w drafcie | Sonnet 5.5 | medium | ~40–60 s |

### Tylko Codex

| Zadanie | Model | Rozumowanie | Czas |
|---|---|---|---|
| Porada w grze | GPT-6.1-Terra | high | ~6–7 s |
| Analiza po śmierci | GPT-6.1-Sol | low | ~7 s |
| Podsumowanie po meczu | GPT-6.1-Sol | medium | ~10–15 s |
| Build | GPT-6.1-Terra | high | ~11 s |
| Brief patcha i matchup | GPT-6.1-Terra | high | ~14 s |
| Runy i role w drafcie | GPT-6.1-Terra | high | ~25–35 s |

## Czego nie używać

- **Poziomów xhigh i max:** 3–10 razy wolniejsze, a odpowiedzi praktycznie te same. Sol dał identyczne porady od low do max; max trwał 30–150 s zamiast 6–30 s. W grze porada sprzed minuty jest bezwartościowa.
- **GPT-6-Luna:** szybka, ale na xhigh i max psuje research (brak winrate, błędy) i częściej pisze analizę śmierci, gdy postać żyje.
- **Haiku 5.5:** wolniejszy od Sonneta na medium i mylił przedmioty (Rumble'owi, postaci AP, zaproponował przedmioty AD).
- **GPT-6-Astra:** odpowiedzi jak Sol, tylko wolniej.
- **Sonnet 5.5 na low:** w buildzie zwrócił niepoprawny JSON. Medium działa stabilnie.

## Jak testowano

- **Dane:** prawdziwy mecz (powtórka rankingowa, Rumble mid vs Yone, 39 min). Model widział to samo, co coach w grze: przedmioty, złoto, timery, przewidywania mgły, brief patcha.
- **Zadania:**
  - porada na lane'ie (3:30),
  - analiza po śmierci (1:47),
  - późna gra (30:00),
  - build,
  - research matchupu Rumble vs Malphite z internetem.
- **Dostęp do internetu:** każdy model miał go we wszystkich zadaniach (Claude Code: WebSearch/WebFetch; Codex: wbudowane wyszukiwanie).
- **Poziomy rozumowania:**
  - Haiku i Sonnet: low, medium, high, xhigh (max pominięty na życzenie);
  - Opus: low, medium;
  - Luna, Terra, Sol: od low do max.
- **Ocena automatyczna (0–10):**
  - czy odpowiedź jest poprawna i mieści się w pasku (≤ 70 znaków);
  - czy nie ma analizy śmierci, gdy postać żyje;
  - czy liczba śmierci i „gank” zgadzają się z danymi;
  - czy przedmioty i miejsca na wardy istnieją;
  - czy build nie powtarza posiadanych przedmiotów;
  - czy research ma sensowny winrate, krzywą siły, wskazówki i źródła.

  Dodatkowo odpowiedzi były czytane ręcznie: trafność buildu do postaci i jakość wniosków.

> Część kar w tabeli to wina promptu, nie modelu, i została już poprawiona. Coach dostaje teraz wprost „żyje / NIE ŻYJE”. Wcześniej modele widziały świeżą śmierć w zdarzeniach i pisały analizę, mimo że postać się odrodziła (kolumna „late”). Angielskie nazwy przedmiotów od Claude'a są teraz rozpoznawane.

### Wyniki (ocena / czas)

| Model | Rozumowanie | Średnio | live 3:30 | śmierć 1:47 | late 30:00 | build | research |
|---|---|---|---|---|---|---|---|
| gpt-6.1-terra | high | 10.0 | 10 · 6 s | 10 · 7 s | 10 · 7 s | 10 · 11 s | 10 · 14 s |
| gpt-6.1-sol | low | 10.0 | 10 · 6 s | 10 · 7 s | 10 · 8 s | 10 · 8 s | 10 · 30 s |
| gpt-6.1-terra | xhigh | 10.0 | 10 · 7 s | 10 · 7 s | 10 · 11 s | 10 · 10 s | 10 · 27 s |
| gpt-6.1-sol | medium | 10.0 | 10 · 12 s | 10 · 10 s | 10 · 9 s | 10 · 8 s | 10 · 32 s |
| gpt-6.1-sol | high | 10.0 | 10 · 13 s | 10 · 12 s | 10 · 22 s | 10 · 22 s | 10 · 76 s |
| claude-sonnet-5-5 | xhigh | 10.0 | 10 · 23 s | 10 · 28 s | 10 · 22 s | 10 · 30 s | 10 · 52 s |
| gpt-6.1-sol | xhigh | 10.0 | 10 · 31 s | 10 · 29 s | 10 · 36 s | 10 · 55 s | 10 · 121 s |
| gpt-6.1-sol | max | 10.0 | 10 · 41 s | 10 · 31 s | 10 · 29 s | 10 · 41 s | 10 · 154 s |
| claude-sonnet-5-5 | high | 9.8 | 10 · 16 s | 10 · 15 s | 9 · 19 s | 10 · 10 s | 10 · 19 s |
| claude-haiku-5-5 | high | 9.8 | 9 · 16 s | 10 · 15 s | 10 · 16 s | 10 · 23 s ⚠ AD na AP | 10 · 47 s |
| claude-haiku-5-5 | xhigh | 9.8 | 9 · 32 s | 10 · 24 s | 10 · 45 s | 10 · 57 s | 10 · 126 s |
| claude-haiku-5-5 | low | 9.6 | 8 · 12 s | 10 · 11 s | 10 · 14 s | 10 · 11 s | 10 · 20 s |
| gpt-6.1-terra | medium | 9.4 | 10 · 5 s | 10 · 7 s | 7 · 7 s | 10 · 8 s | 10 · 17 s |
| gpt-6-luna | medium | 9.4 | 10 · 5 s | 10 · 6 s | 7 · 13 s | 10 · 5 s | 10 · 21 s |
| claude-haiku-5-5 | medium | 9.4 | 10 · 11 s | 10 · 11 s | 7 · 15 s | 10 · 21 s | 10 · 21 s |
| gpt-6.1-terra | max | 9.4 | 10 · 40 s | 10 · 16 s | 7 · 50 s | 10 · 37 s | 10 · 145 s |
| claude-opus-5-5 | low | 9.2 | 10 · 9 s | 10 · 9 s | 6 · 9 s | 10 · 7 s | 10 · 15 s |
| claude-sonnet-5-5 | medium | 9.2 | 10 · 7 s | 10 · 8 s | 6 · 15 s | 10 · 9 s | 10 · 16 s |
| claude-opus-5-5 | medium | 9.0 | 10 · 13 s | 8 · 16 s | 7 · 13 s | 10 · 8 s | 10 · 20 s |
| gpt-6.1-terra | low | 8.8 | 7 · 5 s | 10 · 6 s | 7 · 6 s | 10 · 9 s | 10 · 13 s |
| gpt-6-luna | high | 8.8 | 7 · 16 s | 10 · 11 s | 7 · 19 s | 10 · 21 s | 10 · 32 s |
| gpt-6-luna | low | 8.6 | 6 · 7 s | 10 · 6 s | 7 · 6 s | 10 · 5 s | 10 · 23 s |
| gpt-6-luna | xhigh | 8.4 | 10 · 44 s | 10 · 20 s | 7 · 31 s | 10 · 47 s | 5 · 80 s |
| claude-sonnet-5-5 | low | 8.0 | 10 · 8 s | 10 · 9 s | 10 · 11 s | 0 · 7 s (zły JSON) | 10 · 16 s |
| gpt-6-luna | max | 7.4 | 10 · 32 s | 10 · 34 s | 7 · 32 s | 10 · 79 s | 0 · 126 s (błąd) |

Ocena automatyczna mierzy poprawność i zgodność z danymi, nie głębię wniosków. W ręcznym czytaniu **Opus** konsekwentnie dawał najbardziej użyteczne wnioski (Ignite u przeciwnika, ult z mgły, brak butów, metowe runy pod matchup). Dlatego robi wszystko, przy czym jest czas na czytanie.

## Runy: test na 6 matchupach

Ambessa vs Warwick, Rumble vs Yone, Darius vs Teemo, Ahri vs Zed, Ezreal vs Caitlyn, Jax vs Malphite.

**Prompt.** Wymusza procedurę:
1. Strona bazowa z dużej próbki (Emerald+, tysiące gier; keystone po winrate przy pickrate ≥ 5%).
2. Zmiana keystone'a pod przeciwnika tylko przy ≥ 300 grach i ≥ 2 pkt proc. lepszym winrate.
3. Drobne runy i odłamki pod przeciwnika z jasnym powodem.
4. Lista rzędów run w prompcie.

**Wyniki.**
- **Terra · high:** za każdym razem podała źródło z liczbą gier i winrate'em. Strony są zgodne z danymi patcha (np. Electrocute na Ambessę). Czas 25–50 s.
- **Opus · low:** często nie mógł odczytać stron statystyk (lolalytics ładuje się skryptem) i wracał do pamięci (Conqueror na Ezreala i Dariusa, zgadywane Aery na Ahri). W starym prompcie w ogóle nie szukał.
- **Sonnet · medium:** czyta op.gg poprawnie, ale wolniej (40–60 s).

Wniosek: runy (ogólne i pod przeciwnika) robi model z draftu. Test: `scripts/dev/runes-bench.ts`.

## Ile to kosztuje (mecz 30 min, zestaw „oba”)

- **Porady w grze:** ~60–70 wywołań, ~1,8 tys. nowych tokenów każde (reszta z cache).
- **Analiza po śmierci:** po jednej na każdą śmierć.
- **Build:** ~10–15 wywołań.
- **Brief patcha:** raz na patch. **Matchup:** raz na mecz. **Runy:** raz lub dwa w wyborze postaci.
- **Podsumowanie po meczu:** raz.

## Zmiana ustawień

W **Ustawieniach → Coach** wybierasz modele do każdej roli; jest też pole „inny…” na dowolną nazwę. Nazwy modeli:

- **Claude Code:** `claude-sonnet-5-5`, `claude-opus-5-5`, `claude-haiku-5-5`. Opus 5.5 wymaga Claude Code ≥ 2.1.280 (`claude update`).
- **Codex:** `gpt-6.1-terra`, `gpt-6.1-sol`, `gpt-6-astra`, `gpt-6-luna`.

**Tryb fast:** GPT-6.1-Terra jest domyślnie wywoływana w trybie fast Codexa (`service_tier="fast"`), więc porady w grze przychodzą szybciej, ale zużywają więcej limitu subskrypcji. Lista modeli w trybie fast to `FAST_TIER_MODELS` w `src/main/coach/llm.ts` (oraz w `rust/crates/rc-core/src/llm/codex.rs`).

Poziomy rozumowania są dobierane automatycznie według tego testu (`src/main/coach/profiles.ts`). Test można powtórzyć skryptem `scripts/dev/bench-models.ts` i ocenić skryptem `scripts/dev/bench-score.ts`.
