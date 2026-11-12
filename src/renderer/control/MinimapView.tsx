import { useState } from 'react';
import type { AppStatus } from '../../shared/types';
import { errText, fmtDate, rc } from './util';
import { MinimapPreview } from './MinimapPreview';
import { CalibrationView } from './CalibrationView';

/** Minimapa: found automatically at every game start; manual placement only if detection fails. */
export function MinimapView({ status }: { status: AppStatus }) {
  const [manual, setManual] = useState(false);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const cal = status.settings.calibration;
  const cap = status.capture;
  const mismatch = !!cal && cap.windowWidth != null && cap.windowHeight != null && (cap.windowWidth !== cal.windowWidth || cap.windowHeight !== cal.windowHeight);
  const ok = !!cal && !mismatch;

  const detect = async () => {
    setBusy(true); setMsg(null);
    try {
      const c = await rc().autoCalibrate();
      setMsg(c ? 'Minimapa znaleziona.' : 'Nie znalazłem minimapy — otwórz grę (nie ekran ładowania) i spróbuj ponownie.');
    } catch (e) { setMsg(errText(e)); } finally { setBusy(false); }
  };

  return (
    <div className="mm">
      <div className="mm-main">
        <div className={'mm-status ' + (ok ? 'ok' : 'warn')}>
          <span className="mm-badge">{ok ? '✓' : '!'}</span>
          <div className="mm-text">
            <div className="mm-title">{ok ? 'Minimapa wykryta' : cal ? 'Okno gry ma inny rozmiar' : 'Minimapa jeszcze niewykryta'}</div>
            <div className="mm-sub">
              {ok
                ? `${cal!.source === 'auto' ? 'Automatycznie' : 'Ręcznie'} · ${cal!.rect.width} px · ${cal!.windowWidth}×${cal!.windowHeight} · ${fmtDate(cal!.updatedAt)}`
                : 'Aplikacja wykrywa ją sama przy starcie gry. Możesz też kliknąć „Wykryj teraz” w trakcie meczu.'}
            </div>
          </div>
          <button type="button" className="btn primary" onClick={() => void detect()} disabled={busy || cap.state !== 'connected'} title={cap.state !== 'connected' ? 'Najpierw uruchom grę.' : undefined}>
            {busy ? 'Szukam…' : ok ? 'Wykryj ponownie' : 'Wykryj teraz'}
          </button>
        </div>
        {msg && <div className="small muted">{msg}</div>}
        <button type="button" className="btn ghost small mm-manual" onClick={() => setManual(m => !m)}>{manual ? 'Ukryj ręczne ustawianie' : 'Ustaw ręcznie'}</button>
        {manual && (
          <div className="mm-manual-box">
            <CalibrationView calibration={cal} captureState={cap.state} lastAdvice={status.lastAdvice} windowWidth={cap.windowWidth} windowHeight={cap.windowHeight} />
          </div>
        )}
      </div>
      <aside className="mm-side">
        <div className="side-title">Podgląd na żywo</div>
        <MinimapPreview size={260} />
        <div className="muted small">Tak aplikacja widzi minimapę w grze.</div>
      </aside>
    </div>
  );
}
