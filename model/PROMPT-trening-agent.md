Masz dostęp do repo `C:\Users\Admin\Desktop\ligalarp` (Windows, Git Bash + PowerShell). To Rift Coach: nakładka do League of Legends w Rust/Tauri. Twoje zadanie: SAMODZIELNIE wytrenować i dobrać wagi trackera mgły wojny, sprawdzić je uczciwie i oddać wynik. Nie wdrażaj niczego do aplikacji.

## Czym jest tracker
`rust/crates/rc-vision/src/tracker/` — filtr cząsteczkowy przewidujący, gdzie jest przeciwnik, który zniknął we mgle. Hipotezy "idzie do celu X" ważone są tabelą bazową (`predictor.rs`, `fn table` i `fn goal_prior`) × wyuczone mnożniki (`learning.rs`, `LearnedModel.multipliers`, klucz "ROLA|faza", np. "JUNGLE|early"; rodzaje celów: camp, lane-top, lane-mid, lane-bot, dragon, baron, base, invade, river, stay; zakres 0.2–5.0; fazy: early <14 min, mid 14–25, late >25). Format pliku wag: zobacz `model/priors-v1.json` (to wagi napisane przez Claude — punkt odniesienia).

## Dane
3 powtórki w `%APPDATA%\Rift Coach\recordings\`:
- `replay-4027208890`
- `replay-unknown-2026-11-12T09-21-59`
- `replay-unknown-2026-11-12T01-27-38`
Każda ma klatki minimapy z widoku niebieskich (`frames/`, to "widzi gracz"), czerwonych (`red/`, prawda dla czerwonych postaci) i czasem bez mgły (`all/`), plus `api.jsonl.gz` (Live API). Wykrycia portretów są już przeliczone obecnym detektorem (`detections-*.json`). NIE nadpisuj plików w tych folderach poza `detections-*.json`, i to tylko jeśli przeliczasz je narzędziem `detect_cache`.

## Narzędzia (już są)
Budowanie (toolchain GNU, z katalogu repo, PowerShell):
  powershell -ExecutionPolicy Bypass -File rust\app\cargo-gnu.ps1 build --release -p rc-vision --example eval_tracker --example detect_cache
- `rust/target/release/examples/eval_tracker.exe <sesja>` — ocena trackera na powtórce. Zmienne: `TRUTH=red` (prawda = widok czerwonych), `MODEL=<plik.json>` (start z tych wag; bez TRAIN tylko ocena, plik nie jest zmieniany), `TRAIN=1` (uczy się na tej powtórce z prawdziwych pozycji co 10 s i zapisuje do MODEL). Na końcu wypisuje linię `SUMMARY {json}` z błędem modelu (`errModel`) i prostej metody "stoi, gdzie go widziano" (`errBase`) w przedziałach czasu we mgle.
- `rust/target/release/examples/detect_cache.exe <sesja>` — przelicza wykrycia (kilkanaście sekund na powtórkę). Potrzebuje ikon z raw.communitydragon.org; jeśli serwer nie odpowiada, bierze kwadratowe (`STRICT_ICONS=1` zatrzymuje zamiast tego).
- `scripts/dev/loo-tracker.sh` — test leave-one-out: każdy mecz po kolei testowy, uczenie na 2 pozostałych, 4 warianty (bez wag / priors-v1 / uczenie od zera / priors + uczenie). Uruchom go najpierw, żeby mieć punkt odniesienia; możesz go skopiować i rozszerzyć o swoje warianty.

## Co zrobić
1. Zbuduj narzędzia i odpal `scripts/dev/loo-tracker.sh`, zapisz tabelę odniesienia.
2. Napisz własne wagi z wiedzy o grze (`model/astra-v1.json`) i/lub dobierz je eksperymentalnie (np. przeszukiwanie, uczenie z różnymi startami, regularyzacja, łączenie). Możesz pisać pomocnicze skrypty w `scripts/dev/` albo w katalogu tymczasowym.
3. KAŻDY wariant oceniaj tylko leave-one-out (mecz testowy nigdy nie może być użyty do uczenia ani do strojenia wag pod niego). Strojenie pod te same 3 mecze, na których potem raportujesz, to przeuczenie — jeśli coś stroisz, opisz uczciwie, na czym.
4. Kryterium: wariant jest lepszy tylko jeśli łącznie na 3 meczach testowych ma mniejszy błąd w przedziałach 15–30, 30–60 i 60–120 s (długie odcinki są najważniejsze) i nie pogarsza 5–15 s. Różnice poniżej ~1 punktu procentowego traktuj jako szum.

## Ograniczenia
- Nie zmieniaj kodu produkcyjnego (`rust/app`, `rust/crates/*/src`, `src/`) ani testów. Wolno dodawać pliki w `model/` i `scripts/dev/`.
- Nie ruszaj `%APPDATA%\Rift Coach\model\` (model używany przez aplikację), ustawień ani działającej aplikacji. Nie uruchamiaj gry ani aplikacji.
- Nie commituj i nie pushuj; zostaw pliki w working tree.
- Gdy gra League of Legends jest uruchomiona (proces "League of Legends.exe"), wstrzymaj ciężkie obliczenia.

## Co oddać
1. Najlepszy plik wag (`model/astra-best.json`) — albo jasną informację, że żaden nie bije trackera bez wag.
2. Krótki raport (`model/RAPORT-astra.md`): tabela leave-one-out dla wszystkich wariantów (błąd w % dla 5–15, 15–30, 30–60, 60–120 s + baseline), co próbowałeś, co pomogło, czego nie jesteś pewien.
3. Dokładne komendy, które pozwalają odtworzyć wynik.
