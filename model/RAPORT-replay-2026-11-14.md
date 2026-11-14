# Trening trackera z powtórek — 2026-11-14

**Wyniki zastąpione audytem:** surowe etykiety zawierały fałszywe pozycje. Aktualne pomiary i ograniczenia są w [RAPORT-ganki-2026-11-14.md](RAPORT-ganki-2026-11-14.md).

## Wynik

Wytrenowano kandydata na obu pełnych powtórkach: **922 etykiety treningowe**.
**Kandydat nie został włączony do aplikacji.** Test między meczami nie wykazał stabilnej przewagi nad trackerem bez wyuczonych mnożników: 5–15, 15–30 oraz 60–120 s pogorszyły się; poprawa 30–60 s jest mniejsza niż 1 punkt procentowy. Pozostawiono dotychczasowe aktywne wagi.

## Zebrane dane

Każdy mecz odtworzono od początku do końca dwukrotnie. `frames/` to mgła niebieskich (wejście), `red/` to mgła czerwonych (prawdziwe pozycje czerwonych). Przeciwników niewidocznych w widoku niebieskich nie przekazywano trackerowi jako wykryć.

| Mecz | Czas | Klatki niebieskich | Klatki czerwonych | Klatki wejścia z prawdą ≤300 ms |
|---|---:|---:|---:|---:|
| 4028222906 | 31:39.798 | 6432 | 2407 | 4831 |
| 4028237949 | 21:13.517 | 1613 | 1630 | 1049 |

Łączny czas obu meczów: 52:53.315. API pierwszego meczu obejmuje 0.033–1899.386 s, drugiego 0.033–1272.922 s. Rzadsze klatki czerwonych pierwszego meczu pochodzą z oddzielnie dokończonego przebiegu. Jego kompletny strumień API zastąpił niezamknięty strumień przerwanego kolektora.

Mediana różnicy pozycji przeciwników widocznych w obu widokach: 0.30% szerokości mapy w każdym meczu. Percentyl 95: odpowiednio 1.32% i 2.99%. Prawda pochodzi z detektora ikon, nie z wewnętrznych współrzędnych silnika gry; nakładające się portrety i błędy detekcji ograniczają jakość etykiet.

Ikony pozyskano z lokalnych archiwów gry skryptem `scripts/extract-minimap-icons.mjs`. Przeliczono wykrycia aktualnym `detect_cache`, z `STRICT_ICONS=1`.

## Metoda i wyniki

Dwa testy: uczenie od neutralnych mnożników na meczu A, test na B; następnie odwrotnie. W treningu prawdziwe pozycje we mgle co 10 s oraz zwykłe ponowne pojawienia. Mnożniki osłabiono do `sqrt(multiplier)` (połowa zmiany w logarytmie). Podczas testów `TRAIN` jest wyłączone, a model nie jest zapisywany. Końcowy kandydat powstał dopiero po testach, przez uczenie na obu meczach, po czym zastosowano tę samą regularyzację. Wyników treningu końcowego nie przedstawiamy jako niezależnego testu.

Błąd to odległość euklidesowa podzielona przez szerokość mapy. Mniej znaczy lepiej. Agregacja waży średnią liczbą ocenionych klatek; liczby nie są procentem skutecznie ostrzeżonych ganków.

| Czas we mgle | Oceny | Ostatnia widziana pozycja | Neutralne wagi | Uczony na drugim meczu | Dotychczasowe aktywne* |
|---|---:|---:|---:|---:|---:|
| 5–15 s | 3043 | 25.56% | 24.65% | 24.95% | 24.83% |
| 15–30 s | 2052 | 41.40% | 32.25% | 33.02% | 32.50% |
| 30–60 s | 1544 | 44.51% | 32.57% | 32.25% | 34.73% |
| 60–120 s | 623 | 46.47% | 19.20% | 20.35% | 23.88% |

*Dotychczasowy model uczył się wcześniej na tych meczach na żywo, więc jego kolumna służy tylko porównaniu opisowemu. Nie jest niezależnym testem generalizacji. W przedziałach 5–120 s liczby ocen i błędy prostej metody bazowej są takie same dla wariantów. Drobne różnice pokrycia poniżej 5 s nie służyły do wyboru modelu.

Tylko dwa mecze i skorelowane kolejne klatki nie pozwalają dowieść poprawy ogólnej ani podać wiarygodnej skuteczności ostrzeżeń o gankach. Nie dobierano kolejnych wariantów pod uzyskane wyniki.

## Naprawione błędy

- Weryfikacja mgły odrzucała ciemnoczerwony napis przez próg sumy jasności RGB=330, mimo prawidłowego przełączenia. Teraz liczy piksele liter według koloru, również dla ciemniejszego neutralnego napisu. Zweryfikowano na otwartej powtórce: `OK fog blue`, `OK fog red`, `OK fog all`.
- Narzędzie `eval_tracker` odczytywało przesunięcia API wyłącznie jako całkowite JSON. Harvester zapisuje także `12000.0`; odczyt zamieniał je na zero i dla całego meczu wybierał końcowy stan API. Dodano obsługę obu formatów i test regresji. Wszystkie powyższe wyniki policzono ponownie po poprawce.
- Dodano jawny tryb diagnostyczny `--smoke-harvest-auto`, używający istniejącego skryptu wyłącznie do przełączenia mgły. Zwykły `--smoke-harvest` nadal nie wysyła wejścia do okna gry.

## Odtworzenie

Budowanie z katalogu repo:

```powershell
powershell -ExecutionPolicy Bypass -File rust/app/cargo-gnu.ps1 build --release -p rc-vision --example detect_cache --example eval_tracker
powershell -ExecutionPolicy Bypass -File rust/app/cargo-gnu.ps1 test -p rc-vision --example eval_tracker api_offsets
powershell -ExecutionPolicy Bypass -File rust/app/cargo-gnu.ps1 test -p rc-core harvest::tests --lib
```

Dla każdej lokalnej sesji: `detect_cache.exe <session> frames red` (z `STRICT_ICONS=1`). Ustawić `TRUTH=red`. Ocena: `MODEL=<plik-wag>` i `eval_tracker.exe <session>`, bez zmiennej `TRAIN`. Trening: skopiować `model/astra-best.json` do nowego pliku (neutralny start), ustawić `MODEL=<nowy-plik>` oraz `TRAIN=1`, uruchomić `eval_tracker.exe <session-treningowa>`. Następnie usunąć `TRAIN`, zastąpić każdy mnożnik jego pierwiastkiem i ocenić na drugiej sesji. Powtórzyć z zamienionymi meczami.

Sesje zachowano lokalnie w katalogu roboczym tego czatu `work/replay-truth/`; surowe dane meczowe nie są publikowane w repo. Szczegółowe wyniki i kontrola danych: `replay-results-2026-11-14.json`. Kandydat: `replay-candidate-2026-11-14.json`. Plik kandydata jest artefaktem eksperymentu, aplikacja go automatycznie nie ładuje.

## Weryfikacja i uruchomienie

Testy harwestera: 2/2, test regresji czasu API: 1/1. Kompilacja release zakończona pomyślnie. Test uruchomienia aplikacji (8 s): kod wyjścia 0, LCU połączone, aktywny model 414 próbek. Zainstalowano nowy plik wykonywalny i skrypt mgły oraz uruchomiono aplikację ponownie. Hash zainstalowanego pliku jest zgodny z kompilacją; hash aktywnych wag pozostał bez zmian. Przywrócono kalibrację sprzed zbierania powtórek. Ustawienia coacha nadal: GPT-6.1 Terra, fast=true.
