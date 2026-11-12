import { useEffect, useState } from 'react';
import type { AppStatus } from '../../shared/types';
import { errText, fmtClock, rc } from './util';
import { StatusBar } from './StatusBar';
import { CalibrationView } from './CalibrationView';
import { RecordingsView } from './RecordingsView';
import { SettingsView } from './SettingsView';
import { HarvestView, isHarvestBusy } from './HarvestView';
import { HomeView } from './HomeView';

type Section = 'home' | 'calibration' | 'recordings' | 'harvest' | 'settings';

const ICON: Record<Section, string> = {
  home: 'M3 11l9-7 9 7v9a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z',
  calibration: 'M12 3v4m0 10v4M3 12h4m10 0h4M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z',
  recordings: 'M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16zm0 5a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  harvest: 'M4 6h16v12H4zM10 9.5v5l4.5-2.5z',
  settings: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zm7.4 3a7.4 7.4 0 0 0-.1-1.2l2-1.6-2-3.4-2.4 1a7 7 0 0 0-2-1.2L14.5 3h-5l-.4 2.6a7 7 0 0 0-2 1.2l-2.4-1-2 3.4 2 1.6a7.4 7.4 0 0 0 0 2.4l-2 1.6 2 3.4 2.4-1a7 7 0 0 0 2 1.2l.4 2.6h5l.4-2.6a7 7 0 0 0 2-1.2l2.4 1 2-3.4-2-1.6c.1-.4.1-.8.1-1.2z',
};

const TABS: Array<{ id: Section; label: string }> = [
  { id: 'home', label: 'Główna' },
  { id: 'calibration', label: 'Minimapa' },
  { id: 'recordings', label: 'Nagrania' },
  { id: 'harvest', label: 'Powtórki' },
  { id: 'settings', label: 'Ustawienia' },
];

function Tab({ id, label, active, badge, onClick }: { id: Section; label: string; active: boolean; badge?: string; onClick: () => void }) {
  return (
    <button type="button" className={'tab' + (active ? ' active' : '')} onClick={onClick} aria-current={active ? 'page' : undefined}>
      <svg viewBox="0 0 24 24" width="15" height="15"><path d={ICON[id]} fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" /></svg>
      {label}
      {badge && <span className={'tab-badge' + (badge.startsWith('●') ? ' live' : '')}>{badge}</span>}
    </button>
  );
}

export function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [section, setSection] = useState<Section>('home');

  useEffect(() => {
    let unsub: (() => void) | undefined;
    let cancelled = false;
    try {
      const api = rc();
      unsub = api.onStatus((s) => {
        if (!cancelled) { setStatus(s); setLoadError(null); }
      });
      api.getStatus().then(
        (s) => { if (!cancelled) setStatus((prev) => prev ?? s); },
        (e: unknown) => { if (!cancelled) setLoadError(errText(e)); },
      );
    } catch (e) {
      setLoadError(errText(e));
    }
    return () => {
      cancelled = true;
      try { unsub?.(); } catch { /* ignore */ }
    };
  }, []);

  const badge = (id: Section): string | undefined => {
    if (!status) return undefined;
    if (id === 'home' && status.game) return '● ' + fmtClock(status.game.gameTime);
    if (id === 'recordings' && status.recording.active) return 'REC';
    if (id === 'harvest' && isHarvestBusy(status.harvest)) return 'zbiera';
    return undefined;
  };

  return (
    <div className="app">
      <header className="titlebar">
        <div className="brand"><img src="icon.png" alt="" /><span className="brand-name">RIFT COACH</span><span className="brand-ver">{status ? `v${status.version}` : ''}</span></div>
      </header>
      <nav className="tabs">
        <div className="tabs-left" />
        <div className="tabs-center">
          {TABS.map(t => <Tab key={t.id} id={t.id} label={t.label} active={section === t.id} badge={badge(t.id)} onClick={() => setSection(t.id)} />)}
        </div>
        <StatusBar status={status} error={loadError} />
      </nav>
      <main className="content">
        <div className="page">
          {!status ? (
            <div className="empty">{loadError ? 'Nie udało się pobrać stanu aplikacji.' : 'Łączenie…'}</div>
          ) : section === 'home' ? (
            <HomeView status={status} />
          ) : section === 'calibration' ? (
            <CalibrationView
              calibration={status.settings.calibration}
              captureState={status.capture.state}
              lastAdvice={status.lastAdvice}
              windowWidth={status.capture.windowWidth}
              windowHeight={status.capture.windowHeight}
            />
          ) : section === 'recordings' ? (
            <RecordingsView recording={status.recording} replay={status.replay} />
          ) : section === 'harvest' ? (
            <HarvestView harvest={status.harvest} replayApi={status.replayApi} />
          ) : (
            <SettingsView settings={status.settings} />
          )}
        </div>
      </main>
    </div>
  );
}
