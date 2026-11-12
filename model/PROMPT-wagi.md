Jesteś ekspertem League of Legends (Summoner's Rift, sezon 2026) i znasz się na modelach probabilistycznych.

## Zadanie
Napisz plik JSON z wagami dla trackera mgły wojny. Tracker przewiduje, gdzie jest przeciwnik, który zniknął we mgle (filtr cząsteczkowy: setki hipotez "idzie do celu X", poruszających się z prędkością postaci po przechodnim terenie). Twoje wagi to MNOŻNIKI prawdopodobieństwa, że dana rola w danej fazie gry idzie do danego rodzaju celu. Wypisz je z wiedzy o grze (typowe ścieżki junglera, roamy midów, rotacje supportów, grupowanie w późnej grze itp.).

## Jak tracker liczy (żebyś wiedział, co mnożysz)
waga celu = tabela_bazowa[rola][cel] × Twój_mnożnik[rola|faza][cel]
Do tego tracker sam dokłada: ×4 dla Dragona/Barona, gdy pojawia się w <50 s; ×5 dla bazy, gdy wróg prawdopodobnie robi back; w mid/late wagi linii są przycinane do max(w×0.6, 1.5); zaraz po wyjściu z fontanny baza i "stay" ≈ 0.

Tabela bazowa (wczesna gra, już w kodzie — NIE powtarzaj jej, tylko mnóż):
- JUNGLE: camp 5, lane-top/mid/bot 1.2, dragon 1.5, baron 1.0, base 0.5, invade 0.8, river 1.5
- TOP: camp 0.3, lane-top 8, lane-mid 0.4, lane-bot 0.1, dragon 0.2, baron 0.6, invade 0.2, river 0.6
- MIDDLE: camp 0.5, lane-top 0.8, lane-mid 8, lane-bot 0.8, dragon 0.8, baron 0.6, invade 0.3, river 1.2
- BOTTOM: camp 0.2, lane-top 0.1, lane-mid 0.4, lane-bot 8, dragon 1.0, baron 0.2, invade 0.1, river 0.4
- UTILITY: camp 0.3, lane-top 0.2, lane-mid 0.6, lane-bot 6, dragon 1.5, baron 0.4, invade 0.6, river 2.0
- "stay" (zostaje w miejscu) ma stałą wagę 2.0 dla każdego.

## Format (dokładnie taki)
- Klucze: "ROLA|faza", ROLA ∈ JUNGLE, TOP, MIDDLE, BOTTOM, UTILITY; faza ∈ early (<14 min), mid (14–25 min), late (>25 min). Razem 15 kluczy.
- Rodzaje celów: camp, lane-top, lane-mid, lane-bot, dragon, baron, base, invade, river, stay.
- Wartości: liczby od 0.2 do 5.0 (1.0 = bez zmian). Rodzaj pominięty = 1.0.
- "invade" = dżungla przeciwnika, "camp" = własna dżungla, "river" = rzeka i krzaki przy niej.
- Mapa jest symetryczna; cele są liczone względem strony gracza, więc nie rozróżniaj stron.

Zwróć WYŁĄCZNIE poprawny JSON, bez komentarzy i bez tekstu wokół:
{
  "version": 1,
  "multipliers": {
    "JUNGLE|early": { "camp": 1.0, "river": 1.2 },
    "...": {}
  },
  "score": { "n": 0, "logModel": 0, "logBaseline": 0, "hits": 0 },
  "recent": []
}

## Jak to będzie ocenione
Na 3 prawdziwych meczach (powtórki) porównamy błąd przewidzianej pozycji z prawdziwą pozycją przeciwnika we mgle, osobno dla 5–15 s, 15–30 s, 30–60 s i 60–120 s od zniknięcia, względem trackera bez wag. Uwaga: zbyt skrajne wagi (np. 5.0 albo 0.2 wszędzie) zwykle pogarszają długie odcinki — model zaczyna "wierzyć" w jeden cel. Umiarkowane, dobrze uzasadnione zmiany (0.6–1.8) zwykle wypadają lepiej.
