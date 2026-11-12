# Wynik treningu trackera mgły — 12.11.2026

**Nie znalazłem wag spełniających wymagane kryterium przewagi nad trackerem bez wyuczonych mnożników.**
`astra-best.json` jest celowo modelem neutralnym (`multipliers: {}`), równoważnym `none`.
Nie przedstawiam go jako poprawionego modelu. Własny kandydat pozostaje w `astra-v1.json`.
Nic nie zostało wdrożone do aplikacji.

Błąd to średnia odległość euklidesowa w układzie minimapy ×100: procent **boku mapy**, nie procent błędnych wskazań.
Każda komórka poniżej oznacza **model / baseline „ostatnia widziana pozycja”**, w %; mniej znaczy lepiej.
Wynik łączny = `sum(n × err) / sum(n)` osobno dla każdego przedziału czasu we mgle.

| Wariant | 5–15 s | 15–30 s | 30–60 s | 60–120 s |
|---|---:|---:|---:|---:|
| none | 20.38 / 23.43 | 30.01 / 42.61 | 42.11 / 51.28 | 42.35 / 51.35 |
| priors | 20.25 / 23.44 | 30.49 / 42.61 | 41.39 / 51.28 | 42.18 / 51.35 |
| zero+train | 20.05 / 23.44 | 30.39 / 42.61 | 40.24 / 51.28 | 43.34 / 51.35 |
| priors+train | 19.75 / 23.44 | 30.59 / 42.61 | 41.78 / 51.28 | 41.65 / 51.35 |
| astra | 20.25 / 23.44 | 30.14 / 42.61 | 39.86 / 51.28 | 42.67 / 51.35 |
| astra+train | 20.24 / 23.44 | 30.93 / 42.61 | 42.50 / 51.28 | 47.05 / 51.35 |
| bag-zero-a025 | 20.01 / 23.43 | 29.71 / 42.61 | 40.26 / 51.28 | 43.69 / 51.35 |
| bag-zero-a050 | 20.01 / 23.44 | 31.15 / 42.61 | 41.93 / 51.28 | 44.10 / 51.35 |
| bag-zero-a100 | 19.89 / 23.44 | 30.40 / 42.61 | 42.47 / 51.28 | 45.10 / 51.35 |
| bag-astra-a050 | 19.99 / 23.43 | 31.20 / 42.61 | 44.34 / 51.28 | 46.23 / 51.35 |
| nested | 20.38 / 23.43 | 30.01 / 42.61 | 42.11 / 51.28 | 42.35 / 51.35 |

Liczby próbek `none` w kolejnych kolumnach: **7705, 3727, 2426, 1344**. Wszystkie warianty mają identyczne liczebności i baseline w trzech długich przedziałach.
W 5–15 s niektóre warianty tracą 1–4 próbki przez brak estymaty; dlatego tabela podaje ich własny baseline.
Reguła wyboru odrzuca warianty z różną liczebnością lub średnią baseline. To kontrola porównywalności, nie dowód identyczności identyfikatorów obserwacji — standardowy ewaluator ich nie zapisuje.
Wynik negatywny nie zależy od tej dodatkowej ostrożności: żaden kandydat nie poprawia wszystkich trzech długich przedziałów nawet bez niej.

Pełna tabela wszystkich **3 meczów × 11 wariantów × 4 przedziałów**, wraz z liczebnościami i baseline, jest w `astra-loo-detail.csv`.
`astra-results.json` zawiera także średnie liczone z równą wagą każdego meczu, surowe SUMMARY i decyzje selektora.

**Metoda i uczciwość oceny**

- Najpierw uruchomiłem oryginalny `scripts/dev/loo-tracker.sh`: `none`, `priors-v1`, trening od zera oraz od `priors-v1`. `+train` oznacza przejście raz przez każdy z dwóch innych meczów, w kolejności G1, G2, G3 z pominięciem testowego.
- Przed odczytaniem wyników bazowych ustaliłem `astra-v1.json`, siły regularyzacji i regułę wyboru w `astra-protocol.json`. Potem nie dostrajałem wag ani kandydatów na podstawie wyników testowych.
- `astra` to hipoteza: lekko większe `stay`, więcej wczesnego farmienia dżungli, późniejsze przejście ADC/supporta na mid i większa waga bocznych linii u solo-lanerów. Bez silnego dodatkowego podbijania smoków/barona: kod już mnoży ich priory przez 4 przy dostępnym/nadchodzącym celu. `astra+train` stosuje te priory jako początek istniejącego ucznia.
- `bag-*`: każdy mecz treningowy uczy osobny model od tego samego startu; potem średnia geometryczna mnożników i regularyzacja ku startowi: `log(m) = (1−α)log(start) + α·mean(log(m_j))`. α = 0,25 / 0,50 / 1,00 dla startu neutralnego; α = 0,50 dla startu Astra. Brakujące mnożniki = 1; zakres 0,2–5,0. To uśrednianie wag, nie predykcji.
- Każda ocena ma wyłączone `TRAIN`; żaden model testowany na meczu nie był na nim uczony. Wyników z uruchomień treningowych nie używam w tabeli ani do wyboru.
- `nested`: wewnątrz każdej zewnętrznej pary treningowej odbywa się jeszcze LOO (ucz na jednym, sprawdź na drugim, potem zamień). Kandydat musi obniżyć każdy długi błąd o ≥1 p.p. i nie zwiększyć 5–15 s. Dalej rozstrzyga średnia długich błędów z wagami 1:2:3. Gdy brak kandydata, wybór to `none`. Wybory: **G1: `none`, G2: `none`, G3: `none`**.
- Zwykłe porównanie wielu kandydatów na tych samych 3 meczach pozostaje eksploracyjne. Wiersz `nested` ocenia procedurę wyboru bez użycia zewnętrznego testu do strojenia. Nie przedstawiam najlepszego pojedynczego wyniku jako potwierdzenia generalizacji. Końcowy neutralny model wynika z tej samej reguły zastosowanej do wszystkich trzech wyników cross-validation.

**Co pomogło, a co nie**

`astra` obniża 30–60 s o 2.24 p.p., ale pogarsza 15–30 s o 0.12 i 60–120 s o 0.32 p.p.
`bag-zero-a025` poprawia 15–30 s tylko o 0.30 p.p. (szum), 30–60 s o 1.85, lecz pogarsza 60–120 s o 1.34 p.p.
Uczenie sekwencyjne od zera daje poprawę 30–60 s o 1.86 p.p., ale pogorszenie 60–120 s o 0.99 p.p.
Silniejsze uczenie/łączenie nie daje stabilnej poprawy; `astra+train` pogarsza 60–120 s aż o 4.70 p.p.
Różnice mniejsze od około 1 p.p. traktuję jako szum; nie obliczam pozornie precyzyjnych przedziałów ufności z zależnych klatek.

**Ograniczenia danych**

G1 = `replay-4027208890`; G2 = `replay-unknown-2026-11-12T09-21-59`; G3 = `replay-unknown-2026-11-12T01-27-38`.

| Mecz | Długość | Klatki blue/red | Rozbieżność >10% przy wspólnych wykryciach |
|---|---:|---:|---:|
| G1 | 23:59 | 3692/3685 | 0.09% |
| G2 | 16:02 | 2498/2495 | 2.67% |
| G3 | 38:55 | 5922/5929 | 0.57% |

Tylko G3 ma fazę `late` (≥25 min). Przy testowaniu G3 pozostałe mecze nie dostarczają treningu tej fazy.
„Prawda” to wykrycia detektora z widoku czerwonych, nie ręcznie oznaczone pozycje. Rozbieżności między widokami mogą obejmować błędy detekcji i synchronizacji; audyt nie pozwala jednoznacznie rozdzielić ich przyczyn.
Nie odrzucałem żadnego meczu, nie przeliczałem wykryć i nie zmieniałem danych po zobaczeniu wyników.
Klatki z jednego odcinka mgły są silnie zależne; trzy mecze nie wystarczają do potwierdzenia uniwersalnych wag dla ról, faz i różnych stylów gry.

**Odtworzenie (PowerShell, nowe RunDir przy każdym pełnym przebiegu)**

```powershell
Set-Location 'C:\Users\Admin\Desktop\ligalarp'
py -3 -m pip install psutil==7.2.2
$dataAppData = 'C:\Users\Admin\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming'
$runDir = Join-Path $env:TEMP ('rift-coach-astra-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
powershell -ExecutionPolicy Bypass -File scripts\dev\astra-reproduce.ps1 -DataAppData $dataAppData -RunDir $runDir
py -3 scripts/dev/astra_report.py --out $runDir
```

Wrapper buduje narzędzia dokładną komendą:
```powershell
powershell -ExecutionPolicy Bypass -File rust\app\cargo-gnu.ps1 build --release -p rc-vision --example eval_tracker --example detect_cache
```
Następnie kopiuje ewaluator do `RunDir/bin`, uruchamia kopię `loo-tracker.sh` ze zmienioną wyłącznie ścieżką `E`, wykonuje eksperyment, audyt i weryfikację.
`APPDATA` jest ustawiane tylko dla procesów potomnych. Nie ma dostępu do modelu aplikacji. `TRUTH=red`; `TRAIN`, `MODEL`, `CAMPS`, `NO_ITEMS` są czyszczone i jawnie ustawiane dla każdego uruchomienia.
Trening zapisuje wyłącznie kopie wag w RunDir. Monitor co 0,25 s wstrzymuje własne procesy obliczeniowe, gdy działa `League of Legends.exe`, i wznawia po zakończeniu gry. Mechanizm suspend/resume sprawdzono na własnym procesie pomocniczym.

Do odtworzenia dokładnie na zachowanej binarce, bez ponownej kompilacji, dodaj do wywołania wrappera:
```powershell
-PinnedEvaluator 'C:\Users\Admin\Documents\Codex\2026-11-12\masz-dost-p-do-repo-c\work\astra-20261112\bin\eval_tracker.exe'
```
Jej SHA-256: `ef7ea33f62c6448d2192a990c7cceb18fcabd8cccd08ccc69384904e101001ce`. Kopia jest również w archiwum dowodowym, w `run/bin/eval_tracker.exe`.
Surowe logi, niezależne modele treningowe, modele poszczególnych foldów, manifest wejść i 24 wewnętrzne oceny są w `C:\Users\Admin\Documents\Codex\2026-11-12\masz-dost-p-do-repo-c\work\astra-20261112` oraz w archiwum `astra-evidence.zip`.

**Kontrola wykonania**

Neutralny plik został ponownie oceniony na trzech meczach: wszystkie wartości SUMMARY są dokładnie równe uruchomieniom bez MODEL.
Weryfikacja objęła 42 zapisanych ocen z kontrolą rozdzielenia treningu i testu; żadna ewaluacja nie zmieniła pliku wag.
Manifest 36350 plików nagrań (nazwy, rozmiary, czasy modyfikacji) oraz SHA-256 API/cache wykryć/metadanych, źródeł trackera i ewaluatora, Data Dragon i priory są zgodne przed i po eksperymencie.
Nie modyfikowałem kodu produkcyjnego, testów, ustawień ani modelu aplikacji. Nie uruchamiałem gry/aplikacji.

Podczas pracy inny proces przebudował wspólną binarkę i wykonał commit `3fb0cbba05f3e9efbd9868ee58512b864bc6e691`, obejmując także część nowych plików eksperymentu. Ja nie wykonywałem commit/push i nie cofałem cudzych zmian.
Dlatego oryginalny pierwszy przebieg wykluczyłem z raportu, powtórzyłem baseline na osobnej kopii EXE i kontrolowałem jej hash. Wszystkie 12 powtórzonych SUMMARY okazały się identyczne liczbowo; raport używa wyłącznie powtórzonego przebiegu. Zmiany cudzych źródeł nie dotyczyły trackera ani ewaluatora.
