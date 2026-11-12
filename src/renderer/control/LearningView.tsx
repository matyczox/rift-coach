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
      value: n(s?.fog.samples), sub: s?.fog.samples ? `powrotów z mgły · ${s.fog.patterns} wzorców` : 'jeszcze bez danych',
      text: 'Za każdym razem, gdy wróg wychodzi z mgły, aplikacja sprawdza, gdzie przewidywała go zobaczyć, i poprawia mnożniki celów (obóz, linia, smok, invade, powrót) osobno dla każdej roli i fazy gry.',
    },
    {
      title: 'Samotrening', icon: ICONS.self,
      value: n(s?.observedGames), sub: 'zapisanych meczów',
      text: 'Po meczu na Summoner’s Rift pobiera z Riot API oś czasu z prawdziwymi pozycjami wszystkich graczy co minutę i trenuje na nich tracker: odtwarza mecz i uczy się z każdej pomyłki.',
    },
    {
      title: 'Profile przeciwników', icon: ICONS.people,
      value: n(s?.profiles), sub: 'odpowiedzi Riot API w pamięci',
      text: 'Na starcie meczu sprawdza ostatnie gry wrogów: role, pulę postaci, styl junglera (gdzie zaczyna, jak często ganka). Coach i tracker biorą to pod uwagę.',
    },
    {
      title: 'Patch i matchupy', icon: ICONS.book,
      value: n(s ? s.patchBriefs + s.matchups : undefined), sub: s ? `${s.patchBriefs} patch · ${s.matchups} matchupów · ${s.reviews} analiz` : '',
      text: 'Raz na patch czyta zmiany i metę, raz na mecz matchup na linii. Po każdym meczu zapisuje analizę: co poprawić, kluczowe momenty.',
    },
  ];

  return (
    <div className="learning">
      <div className="section-head">
        <div>
          <h2 className="section-title">Jak Rift Coach się uczy</h2>
          <div className="muted small">Uczy się tylko na prawdziwych meczach na Summoner’s Rift — nie na treningu, ARAM-ie ani Arenie.</div>
        </div>
      </div>
      {err && <div className="error-text small">{err}</div>}
      <div className="learn-grid">
        {sources.map(x => (
          <div key={x.title} className="learn-card">
            <div className="learn-top">
              <span className="learn-ico"><svg viewBox="0 0 24 24" width="20" height="20"><path d={x.icon} fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" /></svg></span>
              <span className="learn-title">{x.title}</span>
            </div>
            <div className="learn-value num">{x.value}</div>
            <div className="learn-sub">{x.sub}</div>
            <p className="learn-text">{x.text}</p>
          </div>
        ))}
      </div>

      <div className="learn-section">
        <h2 className="section-title">Dane z powtórek</h2>
        <div className="muted small">Najszybszy sposób nauki: aplikacja odtwarza Twoje powtórki i widzi, gdzie naprawdę byli wrogowie.</div>
        <HarvestView harvest={status.harvest} replayApi={status.replayApi} />
      </div>
      <div className="learn-section">
        <h2 className="section-title">Nagrania</h2>
        <RecordingsView recording={status.recording} replay={status.replay} />
      </div>
    </div>
  );
}
