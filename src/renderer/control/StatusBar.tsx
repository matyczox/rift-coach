import { memo, useEffect, useRef, useState } from 'react';
import type { AppStatus, SourceState } from '../../shared/types';
import type { CoachStatus } from '../../shared/intel';
import { fmtClock, fmtMB, lcuPhaseLabel, STATE_LABEL } from './util';
import { isHarvestBusy, overallProgress, PASS_LABEL } from './HarvestView';

interface Item { label: string; state: SourceState; value: string; title?: string }

function coachOf(c: CoachStatus): { state: SourceState; value: string } {
  const lat = c.lastLatencyMs != null ? ` · ${(c.lastLatencyMs / 1000).toFixed(1)} s` : '';
  switch (c.state) {
    case 'off': return { state: 'off', value: 'wyłączony' };
    case 'not-logged-in': return { state: 'error', value: 'niezalogowany' };
    case 'idle': return { state: 'connected', value: 'gotowy' + lat };
    case 'thinking': return { state: 'connected', value: 'myśli…' + lat };
    case 'error': return { state: 'error', value: 'błąd' };
  }
}

function profilesOf(p: AppStatus['profiles']): { state: SourceState; value: string } {
  switch (p.state) {
    case 'idle': return { state: 'off', value: 'brak' };
    case 'loading': return { state: 'waiting', value: 'pobieranie' };
    case 'ready': return { state: 'connected', value: `${p.count} gotowe` };
    case 'error': return { state: 'error', value: 'błąd' };
    case 'no-key': return { state: 'waiting', value: 'brak klucza Riot' };
  }
}

function runesOf(r: AppStatus['runes']): { state: SourceState; value: string; title?: string } {
  switch (r.state) {
    case 'off': return { state: 'off', value: 'wyłączone' };
    case 'idle': return { state: 'off', value: 'czeka na wybór postaci' };
    case 'working': return { state: 'waiting', value: 'dobieram…' };
    case 'set': return { state: 'connected', value: r.label ?? 'ustawione', title: r.label ?? undefined };
    case 'no-slot': return { state: 'waiting', value: 'brak miejsca na stronę', title: r.error ?? undefined };
    case 'error': return { state: 'error', value: 'błąd', title: r.error ?? undefined };
  }
}

/** "gpt-6.1-terra" -> "Terra", "claude-opus-5-5" -> "Opus 5.5". */
const modelLabel = (m: string | null) => !m ? "gotowy" : m.replace(/^gpt-[\d.]+-/, "").replace(/^claude-/, "").replace(/-(\d+)-(\d+)$/, " $1.$2").replace(/^./, c => c.toUpperCase());

/** Every subsystem as one row of the status panel. */
function systemItems(s: AppStatus): Item[] {
  const { capture, recording, settings } = s;
  const cal = settings.calibration;
  const lcuPhase = lcuPhaseLabel(s.lcuPhase);
  const items: Item[] = [
    { label: 'Gra (Live API)', state: s.liveApi, value: STATE_LABEL[s.liveApi] },
    { label: 'Klient LoL', state: s.lcu, value: s.lcu === 'connected' && lcuPhase ? lcuPhase : STATE_LABEL[s.lcu], title: s.lcuPhase ?? undefined },
    {
      label: 'Obraz gry', state: capture.state, title: capture.error ?? undefined,
      value: capture.state === 'connected' && capture.windowWidth && capture.windowHeight
        ? `${capture.windowWidth}×${capture.windowHeight} · ${capture.fps.toFixed(0)} fps` : STATE_LABEL[capture.state],
    },
  ];
  if (cal) {
    const mismatch = capture.windowWidth != null && capture.windowHeight != null
      && (capture.windowWidth !== cal.windowWidth || capture.windowHeight !== cal.windowHeight);
    items.push({ label: 'Minimapa', state: mismatch ? 'waiting' : 'connected', value: mismatch ? 'inny rozmiar okna' : (cal.source === 'auto' ? 'skalibrowana (auto)' : 'skalibrowana ręcznie'), title: `Zmierzona dla ${cal.windowWidth}×${cal.windowHeight}` });
  } else items.push({ label: 'Minimapa', state: 'waiting', value: 'nieskalibrowana' });
  const coach = coachOf(s.coach);
  items.push({ label: 'Coach AI', ...coach, title: s.coach.lastError ?? (s.coach.model ? `Model: ${s.coach.model} · porad: ${s.coach.calls}` : undefined) });
  items.push({ label: 'Profile wrogów', ...profilesOf(s.profiles), title: s.profiles.error ?? undefined });
  items.push({ label: 'Runy', ...runesOf(s.runes) });
  items.push({ label: 'Nagrywanie', state: recording.active ? 'connected' : 'off', value: recording.active ? `${recording.frames} kl. · ${fmtMB(recording.bytes)}` : 'wyłączone' });
  items.push({ label: 'Powtórka', state: s.replayApi, value: s.replayApi === 'connected' ? 'otwarta' : STATE_LABEL[s.replayApi] });
  if (isHarvestBusy(s.harvest)) items.push({
    label: 'Zbieranie danych', state: s.harvest.state === 'running' ? 'connected' : 'waiting',
    value: s.harvest.state === 'launching' ? 'uruchamianie' : s.harvest.state === 'waiting-fog' ? 'zmień mgłę wojny'
      : `${s.harvest.pass ? PASS_LABEL[s.harvest.pass] + ' · ' : ''}${Math.round(overallProgress(s.harvest) * 100)}%`,
  });
  return items;
}

export const Portrait = memo(function Portrait({ id, name, version, side }: { id?: string; name: string; version: string | null; side: 'ally' | 'enemy' }) {
  const [broken, setBroken] = useState(false);
  const src = id && version && !broken ? `https://ddragon.leagueoflegends.com/cdn/${version}/img/champion/${id}.png` : null;
  return (
    <span className={`portrait ${side}`} title={name}>
      {src ? <img src={src} alt={name} onError={() => setBroken(true)} /> : <span className="portrait-fallback">{name.slice(0, 2)}</span>}
    </span>
  );
});

function SystemPill({ items, thinking }: { items: Item[]; thinking?: boolean }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    window.addEventListener('mousedown', close);
    return () => window.removeEventListener('mousedown', close);
  }, [open]);
  const errors = items.filter(i => i.state === 'error').length;
  // 'waiting' on game / capture / replay just means no game is running: not something to fix.
  const PASSIVE = ['Gra (Live API)', 'Obraz gry', 'Powtórka', 'Zbieranie danych'];
  const warns = items.filter(i => i.state === 'waiting' && !PASSIVE.includes(i.label)).length;
  const tone = errors ? 'error' : warns ? 'waiting' : 'connected';
  // All fine and the coach idle: the Gotowość card already says so; keep the title bar clean.
  if (!errors && !warns && !thinking) return null;
  const text = thinking && !errors ? 'Coach myśli…' : errors ? `${errors} ${errors === 1 ? 'problem' : 'problemy'}` : warns ? `${warns} do uwagi` : 'Wszystko działa';
  return (
    <div className="sys" ref={ref}>
      <button type="button" className={`sys-pill s-${tone}`} onClick={() => setOpen(o => !o)} aria-expanded={open}>
        <span className="dot" />{text}
        <svg className={'chev' + (open ? ' up' : '')} viewBox="0 0 16 16" width="12" height="12"><path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      {open && (
        <div className="sys-panel">
          {items.map(i => (
            <div key={i.label} className={`sys-row s-${i.state}`} title={i.title}>
              <span className="dot" /><span className="sys-label">{i.label}</span><span className="sys-value">{i.value}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/** Right side of the title bar: live clock, coach model, system health. */
export const StatusBar = memo(function StatusBar({ status, error }: { status: AppStatus | null; error: string | null }) {
  if (!status) return <div className="tb-status">{error ? <span className="error-text small">{error}</span> : <span className="muted small">Łączenie…</span>}</div>;
  const coach = coachOf(status.coach);
  return (
    <div className="tb-status">
      <SystemPill items={systemItems(status)} thinking={status.coach.state === 'thinking'} />
    </div>
  );
});
