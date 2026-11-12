# Rift Coach (Rust) — lista do przetestowania

Skrót na pulpicie: **Rift Coach (Rust)** (obok starego „Rift Coach” = wersja Electron).

**Przed testem zamknij wersję Electron** (ikona w zasobniku / okno). Obie wersje mają wspólne
ustawienia, cache i folder analiz (`%APPDATA%\Rift Coach`), ale osobne blokady jednej instancji —
działając naraz pokazałyby dwa HUD-y.

## Start
- [ ] Okno otwiera się **bez zabierania fokusu** (gra/klient zostaje na wierzchu).
- [ ] Zakładki Główna / Minimapa / Nauka / Ustawienia działają, przyciski okna (min/max/zamknij) w pasku tytułu.
- [ ] Ustawienia: zmiana np. modelu zapisuje się (po restarcie zostaje).
- [ ] Nauka: liczniki (mgła, obserwowane mecze, profile, matchupy, briefy patcha, analizy, nagrania) — takie same jak w wersji Electron.

## Wybór postaci
- [ ] Po zablokowaniu postaci (~2,5 s po ostatnim picku wroga) status run: „ustawiam…” → „ustawione”.
- [ ] W kliencie zmieniła się **tylko** strona run „Rift Coach” (inne strony nietknięte).
- [ ] Brak wolnej strony → czytelny komunikat zamiast błędu.

## W grze (Summoner's Rift)
- [ ] HUD pojawia się tylko nad grą, gdy gra jest aktywnym oknem; znika po Alt+Tab.
- [ ] Klikanie w grze działa normalnie (HUD nie łapie myszy ani klawiatury).
- [ ] Minimapa wykryta sama na starcie gry (Minimapa → ramka pasuje); ręczna kalibracja zostaje, jeśli ją ustawisz.
- [ ] Karta ładowania (pierwsze ~75 s): profile wrogów — wymaga klucza Riot API w Ustawieniach (status „profile: gotowe N”).
- [ ] Pasek siły z przeciwnikiem z linii (po pobraniu matchupu; pierwszy raz w danym matchupie może potrwać 1–3 min).
- [ ] Panel buildu: 2–3 przedmioty, odświeża się po zakupie / śmierci / co 3 min.
- [ ] Porada coacha na górze (co ~25 s), karta śmierci z analizą.
- [ ] Duchy wrogów we mgle na minimapie, alerty (invade / gank / brak na linii / fed), timery smoka/barona.
- [ ] Wskaźnik zagrożenia (bezpiecznie / uważaj / uciekaj) i werdykt na obiekt (go / contest / skip).
- [ ] Skróty: Alt+Shift+O (HUD wł./wył.), Alt+Shift+F (Flash wroga z linii → timer), Alt+Shift+R (nagrywanie).
- [ ] ARAM / inne mapy: brak duchów i nauki mgły (to normalne), paski HP i porady działają.

## Po meczu
- [ ] Po ekranie końca gry (≥ 5 min meczu) w ciągu ~1–3 min pojawia się analiza meczu na liście meczów (Główna).
- [ ] Analiza ma wynik (wygrana/przegrana), KDA, CS/min, co dobrze / co poprawić, momenty, wykres złota.
- [ ] Nauka: „obserwowane mecze” +1 (tylko gry rankingowe/normalne na SR, ≥ 2 min); po ~1–3 min licznik próbek mgły rośnie (wymaga klucza Riot API).
- [ ] Z włączonym „Nagrywaj automatycznie”: nowe nagranie w Nauce → Nagrania, zatrzymane ~15 s po meczu; najwyżej 5 najnowszych / 14 dni.

## Powtórki (opcjonalnie)
- [ ] Nauka → Powtórki: lista plików .rofl; „Zbierz” uruchamia powtórkę przez klienta, przełącza mgłę i zbiera klatki
  (gdy przełączenie się nie uda — prośba o ręczną zmianę „Mgła wojny” i „Kontynuuj”).
- [ ] Odtwarzanie własnego nagrania (Nagrania → otwórz): minimapa się odtwarza, play/pauza/przewijanie/prędkość.

## Co zgłosić
Zrzut ekranu + godzina, co się stało, czy to wersja Rust. Jeśli coś się zawiesi: menedżer zadań →
pamięć/CPU procesu `rift-coach.exe` i jego procesów `msedgewebview2.exe`.
