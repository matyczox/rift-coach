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

const SECTIONS: { id: Section; label: string }[] = [
  { id: 'lastMatch', label: 'Ostatni mecz' },
  { id: 'calibration', label: 'Kalibracja minimapy' },
  { id: 'recordings', label: 'Nagrania' },
  { id: 'harvest', label: 'Powtórki' },
  { id: 'settings', label: 'Ustawienia' },
];

export function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [section, setSection] = useState<Section>('calibration');

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
      <StatusBar status={status} error={loadError} />
      <div className="body">
        <nav className="sidebar">
          <div className="sidebar-title">Rift Coach</div>
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              type="button"
              className={'nav-item' + (section === s.id ? ' active' : '')}
              onClick={() => setSection(s.id)}
            >
              {s.label}
              {s.id === 'recordings' && status?.replay && <span className="nav-badge">odtw.</span>}
              {s.id === 'harvest' && status && isHarvestBusy(status.harvest) && <span className="nav-badge">zbiera</span>}
            </button>
          ))}
          <div className="sidebar-foot">{status ? `v${status.version}` : ''}</div>
        </nav>
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
