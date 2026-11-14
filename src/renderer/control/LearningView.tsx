import { useEffect, useState } from 'react';
import type { AppStatus, LearningStats } from '../../shared/types';
import { rc } from './util';
import { HarvestView } from './HarvestView';
import { RecordingsView } from './RecordingsView';

interface Source { title: string; value: string; sub: string; text: string; icon: string }

const ICONS = {
  fog: 'M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16zm0 4v4l3 2',
  self: 'M4 19V9m5 10V5m5 14v-7m5 7V8',
  people: 'M9 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6zm-6 9a6 6 0 0 1 12 0m2-9a3 3 0 1 0 0-6m2 15a6 6 0 0 0-4-5.6',
  book: 'M5 4h10a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3zm0 13a3 3 0 0 1 3-3h10',
};

/** "Nauka": what the app learns from the player's games, plus the data it learns from. */
export function LearningView({ status }: { status: AppStatus }) {
  const [s, setS] = useState<LearningStats | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () => rc().getLearning().then(v => { if (alive) { setS(v); setErr(null); } }, e => { if (alive) setErr(String(e)); });
    void load();
    const t = setInterval(load, 15000);
    return () => { alive = false; clearInterval(t); };
  }, []);

  const n = (v: number | undefined) => (v === undefined ? '—' : String(v));
  const sources: Source[] = [
    {
      title: 'Tracker mgły', icon: ICONS.fog,
      value: n(s?.fog.samples), sub: 'wyjść wroga z mgły',
      text: 'Za każdym razem, gdy wróg wychodzi z mgły, aplikacja sprawdza, gdzie przewidywała go zobaczyć, i poprawia mnożniki celów (obóz, linia, smok, invade, powrót) osobno dla każdej roli i fazy gry.',
    },
    {
      title: 'Trening na meczach', icon: ICONS.self,
      value: n(s?.observedGames), sub: 'meczów w zbiorze',
      text: 'Zapisy z gry na żywo i zaimportowane powtórki do treningu trackera. Każdy mecz liczy się raz. Po meczu aplikacja może pobrać z Riot API oś czasu z prawdziwymi pozycjami i użyć jej do dalszego uczenia.',
    },
    {
      title: 'Profile przeciwników', icon: ICONS.people,
      value: n(s?.profiles), sub: 'zapisanych profili',
      text: 'Na starcie meczu sprawdza ostatnie gry wrogów: role, pulę postaci, styl junglera (gdzie zaczyna, jak często ganka). Coach i tracker biorą to pod uwagę.',
    },
    {
      title: 'Wiedza o grze', icon: ICONS.book,
      value: n(s ? s.patchBriefs + s.matchups + s.reviews : undefined), sub: 'patchy, matchupów i analiz',
      text: 'Raz na patch czyta zmiany i metę, raz na mecz matchup na linii. Po każdym meczu zapisuje analizę: co poprawić, kluczowe momenty.',
    },
  ];

  return (
    <div className="learning">
      <div className="section-head">
        <div>
          <h2 className="section-title">Czego się nauczył</h2>
          <div className="muted small">Tylko z meczów na Summoner’s Rift. Najedź na kafelek, żeby zobaczyć, jak to działa.</div>
        </div>
      </div>
      {err && <div className="error-text small">{err}</div>}
      <div className="learn-grid">
        {sources.map(x => (
          <div key={x.title} className="learn-card" tabIndex={0}>
            <div className="learn-top">
              <span className="learn-ico"><svg viewBox="0 0 24 24" width="20" height="20"><path d={x.icon} fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" /></svg></span>
              <span className="learn-title">{x.title}</span>
            </div>
            <div className="learn-value num">{x.value}</div>
            <div className="learn-sub">{x.sub}</div>
            <div className="learn-detail">
              <div className="learn-detail-title">{x.title}</div>
              <p>{x.text}</p>
            </div>
          </div>
        ))}
      </div>

      <div className="learn-section">
        <HarvestView harvest={status.harvest} replayApi={status.replayApi} />
      </div>
      <div className="learn-section">
        <RecordingsView recording={status.recording} replay={status.replay} />
      </div>
    </div>
  );
}
