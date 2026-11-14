# Import czterech powtórek i poprawki HUD

Poprzedni `RAPORT-ganki-2026-11-14.md` opisuje decyzję o zachowaniu modelu z 414 etykietami. Po tym audycie użytkownik wyraźnie wybrał import danych i włączenie nowych wag. Przygotowano model `fog-replay-4games.json`: cztery mecze, 1770 zaakceptowanych aktualizacji, równy udział meczów i kurczenie mnożników do wartości neutralnych.

## Dane

Pełne przebiegi meczów 4028222906, 4028237949, 4027181891 i 4027208890 zastępują dwa wcześniejsze krótkie zapisy. Importowane obserwacje mają odpowiednio 1668, 800, 1461 i 898 klatek. Pochodzą wyłącznie z widoku niebieskiego; czerwone etykiety pozostają w osobnym katalogu. Cztery pliki przeszły rzeczywisty dekoder `rc-core::self_train::load_obs`. Import zachowuje kopie zastępowanych danych i deduplikuje po identyfikatorze meczu. Bieżąca rozgrywka może później dodać piąty zapis.

## Walidacja

Walidacja czterech meczów wyłącza oceniany mecz z treningu. Średni błąd pozycji to odległość na mapie, **nie procent niewykrytych ganków**. Mniejsza wartość jest lepsza.

| Czas we mgle | Neutralny | Trenowany na pozostałych meczach |
|---|---:|---:|
| 0–5 s | 6,56% | 6,51% |
| 5–15 s | 17,67% | 17,78% |
| 15–30 s | 28,26% | 28,50% |
| 30–60 s | 31,92% | 32,20% |
| 60–120 s | 22,08% | 24,62% |

Większy zbiór nie gwarantuje poprawy. Nowe wagi są wyborem użytkownika, a wyniki nie uzasadniają twierdzenia o lepszej skuteczności. Szczegóły: `fog-replay-4games-validation.json`; sumy kontrolne importu: `replay-4games-manifest.json`.

## Sprawdzenie bieżącej gry 4028311966

- API zapisało w 504,6 s: Leona 6, Caitlyn 5, Jhin 4, Mel 7. Przewaga Leony nad Jhinem +2 nie jest przewagą pary: sumy wynoszą 11 do 11. HUD dostaje teraz osobno obie pary i ich sumę; istniejący lokalny model walki uwzględnia poziomy pobliskich partnerów.
- W 996 s Sett był w pozycji (0,792; 0,572), Leona w (0,897; 0,689). Wkrótce Sett znalazł się przy Leonie, lecz log nie zawierał powiadomienia o podejściu. Prędkości używane przez TeamWatcher były przesunięciami na klatkę; próg ruchu blokował zwykły marsz przy częstych odczytach. Naprawiono jednostki na sekundy, wygładzanie względem czasu i odrzucanie nagłych skoków. Osobny limit powiadomień dla każdej postaci zapobiega wyciszeniu Setta przez wcześniejsze podejście innego sojusznika. Trasa ma prowadzić w obszar walki, nie dokładnie w środek gracza.
- Zmieniono katalog przedmiotów na pl_PL; angielska odpowiedź modelu również mapuje się do polskiej nazwy, ikony i ceny. Panel ponownie pokazuje do trzech pozycji planu oraz dostępne zakupy.
- Klucz Riot zapisuje się po wklejeniu z opóźnieniem 600 ms, również po opuszczeniu ustawień. Zmiana klucza/serwera ponawia profile; spóźniona odpowiedź dla starego klucza nie zastępuje nowej. Brak zapisanej wartości nie oznacza aktywnego połączenia z Riot.
- Baron zginął w 1650,4 s; w 1653 s dotarła odpowiedź „Przygotuj Baron z Caitlyn”, oparta na wcześniejszym stanie. Dodano deterministyczną kontrolę dostępności celu przy odbiorze odpowiedzi i przy każdym wyświetleniu HUD. Niedostępny cel z respawnem późniejszym niż za 90 s nie dostaje polecenia ani strzałki. Sprawdzenie działa także, gdy cel zginie już podczas wyświetlania porady; nie usuwa ostrzeżenia „Nie idź na Barona”.

Testy obejmują polską nazwę dla angielskiej odpowiedzi modelu, oba poziomy pary bota, wpływ poziomów i bliskości partnerów na walkę, ruch przy 5/10/15/30 Hz oraz podejście Setta i niezależne limity powiadomień. Logi same nie potwierdzają kompletności ostrzeżeń przed wszystkimi rzeczywistymi gankami.
