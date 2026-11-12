# Rift Coach v2 — podstawy

Nowy projekt od zera. Ze starej wersji nie bierzemy kodu, UI ani założeń.

## 1. Co budujemy: dwa systemy

| | **System 1: Tracker** (lokalny, algorytmiczny) | **System 2: Coach** (Claude z twojej subskrypcji) |
|---|---|---|
| Co robi | Gdzie są wrogowie we mgle, kółko „gdzie mogą być”, ostrzeżenia invade/gank | Decyzje makro: smok tak/nie, farm czy roam, back czy zostać, grać pasywnie czy agresywnie |
| Jak szybko | co klatkę (~10/s), natychmiast | co kilkanaście sekund i przy zdarzeniach, 3–10 s na odpowiedź |
| Jak | wizja minimapy + filtr cząsteczkowy + model uczony na twoich meczach | dostaje cały stan meczu, profile przeciwników i wynik Systemu 1 |
| Gdzie pokazuje | minimapa gry | krótki komunikat na górze ekranu, dłuższa analiza po twojej śmierci |

System 1 karmi System 2. Coach nie zgaduje pozycji, tylko dostaje gotowe „Warwick 70% bot-side dżungla, zniknął 14 s temu”.

**Granice:** zero czytania pamięci gry, wstrzykiwania kodu, klikania za gracza i automatycznych zakupów. Dane biorą się z obrazu okna gry, lokalnych API gry i klienta oraz oficjalnego API Riotu.

> Ryzyko: zasady Riota dla aplikacji zewnętrznych zabraniają pokazywania w grze informacji, których gracz nie ma. Przewidywanie pozycji we mgle może pod to podpadać. Projekt prywatny, decyzja należy do ciebie.

## 2. Zasady

- **Najpierw pomiar, potem „działa”.** Wizja i predykcja mają liczbę skuteczności na nagranych prawdziwych meczach. Testy syntetyczne nie są dowodem.
- **Brak detekcji ≠ zniknięcie z wizji.** O mgle decyduje maska wizji z minimapy plus brak portretu przez kilka klatek. O śmierci decyduje API.
- **Nagrywanie i odtwarzanie.** Każdy mecz można nagrać (klatki minimapy + wszystkie dane API) i odtworzyć offline z nakładką.
- **Nie czytasz, tylko zerkasz.** W trakcie gry maks. ~8 słów naraz. Dłuższy tekst tylko, gdy jesteś martwy.
- **Czytelny kod.** TypeScript, małe moduły, testy obok kodu.

## 3. Źródła danych („max danych”)

| Źródło | Kiedy | Co daje |
|---|---|---|
| **Live Client API** (127.0.0.1:2999, w grze) | co 500 ms | wszyscy gracze: itemy, poziomy, KDA, CS, czary przywoływacza, runy, martwy + czas respawnu; twoje pełne staty i złoto; zdarzenia: kille, asysty, smoki z typem, herald, baron, wieże, inhibitory, first blood, multikille |
| **LCU API** (klient LoL, lokalnie) | lobby i wybór postaci | skład drużyn, nazwy graczy, pozycje (role), wybrane runy i czary, twoja historia |
| **Riot API** (osobisty klucz deweloperski) | w trakcie wyboru postaci i ekranu ładowania | historia meczów przeciwników + **timeline’y** (pozycje co minutę, kille, wardy, zakupy) |
| **Obraz minimapy** | ~10 klatek/s | pozycje widocznych, mgła (maska wizji), kamera, pingi |
| **Data Dragon** | przy aktualizacji patcha | itemy, staty postaci, ikony, cooldowny |

Z tego liczymy rzeczy, których nie widać od razu:
- **Złoto przeciwników** (wartość itemów + szacunek z CS/killi): kto ma spike, kto zaraz wraca do bazy.
- **Power spike’i:** poziom 6/11/16, ukończony mityczny/drugi item, czy wróg może teraz cię zabić (prosty kalkulator obrażeń).
- **Cooldowny czarów przywoływacza** po zdarzeniach, które wiadomo z API (np. śmierć, czyli reset), oraz ręczny skrót „Flash wroga użyty”.
- **Timery** smoka, heralda, barona, inhibitorów z wydarzeń.

## 4. Profile przeciwników (jak Blitz, tylko głębiej)

Liczone w trakcie wyboru postaci z ostatnich ~20 gier każdego przeciwnika (Riot API, z cache):

- **Agresja:** kille + śmierci przed 15 min, solo kille, first blood %, jak często umiera przed 10 min.
- **Jungler:** z timeline’ów: **pierwsza ścieżka** (od której strony zaczyna), minuta i linia pierwszego ganka, jak często invaduje, jak często idzie na smoka/heralda. To idzie wprost do Systemu 1 jako priors: „ten Warwick w 70% gier zaczyna od dołu i gankuje bot przed 3:30”.
- **Laner:** czy roamuje (zniknięcia z linii na timeline’ach), czy farmi, czy jest tiltowalny (seria przegranych, KDA w ostatnich grach).
- **Postać:** liczba gier i winrate na tej postaci w sezonie, maestria. Mało gier = pierwsza próba = słabszy.
- **Forma:** seria wygranych/przegranych, LP.

Na ekranie ładowania: jedna linia na wroga, np. „WW: agresywny, start dół, gank bot ~3:20”.

## 5. System 1: Tracker

### 5.1 Capture i kalibracja
- Tylko okno gry. Wycinek minimapy ~10 klatek/s, cały ekran 1 klatka/s (kamera).
- Automatyczna kalibracja ramki minimapy + ręczna korekta. Skala minimapy, rozdzielczość, flip dla czerwonej strony.

### 5.2 Detektor portretów
- Szukamy tylko 10 znanych postaci.
- **Klasycznie (start):** okrągły szablon (NCC z maską koła) w skali ikony z kalibracji, kolor obwódki = drużyna, każda postać najwyżej raz, dokładniejsze szukanie wokół ostatniej pozycji.
- **Sieć (jeśli trzeba):** mały detektor ONNX trenowany na klatkach oznaczonych automatycznie i poprawionych ręcznie.
- **Cel:** ≥95% wykrytych widocznych portretów, ≤1% zamian tożsamości, ≤150 ms opóźnienia.

### 5.3 Maska wizji
Mgła na minimapie jest ciemniejsza. Odczyt maski pól z wizją na siatce 64×64. Wiadomo, gdzie wroga **na pewno nie ma**. To zawęża przewidywania i usuwa duplikaty.

### 5.4 Stan wroga
`visible` → `fog` → `dead`, z jawnymi warunkami:
- `visible`: portret w ≥2 z 3 ostatnich klatek.
- `fog`: brak portretu ≥3 klatki i ostatnie miejsce poza maską wizji.
- `dead`: z API. Po respawnie pozycja = fontanna.

### 5.5 Predykcja
- Rozkład prawdopodobieństwa dla każdego wroga we mgle (filtr cząsteczkowy, ~500 cząstek na siatce 64×64).
- Ruch tylko po przechodnim terenie, z prędkością postaci, kierunki z wyuczonych przejść.
- Cząstki w polach, które widzisz, a wroga tam nie ma, odpadają.
- Priors (na czym AI opiera przewidywanie):
  - **Campy:** stan ikon obozów na minimapie (zniknęła ikona = ktoś wyczyścił; twój camp zniknął, a ty go nie brałeś = invade), czasy respawnu campów, typowa kolejność czyszczenia.
  - **Wcześniejsze zachowanie w tym meczu:** gdzie ten wróg już bywał, którędy chodził, po ilu sekundach wracał na linię, kiedy robił back.
  - **Profil gracza z Riot API:** pierwsza ścieżka, minuta i linia ganków, częstość invade.
  - Rola, czas gry, timery smoka/heralda/barona, stan fali, czas od zniknięcia, prędkość ruchu.
- Wyświetlanie: **półprzezroczyste kółko** (środek = najbardziej prawdopodobne miejsce, promień = obszar, w którym jest ~70% prawdopodobieństwa) + blady portret w środku.
- **Uczenie:** przy każdym ponownym pojawieniu się zapis, ile prawdopodobieństwa model dawał temu miejscu. Wagi przejść aktualizują się między meczami. Statystyka: model vs. „został tam, gdzie go widziano”.

### 5.6 Ostrzeżenia (lokalne, natychmiastowe)
- **Invade:** prawdopodobieństwo, że wrogi jungler lub ≥2 wrogów jest w twojej ćwiartce dżungli albo w promieniu od ciebie, > próg.
- **Gank:** masa prawdopodobieństwa przy twojej linii rośnie, a ty stoisz wysoko.
- **Brak wizji:** N wrogów we mgle dłużej niż T, a ty idziesz w ich stronę.
- **Spike wroga:** twój przeciwnik z linii właśnie skończył item lub ma 6, a ty nie.

## 6. System 2: Coach (Claude z subskrypcji)

### 6.1 Jak to technicznie działa
- Aplikacja uruchamia **Claude Code w trybie bez okna** (`claude -p`, sesja zalogowana twoją subskrypcją) i przekazuje stan meczu. Odpowiedź wraca jako JSON. Bez klucza API i bez dodatkowych opłat, w ramach limitów subskrypcji.
- Jedna sesja na mecz (`--resume`), więc coach pamięta, co już doradził i jak toczy się gra.
- Wywołania: co ~20–30 s oraz natychmiast przy zdarzeniu (kill/śmierć, objective za ≤60 s, zniknięcie junglera, możliwy zakup, twoja śmierć). Limit, żeby nie zjeść subskrypcji.
- Porada odrzucana, jeśli stan zmienił się istotnie od wysłania.
- Szybki model w grze, mocniejszy do analizy po śmierci i po meczu.

> Obecnie Claude Code na tym komputerze **nie jest zalogowany**. Przed etapem z coachem trzeba raz uruchomić `claude` i zalogować się kontem z subskrypcją.

### 6.2 Co dostaje
- Twój bohater, staty, złoto, itemy, poziom, pozycja, czy jesteś na linii.
- Wszyscy gracze: itemy, szacowane złoto, poziomy, KDA, CS, martwi + respawn.
- Timery objective, stan wież.
- Z Systemu 1: pozycje i prawdopodobieństwa wrogów, ostrzeżenia.
- Profile przeciwników (agresja, ścieżka junglera, forma).

### 6.3 Co zwraca
```json
{
  "call": "Smok: TAK — 3 wrogów bot-side martwych lub daleko",
  "tone": "go | wait | danger | farm",
  "mapHint": { "type": "objective", "target": "dragon" },
  "item": "Zhonya's",
  "deathReview": null,
  "ttlSec": 20
}
```
- `call`: **maks. ~8 słów**, wyświetlane na górze ekranu.
- `deathReview`: tylko gdy jesteś martwy. 3–5 krótkich punktów: dlaczego zginąłeś, co po respawnie (back czy farm, gdzie iść), styl na najbliższe minuty (pasywnie/agresywnie i dlaczego).

## 7. UI / UX

**Zasada:** jak najmniej na ekranie, zerknięcie wystarczy, nic nie zasłania gry, wszystko przepuszcza kliknięcia.

### 7.1 Pasek na górze (System 2)
- Wąski pasek u góry pośrodku, tło ~55% ciemne.
- Jeden komunikat, maks. ~8 słów. Kolor lewej krawędzi = `tone`: zielony = rób, żółty = czekaj, czerwony = niebezpiecznie, szary = farm.
- Nowy komunikat pojawia się płynnie (150 ms). Znika po `ttlSec`.

### 7.2 Minimapa (System 1)
- Wróg we mgle: **półprzezroczyste czerwone kółko** (~15–20% wypełnienia, cienka obwódka) = obszar, gdzie może być, z bladym portretem (~55%) w środku. Kółko przesuwa się tam, gdzie AI przewiduje ruch (np. w stronę campa, który mu się respawnuje), i rośnie, im dłużej wróg jest niewidoczny. Po 45 s sam blady obszar bez portretu.
- Widoczny wróg: nic nie rysujemy.
- Ostrzeżenie o invade: strefa w twojej dżungli pulsuje na czerwono 2 s, potem jest statyczna.
- Strzałka/cel coacha (np. smok): zielona, wygasa po 15 s.
- Timery smoka/barona przy ich miejscach, tylko ≤60 s.

### 7.3 Ekran po śmierci
- Gdy jesteś martwy, na środku-prawej pojawia się karta z `deathReview`: 3–5 punktów, czytelny tekst. Znika przy respawnie.

### 7.4 Ekran ładowania
- Karta z profilami 5 wrogów, jedna linia na każdego.

### 7.5 Okno ustawień (poza grą)
- Status: gra / API / klient / minimapa / Claude zalogowany / klucz Riot.
- Kalibracja minimapy z podglądem.
- Częstotliwość porad, model.
- Nagrania meczów z odtwarzaniem i nakładką.
- Statystyki: detektor, predykcja vs. baseline, trafność porad.
- Skróty: pokaż/ukryj, „Flash wroga użyty”, nagrywanie.

## 8. Architektura

```
Rust / Tauri (rust/app + crates rc-core, rc-vision); UI w TypeScript/React (src/renderer)
├─ sources/   live-api · lcu · riot-api (cache) · ddragon · capture
├─ recorder/  zapis i odtwarzanie sesji
├─ tracker/   (worker) kalibracja · detektor · maska wizji · stan · predykcja · ostrzeżenia
├─ intel/     złoto wrogów · spike’i · timery · profile przeciwników
├─ coach/     snapshot → claude -p → walidacja → porada / deathReview
└─ windows/   top-bar · minimap-layer · death-card · loading-card · control (React)
```

## 9. Etapy

| Etap | Co | Gotowe, gdy |
|---|---|---|
| **M0** | Szkielet, capture, Live API + LCU, kalibracja, **nagrywanie i odtwarzanie**, okno ustawień | nagrany mecz odtwarza się offline |
| **M1** | Detektor + narzędzie do etykiet + pomiar | ≥95% wykrytych na nagraniach |
| **M2** | Maska wizji + stan visible/fog/dead | zero duplikatów na nagraniach |
| **M3** | Predykcja + warstwa minimapy + uczenie + ostrzeżenia | model lepszy od baseline |
| **M4** | Coach przez `claude -p`: pasek na górze + ekran po śmierci | porady w żywym meczu |
| **M5** | Riot API: profile przeciwników, priors junglera, ekran ładowania | profile na ekranie ładowania |
| **M6** | Intel: złoto, spike’i, kalkulator obrażeń, zakupy | — |
| **M7** | Detektor sieciowy, jeśli M1 nie osiągnie celu | — |

## 10. Czego potrzebuję od ciebie

1. **2–3 nagrane mecze** po etapie M0. Nagrywanie tylko zapisuje minimapę i dane, nic nie klika.
2. **Zalogowanie Claude Code** (`claude` → login subskrypcją) przed M4.
3. **Osobisty klucz Riot API** z developer.riotgames.com (darmowy, ważny 24 h, do odnowienia) przed M5.
4. Rozdzielczość gry i skala minimapy, jeśli inne niż 1600×900.
