import { useEffect, useState } from 'react';
import type { AppStatus } from '../../shared/types';
import { errText, rc } from './util';
import { StatusBar } from './StatusBar';
import { CalibrationView } from './CalibrationView';
import { RecordingsView } from './RecordingsView';
import { SettingsView } from './SettingsView';
import { HarvestView, isHarvestBusy } from './HarvestView';
import { LastMatchView } from './LastMatchView';

type Section = 'lastMatch' | 'calibration' | 'recordings' | 'harvest' | 'settings';

const ICON: Record<Section, string> = {
  lastMatch: 'M4 19V9m5 10V5m5 14v-7m5 7V8',
  calibration: 'M12 3v4m0 10v4M3 12h4m10 0h4M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z',
  recordings: 'M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16zm0 5a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  harvest: 'M4 6h16v12H4zM9 9.5v5l4.5-2.5z',
  settings: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zm7.4 3a7.4 7.4 0 0 0-.1-1.2l2-1.6-2-3.4-2.4 1a7 7 0 0 0-2-1.2L14.5 3h-5l-.4 2.6a7 7 0 0 0-2 1.2l-2.4-1-2 3.4 2 1.6a7.4 7.4 0 0 0 0 2.4l-2 1.6 2 3.4 2.4-1a7 7 0 0 0 2 1.2l.4 2.6h5l.4-2.6a7 7 0 0 0 2-1.2l2.4 1 2-3.4-2-1.6c.1-.4.1-.8.1-1.2z',
};

const SECTIONS: { id: Section; label: string }[] = [
  { id: 'lastMatch', label: 'Mecz' },
  { id: 'calibration', label: 'Minimapa' },
  { id: 'recordings', label: 'Nagrania' },
  { id: 'harvest', label: 'Powtórki' },
];

function NavButton({ id, label, active, badge, onClick }: { id: Section; label: string; active: boolean; badge?: boolean; onClick: () => void }) {
  return (
    <button type="button" className={'rail-item' + (active ? ' active' : '')} onClick={onClick} title={label} aria-current={active ? 'page' : undefined}>
      <svg viewBox="0 0 24 24" width="22" height="22"><path d={ICON[id]} fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" /></svg>
      <span>{label}</span>
      {badge && <i className="rail-badge" />}
    </button>
  );
}

export function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [section, setSection] = useState<Section>('lastMatch');

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

  return (
    <div className="app">
      <nav className="rail">
        <img className="rail-logo" src="icon.png" alt="Rift Coach" />
        {SECTIONS.map((s) => (
          <NavButton key={s.id} id={s.id} label={s.label} active={section === s.id} onClick={() => setSection(s.id)}
            badge={(s.id === 'recordings' && Boolean(status?.replay)) || (s.id === 'harvest' && Boolean(status && isHarvestBusy(status.harvest)))} />
        ))}
        <div className="rail-spacer" />
        <NavButton id="settings" label="Ustawienia" active={section === 'settings'} onClick={() => setSection('settings')} />
        <div className="rail-foot">{status ? `v${status.version}` : ''}</div>
      </nav>
      <div className="main">
        <StatusBar status={status} error={loadError} />
        <main className="content">
          {!status ? (
            <div className="empty">{loadError ? 'Nie udało się pobrać stanu aplikacji.' : 'Łączenie…'}</div>
          ) : section === 'lastMatch' ? (
            <LastMatchView />
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
        </main>
      </div>
    </div>
  );
}
