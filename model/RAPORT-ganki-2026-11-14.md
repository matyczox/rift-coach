# Audyt treningu i ostrzegania o gankach — 2026-11-14

## Co rzeczywiście poprawiono

Naprawiono jakość etykiet i aktualizację wag. Dodano wcześniejsze żółte ostrzeżenie o utrzymanym podejściu junglera lub roamującego przeciwnika. Nie włączono nowo wyuczonych wag ani eksperymentalnej zmiany siły mgły: nie przeszły kontroli jakości. Aktywny model z 414 obserwacjami zachowano.

**Nie ma dowodu na ogólną poprawę dokładności nowych wag.** Poprawa wczesnych ostrzeżeń w poniższej symulacji wiąże się z nową regułą podejścia. Większa czułość przynosi też dodatkowe fałszywe ostrzeżenia. To nie jest raport 80–90% skuteczności.

## Dlaczego poprzednie uczenie było zawodne

1. Surowe wykrycia czerwonych traktowano jako prawdę, mimo niskiej pewności. Potwierdzony przykład: w 87.780 s pierwszego meczu Kai’Sę przypisano do wieży na topie (`score=0.483`), gdy była na bocie. Takie etykiety tworzyły pozorne przemieszczenia przez całą mapę. Z dwóch ostatnich meczów nowy filtr zachował 78.2% i 74.6% wykryć czerwonych. Odrzucone punkty są niepewne, nie wszystkie muszą być błędne.
2. Stanie przez 7 s i zniknięcie było uznawane za zakończony recall. W drugim meczu znaleziono cztery takie zdarzenia; trzy pierwsze dostępne etykiety były daleko od bazy, w tym pewne wykrycia Caitlyn i Renektona. Usunięto teleport całego przewidywania do bazy z tej przesłanki. Zwykła hipoteza recallu pozostaje w filtrze.
3. Uczenie porównywało wyjaśnienie etykiety z początkowym rozkładem celów, choć mgła i resampling zdążyły zmienić rozkład. Teraz porównuje je z aktualną mieszaniną cząsteczek przed etykietą. Etykieta, która nie wnosi nowej informacji, nie wzmacnia losowo preferencji celu.
4. Wiele etykiet z tego samego odcinka nie stanowi niezależnych przykładów ganków. Mnożniki roli/fazy są zbyt ogólne, by z kilku ganków nauczyć się wszystkich decyzji przeciwnika. Ograniczono krok aktualizacji, dodano kurczenie do neutralnych wag, oddzielny limit próbkowania dla każdej postaci i równy udział meczów w końcowym zespole modeli.

## Jak działa poprawione uczenie

- Etykieta: dopasowanie co najmniej 0.75, fizycznie spójny sąsiad czasowy do 2 s; odrzucenie sprzeczności z pewnym widocznym przeciwnikiem. Weryfikacja może korzystać z przyszłej klatki tylko przy tworzeniu etykiet. Te dane nie trafiają do wejścia trackera.
- Prawdziwe pozycje do uczenia co najmniej co 10 s na postać, głównie w odcinkach 3–30 s; jawnie widoczny teren z brakującą ikoną nie jest traktowany jako trasa we mgle.
- W grze i w zapisie do późniejszego uczenia słabe tożsamości poniżej 0.75 nie uczą wag. Nieprawdopodobny skok nie uczy normalnych tras marszu.
- Każdy model startuje od neutralnych mnożników na osobnym meczu. Średnia geometryczna daje meczom równe znaczenie, po czym stosowane jest osłabienie zmiany w logarytmie o połowę. Każdy mecz walidacyjny jest wyłączony z treningu swojego modelu.
- Narzędzie odmawia treningu na surowym `TRUTH=red`; wymagane są przygotowane `TRUTH=truth` i raport jakości.

Końcowy kandydat ma 1417 etykiet z trzech meczów. Nadal nie pokonał stabilnie neutralnych wag w walidacji między meczami. Zachowano go jako artefakt badania, bez instalowania go do aktywnego modelu.

## Ostrzeganie i pomiar procentów

Nowe ostrzeżenie rozpoznaje także widocznego przeciwnika idącego w stronę gracza. Wymaga utrzymania sygnału przez 1.5 s, świeżej obserwacji i przecięcia przewidywanej trasy z obszarem gracza w ciągu 1–6 s. Sam dodatni kierunek ruchu nie wystarcza: trasa może przebiegać obok. Uwzględniono większą utrzymaną szybkość postaci z przyspieszeniem. Przeciwnicy z własnej linii nie są nazywani gankerami; jungler nie jest traktowany jako własny przeciwnik liniowy. To żółta informacja o ryzyku, bez dodawania przyszłej pozycji do obecnego paska walki i bez udawanego procentu pewności na tym komunikacie.

**Pomiar jest automatycznym wskaźnikiem podejść, nie ręcznym oznaczeniem intencji ganku.** Zdarzenie: przeciwnik spoza własnej linii przemieszcza się z odległości ≥0.22 do ≤0.13 szerokości mapy, kontakt ma potwierdzenie w kolejnej klatce, a przed nim był okres ukrycia. Nie liczymy samego podejścia gracza do nieruchomego wroga. Ostrzeżenie uznajemy za wczesne, gdy wystąpiło 2–8 s przed wejściem w ten obszar. Brak wiarygodnych etykiet ogranicza liczbę ocenianych zdarzeń.

| Pomiar na czterech powtórkach | Przed | Po |
|---|---:|---:|
| Wczesne ostrzeżenie na oznaczonych podejściach | 2/22 = 9.1% | 4/22 = 18.2% |
| Potwierdzone kontaktem epizody ostrzegania | 6 | 9 |
| Epizody uznane za fałszywe przy dostępnych etykietach | 3 | 7 |
| Epizody bez danych wystarczających do oceny | 11 | 15 |

Mediana wyprzedzenia trafionych wczesnych ostrzeżeń po zmianie: 5.8 s. Wśród ocenialnych epizodów ostrzegania kontakt potwierdził się w 6/9 przed i 9/16 po zmianie. Pominiętych epizodów nie wolno uznać ani za trafione, ani za fałszywe.

| Mecz | Oznaczone podejścia | Wczesne przed | Wczesne po | Fałszywe przed/po |
|---|---:|---:|---:|---:|
| 4028222906 | 9 | 1 | 3 | 2/5 |
| 4028237949 | 2 | 0 | 0 | 0/0 |
| 4027181891 | 10 | 1 | 1 | 1/2 |
| 4027208890 | 1 | 0 | 0 | 0/0 |

Próbka jest mała. Regułę utrzymania kierunku dobierano na pierwszym meczu; inne elementy audytowano iteracyjnie na powtórkach. Wynik zbiorczy jest retrospektywny, nie stanowi niezależnej oceny następnej gry. Ostatni mecz zawierał tylko jedno kwalifikujące się podejście i nie wykazał poprawy wykrywania — nie daje podstaw do deklarowania dobrej generalizacji.

W jednym z pominiętych podejść Warwick nie był widziany od około 80 s. Same wagi ogólnej trasy nie dostarczają wtedy wiedzy o jego rzeczywistej pozycji. Nowa reguła kierunku pomaga przy świeżych obserwacjach, nie rozwiązuje wszystkich braków informacji.

## Dokładność pozycji

Niżej błąd odległości jako procent szerokości mapy (mniej jest lepiej). 17.7% nie oznacza skuteczności ganków. Liczby są ważone liczbą ocenianych klatek; po zmianie aktywne wagi są takie same. Zmiany około 0.3–0.6 punktu procentowego nie stanowią dowodu poprawy dokładności pozycji.

| Czas we mgle | Przed | Po | Oceny przed/po |
|---|---:|---:|---:|
| 0–5 s | 6.43% | 6.54% | 3255/3254 |
| 5–15 s | 17.44% | 17.73% | 4156/4156 |
| 15–30 s | 27.95% | 28.30% | 3083/3083 |
| 30–60 s | 32.42% | 32.73% | 2446/2446 |
| 60–120 s | 24.13% | 24.69% | 1002/1002 |

## Odrzucone warianty

Sprawdzono normalizację wpływu mgły według czasu oraz efektywne częstotliwości 1/3/10 Hz. Wybór na trzech meczach nie przeszedł końcowej kontroli regresji czwartego meczu w połączeniu z neutralnymi wagami. Nie przypisujemy całej różnicy wyłącznie temu parametrowi. Produkcja zachowuje dotychczasową siłę obserwacji; eksperymentalny parametr pozostaje wyłącznie do dalszej oceny offline. Nie zresetowano dotychczasowych aktywnych wag.

Kolektor powtórek wraca teraz na początek i przywraca HUD przed zmianą drużyny mgły. Animacja końca gry potrafiła blokować listę wyboru i zatrzymywać drugi przebieg.

## Odtworzenie i zakres kontroli

```powershell
powershell -ExecutionPolicy Bypass -File rust/app/cargo-gnu.ps1 build --release -p rc-vision --example eval_tracker --example detect_cache
py scripts/dev/prepare-replay-labels.py SESSION1 SESSION2 SESSION3
py scripts/dev/train-replay-tracker.py --out WORK_DIR --trace SESSION1 SESSION2 SESSION3
$env:TRUTH='truth'
$env:MODEL='FROZEN_MODEL.json'
$env:MY_CHAMPION='Leona'
$env:TRACE='TRACE.jsonl'
# TRAIN oraz ABSENCE_HZ nie są ustawione dla konfiguracji wdrożonej.
rust/target/release/examples/eval_tracker.exe SESSION1
py scripts/dev/eval-gank-approaches.py TRACE.jsonl --role UTILITY
py scripts/dev/test-replay-training.py
```

Surowe sesje pozostają lokalnie w katalogach roboczych czatu. Zbiorcze wyniki zawierają osobno dokładne definicje, parametry i wyniki ganków oraz walidację wag. Ewaluator używa wykryć minimapy i 300 cząsteczek; aplikacja na żywo ma 400 cząsteczek oraz dodatkową filtrację pozycji i zasłoniętych portretów. To test silnika i heurystyk na zapisach, nie odtworzenie całego HUD klatka po klatce. Nie publikujemy surowego API z nazwami graczy.

Wyniki wcześniejszego raportu `RAPORT-replay-2026-11-14.md` opierały się na niewystarczająco filtrowanych etykietach i nie powinny służyć do oceny aktualnej dokładności.

## Kontrola wdrożenia

Przeszły 43 właściwe testy Rust (21 jednostkowych rc-vision, 19 integracyjnych trackera, 2 kolektora, 1 parsera czasu) i 7 testów narzędzi Python. Osobno sprawdzono odmowę uczenia z surowych etykiet: proces odrzucił TRAIN=1/TRUTH=red i nie zapisał modelu. Kompilacja release i test startu aplikacji (8 s, kod wyjścia 0) przeszły. Aplikacja uruchomiona ponownie z aktywnymi 414 obserwacjami, zachowano hash wag i przywrócono pierwotną kalibrację.

Filtr zwykłego ADC + support na bocie był dodany wcześniej. Nowy test narzędzia pomiarowego potwierdza, że Caitlyn, Kai’Sa i support na własnej linii nie tworzą etykiety ganku ani alarmu w tej ocenie. Nie zaliczamy wcześniejszego filtra do nowej poprawy.
