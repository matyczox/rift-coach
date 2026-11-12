"""Write the human report and complete per-fold CSV from saved measurements."""
import argparse
import csv
from pathlib import Path
from astra_experiment import REPO, BINS, GAMES, VARIANTS, read


def generate(out):
    results = read(REPO / "model/astra-results.json")
    audit = read(out / "data-audit.json")
    verify = read(out / "verification.json")
    provenance = read(out / "provenance.json")
    assert results["final_choice"] == "none", "Rewrite the interpretation for a different outcome"
    # Include n and the variant's own baseline: small abstention differences
    # mean it is misleading to print a single common short-gap baseline.
    with (REPO / "model/astra-loo-detail.csv").open("w", newline="", encoding="utf-8-sig") as f:
        fields = ["game", "variant", "lo_s", "hi_s", "n", "err_model_pct", "err_baseline_pct"]
        writer = csv.DictWriter(f, fieldnames=fields)
        writer.writeheader()
        for row in results["outer"]:
            for bucket in row["summary"]["buckets"]:
                if bucket["lo"] in BINS:
                    writer.writerow({"game": row["test"], "variant": row["variant"],
                        "lo_s": bucket["lo"], "hi_s": bucket["hi"], "n": bucket["n"],
                        "err_model_pct": bucket["errModel"] * 100,
                        "err_baseline_pct": bucket["errBase"] * 100})
    rows = results["aggregate"]
    table = ["| Wariant | 5–15 s | 15–30 s | 30–60 s | 60–120 s |",
             "|---|---:|---:|---:|---:|"]
    for variant in VARIANTS:
        cells = [f"{rows[variant][str(b)]['errModel']*100:.2f} / {rows[variant][str(b)]['errBase']*100:.2f}" for b in BINS]
        table.append("| " + variant + " | " + " | ".join(cells) + " |")
    counts = ", ".join(f"{rows['none'][str(b)]['n']}" for b in BINS)
    sample_table = ["| Mecz | Długość | Klatki blue/red | Rozbieżność >10% przy wspólnych wykryciach |",
                    "|---|---:|---:|---:|"]
    for i, data in enumerate(audit):
        seconds = data["last_game_time"]
        sample_table.append(f"| G{i+1} | {int(seconds)//60}:{int(seconds)%60:02d} | "
                            f"{data['frames']['frames']}/{data['frames']['red']} | "
                            f"{data['visible_cross_view_gt10pct_fraction']*100:.2f}% |")
    decisions = ", ".join(f"G{i+1}: `{results['nested_decisions'][g]['choice']}`" for i, g in enumerate(GAMES))
    a = rows['none']
    gain = lambda variant, bucket: (a[str(bucket)]['errModel']-rows[variant][str(bucket)]['errModel'])*100
    text = f"""# Wynik treningu trackera mgły — 12.11.2026

**Nie znalazłem wag spełniających wymagane kryterium przewagi nad trackerem bez wyuczonych mnożników.**
`astra-best.json` jest celowo modelem neutralnym (`multipliers: {{}}`), równoważnym `none`.
Nie przedstawiam go jako poprawionego modelu. Własny kandydat pozostaje w `astra-v1.json`.
Nic nie zostało wdrożone do aplikacji.

Błąd to średnia odległość euklidesowa w układzie minimapy ×100: procent **boku mapy**, nie procent błędnych wskazań.
Każda komórka poniżej oznacza **model / baseline „ostatnia widziana pozycja”**, w %; mniej znaczy lepiej.
Wynik łączny = `sum(n × err) / sum(n)` osobno dla każdego przedziału czasu we mgle.

{chr(10).join(table)}

Liczby próbek `none` w kolejnych kolumnach: **{counts}**. Wszystkie warianty mają identyczne liczebności i baseline w trzech długich przedziałach.
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
- `nested`: wewnątrz każdej zewnętrznej pary treningowej odbywa się jeszcze LOO (ucz na jednym, sprawdź na drugim, potem zamień). Kandydat musi obniżyć każdy długi błąd o ≥1 p.p. i nie zwiększyć 5–15 s. Dalej rozstrzyga średnia długich błędów z wagami 1:2:3. Gdy brak kandydata, wybór to `none`. Wybory: **{decisions}**.
- Zwykłe porównanie wielu kandydatów na tych samych 3 meczach pozostaje eksploracyjne. Wiersz `nested` ocenia procedurę wyboru bez użycia zewnętrznego testu do strojenia. Nie przedstawiam najlepszego pojedynczego wyniku jako potwierdzenia generalizacji. Końcowy neutralny model wynika z tej samej reguły zastosowanej do wszystkich trzech wyników cross-validation.

**Co pomogło, a co nie**

`astra` obniża 30–60 s o {gain('astra',30):.2f} p.p., ale pogarsza 15–30 s o {-gain('astra',15):.2f} i 60–120 s o {-gain('astra',60):.2f} p.p.
`bag-zero-a025` poprawia 15–30 s tylko o {gain('bag-zero-a025',15):.2f} p.p. (szum), 30–60 s o {gain('bag-zero-a025',30):.2f}, lecz pogarsza 60–120 s o {-gain('bag-zero-a025',60):.2f} p.p.
Uczenie sekwencyjne od zera daje poprawę 30–60 s o {gain('zero+train',30):.2f} p.p., ale pogorszenie 60–120 s o {-gain('zero+train',60):.2f} p.p.
Silniejsze uczenie/łączenie nie daje stabilnej poprawy; `astra+train` pogarsza 60–120 s aż o {-gain('astra+train',60):.2f} p.p.
Różnice mniejsze od około 1 p.p. traktuję jako szum; nie obliczam pozornie precyzyjnych przedziałów ufności z zależnych klatek.

**Ograniczenia danych**

G1 = `{GAMES[0]}`; G2 = `{GAMES[1]}`; G3 = `{GAMES[2]}`.

{chr(10).join(sample_table)}

Tylko G3 ma fazę `late` (≥25 min). Przy testowaniu G3 pozostałe mecze nie dostarczają treningu tej fazy.
„Prawda” to wykrycia detektora z widoku czerwonych, nie ręcznie oznaczone pozycje. Rozbieżności między widokami mogą obejmować błędy detekcji i synchronizacji; audyt nie pozwala jednoznacznie rozdzielić ich przyczyn.
Nie odrzucałem żadnego meczu, nie przeliczałem wykryć i nie zmieniałem danych po zobaczeniu wyników.
Klatki z jednego odcinka mgły są silnie zależne; trzy mecze nie wystarczają do potwierdzenia uniwersalnych wag dla ról, faz i różnych stylów gry.

**Odtworzenie (PowerShell, nowe RunDir przy każdym pełnym przebiegu)**

```powershell
Set-Location 'C:\\Users\\Admin\\Desktop\\ligalarp'
py -3 -m pip install psutil==7.2.2
$dataAppData = '{provenance['appdata']}'
$runDir = Join-Path $env:TEMP ('rift-coach-astra-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
powershell -ExecutionPolicy Bypass -File scripts\\dev\\astra-reproduce.ps1 -DataAppData $dataAppData -RunDir $runDir
py -3 scripts/dev/astra_report.py --out $runDir
```

Wrapper buduje narzędzia dokładną komendą:
```powershell
powershell -ExecutionPolicy Bypass -File rust\\app\\cargo-gnu.ps1 build --release -p rc-vision --example eval_tracker --example detect_cache
```
Następnie kopiuje ewaluator do `RunDir/bin`, uruchamia kopię `loo-tracker.sh` ze zmienioną wyłącznie ścieżką `E`, wykonuje eksperyment, audyt i weryfikację.
`APPDATA` jest ustawiane tylko dla procesów potomnych. Nie ma dostępu do modelu aplikacji. `TRUTH=red`; `TRAIN`, `MODEL`, `CAMPS`, `NO_ITEMS` są czyszczone i jawnie ustawiane dla każdego uruchomienia.
Trening zapisuje wyłącznie kopie wag w RunDir. Monitor co 0,25 s wstrzymuje własne procesy obliczeniowe, gdy działa `League of Legends.exe`, i wznawia po zakończeniu gry. Mechanizm suspend/resume sprawdzono na własnym procesie pomocniczym.

Do odtworzenia dokładnie na zachowanej binarce, bez ponownej kompilacji, dodaj do wywołania wrappera:
```powershell
-PinnedEvaluator '{out / 'bin/eval_tracker.exe'}'
```
Jej SHA-256: `{verify['exe_sha256']}`. Kopia jest również w archiwum dowodowym, w `run/bin/eval_tracker.exe`.
Surowe logi, niezależne modele treningowe, modele poszczególnych foldów, manifest wejść i 24 wewnętrzne oceny są w `{out}` oraz w archiwum `astra-evidence.zip`.

**Kontrola wykonania**

Neutralny plik został ponownie oceniony na trzech meczach: wszystkie wartości SUMMARY są dokładnie równe uruchomieniom bez MODEL.
Weryfikacja objęła {verify['lineage_checks_passed']} zapisanych ocen z kontrolą rozdzielenia treningu i testu; żadna ewaluacja nie zmieniła pliku wag.
Manifest {sum(len(v['files']) for v in provenance['recordings'].values())} plików nagrań (nazwy, rozmiary, czasy modyfikacji) oraz SHA-256 API/cache wykryć/metadanych, źródeł trackera i ewaluatora, Data Dragon i priory są zgodne przed i po eksperymencie.
Nie modyfikowałem kodu produkcyjnego, testów, ustawień ani modelu aplikacji. Nie uruchamiałem gry/aplikacji.

Podczas pracy inny proces przebudował wspólną binarkę i wykonał commit `3fb0cbba05f3e9efbd9868ee58512b864bc6e691`, obejmując także część nowych plików eksperymentu. Ja nie wykonywałem commit/push i nie cofałem cudzych zmian.
Dlatego oryginalny pierwszy przebieg wykluczyłem z raportu, powtórzyłem baseline na osobnej kopii EXE i kontrolowałem jej hash. Wszystkie 12 powtórzonych SUMMARY okazały się identyczne liczbowo; raport używa wyłącznie powtórzonego przebiegu. Zmiany cudzych źródeł nie dotyczyły trackera ani ewaluatora.
"""
    (REPO / "model/RAPORT-astra.md").write_text(text, encoding="utf-8")
    print("Written model/RAPORT-astra.md and model/astra-loo-detail.csv")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    generate(args.out.resolve())
