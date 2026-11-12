import { memo, useCallback, useEffect, useRef, useState } from 'react';
import type { HarvestStatus, ReplayFile, SourceState } from '../../shared/types';
import { fmtClock, fmtDate, rc, useAction } from './util';

const SPEEDS = [4, 8, 16];

export const HARVEST_STATE_LABEL: Record<HarvestStatus['state'], string> = {
  idle: 'bezczynne',
  launching: 'uruchamianie powtórki',
  running: 'zbieranie',
  'waiting-fog': 'czeka na zmianę mgły',
  done: 'zakończono',
  error: 'błąd',
};

type Team = NonNullable<HarvestStatus['pass']>;

export const PASS_LABEL: Record<Team, string> = {
  blue: 'Wizja niebieskich',
  red: 'Wizja czerwonych',
  all: 'Wszyscy (prawda)',
};

const FOG_TEAM_LABEL: Record<Team, string> = { blue: 'Niebieski', red: 'Czerwony', all: 'Wszystkie' };

/** Pass order used for overall progress: blue first, red second. */
const PASS_INDEX: Record<Team, number> = { blue: 0, red: 1, all: 1 };

export const isHarvestBusy = (h: HarvestStatus) =>
  h.state === 'launching' || h.state === 'running' || h.state === 'waiting-fog';

/** Progress of the current pass, 0..1. */
export function passProgress(h: HarvestStatus): number {
  if (h.lengthSec <= 0) return 0;
  return Math.min(1, Math.max(0, h.positionSec / h.lengthSec));
}

/** Progress of the current game across both passes: first pass = first half, second = second half. */
export function overallProgress(h: HarvestStatus): number {
  if (h.state === 'done') return 1;
  // Paused before a pass: everything before that pass is done.
  if (h.state === 'waiting-fog' && h.needFog) return PASS_INDEX[h.needFog] / 2;
  if (!h.pass) return 0;
  return (PASS_INDEX[h.pass] + passProgress(h)) / 2;
}

const pctText = (v: number) => `${Math.round(v * 100)}%`;

interface Props { harvest: HarvestStatus; replayApi: SourceState }

export const HarvestView = memo(function HarvestView({ harvest, replayApi }: Props) {
  const [files, setFiles] = useState<ReplayFile[] | null>(null);
  const [selected, setSelected] = useState<Set<number>>(() => new Set());
  const [speed, setSpeed] = useState(8);
  const list = useAction();
  const act = useAction();

  const refresh = useCallback(async () => {
    const r = await list.run(() => rc().listReplayFiles());
    if (r.ok) {
      const sorted = [...r.value].sort((a, b) => b.modifiedAt - a.modifiedAt);
      setFiles(sorted);
      // Drop selections that no longer exist.
      setSelected((prev) => new Set([...prev].filter((id) => sorted.some((f) => f.gameId === id))));
    }
  }, [list.run]);

  useEffect(() => { void refresh(); }, [refresh]);

  // Reload the list (✓ column) when a harvest finishes.
  const prevState = useRef(harvest.state);
  useEffect(() => {
    const was = prevState.current;
    prevState.current = harvest.state;
    if (was !== harvest.state && (harvest.state === 'done' || harvest.state === 'error' || harvest.state === 'idle')) {
      void refresh();
    }
  }, [harvest.state, refresh]);

  const busy = isHarvestBusy(harvest);

  const toggle = (id: number) => setSelected((prev) => {
    const next = new Set(prev);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });

  const allSelected = !!files && files.length > 0 && files.every((f) => selected.has(f.gameId));
  const toggleAll = () => setSelected(allSelected || !files ? new Set() : new Set(files.map((f) => f.gameId)));

  const startSelected = () => {
    if (!files) return;
    // Keep table order (newest first).
    const gameIds = files.filter((f) => selected.has(f.gameId)).map((f) => f.gameId);
    if (gameIds.length) void act.run(() => rc().harvestStart({ gameIds, speed }));
  };
  const startOpen = () => { void act.run(() => rc().harvestStart({ speed })); };
  const stop = () => { void act.run(() => rc().harvestStop()); };
  const cont = () => { void act.run(() => rc().harvestContinue()); };

  const err = list.error ?? act.error;

  return (
    <section className="section">
      <div className="section-head">
        <h1>Powtórki</h1>
        <div className="toolbar">
          <label className="speed">
            <span className="muted">Prędkość</span>
            <select value={String(speed)} onChange={(e) => setSpeed(Number(e.target.value))} disabled={busy}>
              {SPEEDS.map((s) => <option key={s} value={String(s)}>{s}×</option>)}
            </select>
          </label>
          <button
            type="button"
            className="btn"
            onClick={startOpen}
            disabled={busy || act.busy || replayApi !== 'connected'}
            title={replayApi === 'connected' ? undefined : 'Najpierw otwórz powtórkę w kliencie.'}
          >
            Zbierz z otwartej powtórki
          </button>
          <button type="button" className="btn primary" onClick={startSelected} disabled={busy || act.busy || selected.size === 0}>
            Zbierz zaznaczone{selected.size > 0 ? ` (${selected.size})` : ''}
          </button>
        </div>
      </div>

      <div className="field-hint">Zaznacz powtórki i kliknij „Zbierz zaznaczone”. Aplikacja sama je otworzy i przełączy „Mgłę wojny”. Nie ruszaj myszką przez sekundę, gdy przełącza.</div>
      {err && <div className="error-text">{err}</div>}

      <HarvestCard harvest={harvest} onStop={stop} onContinue={cont} pending={act.busy} />

      <div className="section-head">
        <h2 className="h2-inline">Pobrane powtórki</h2>
        <button type="button" className="btn small" onClick={() => void refresh()} disabled={list.busy}>Odśwież</button>
      </div>
      <div className="table-wrap">
        <table className="rec-table">
          <thead>
            <tr>
              <th className="c-check">
                <input type="checkbox" checked={allSelected} onChange={toggleAll} disabled={!files?.length || busy} aria-label="Zaznacz wszystkie" />
              </th>
              <th className="c-game">ID gry</th>
              <th className="c-date">Data</th>
              <th>Plik</th>
              <th className="c-num">Rozmiar</th>
              <th className="c-done">Zebrane</th>
            </tr>
          </thead>
          <tbody>
            {files === null ? (
              <tr><td colSpan={6} className="empty-row">{list.busy ? 'Wczytywanie…' : 'Nie udało się wczytać listy.'}</td></tr>
            ) : files.length === 0 ? (
              <tr><td colSpan={6} className="empty-row">Brak pobranych powtórek. Pobierz je w kliencie (Profil → Historia meczów).</td></tr>
            ) : files.map((f) => {
              const current = busy && harvest.gameId === f.gameId;
              const queued = busy && !!harvest.queue?.includes(f.gameId);
              return (
                <tr
                  key={f.gameId}
                  className={'rec-row' + (selected.has(f.gameId) ? ' selected' : '') + (current ? ' open' : '')}
                  onClick={() => { if (!busy) toggle(f.gameId); }}
                >
                  <td className="c-check" onClick={(e) => e.stopPropagation()}>
                    <input type="checkbox" checked={selected.has(f.gameId)} onChange={() => toggle(f.gameId)} disabled={busy} aria-label={`Zaznacz ${f.gameId}`} />
                  </td>
                  <td className="c-game num">{f.gameId}</td>
                  <td className="c-date num">{fmtDate(f.modifiedAt)}</td>
                  <td className="muted" title={f.name}>{f.name}</td>
                  <td className="c-num num">{f.sizeMb.toFixed(1)} MB</td>
                  <td className="c-done">
                    {current ? <span className="note-text">teraz</span>
                      : queued ? <span className="muted">w kolejce</span>
                      : f.harvested ? <span className="ok-text" title="Dane zebrane">✓</span>
                      : <span className="muted">—</span>}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
});

function Bar({ value, tone }: { value: number; tone?: 'error' }) {
  return (
    <div className={'bar' + (tone ? ` bar-${tone}` : '')} role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(value * 100)}>
      <div className="bar-fill" style={{ width: `${value * 100}%` }} />
    </div>
  );
}

interface CardProps {
  harvest: HarvestStatus;
  onStop: () => void;
  onContinue: () => void;
  pending: boolean;
}

function HarvestCard({ harvest: h, onStop, onContinue, pending }: CardProps) {
  if (h.state === 'idle') return null;
  const busy = isHarvestBusy(h);
  const waiting = h.state === 'waiting-fog';
  const queueLen = h.queue?.length ?? 0;
  const stateCls = h.state === 'error' ? 's-error'
    : h.state === 'launching' || waiting ? 's-waiting'
    : 's-connected';

  return (
    <div className={'player harvest-card' + (waiting ? ' waiting-fog' : '')}>
      <div className="player-main">
        <div className="player-title">
          <span className={`chip-inline ${stateCls}`}><span className="dot" />{HARVEST_STATE_LABEL[h.state]}</span>
          {h.gameId != null && <span className="muted num">gra {h.gameId}</span>}
          {h.pass && !waiting && <span>{PASS_LABEL[h.pass]}</span>}
          <span className="spacer" />
          {busy && (
            <button type="button" className="btn danger small" onClick={onStop} disabled={pending}>Zatrzymaj</button>
          )}
        </div>

        {waiting && (
          <div className="fog-prompt">
            <div className="fog-prompt-text">
              <div className="fog-prompt-title">
                Ustaw w powtórce: Mgła wojny → {h.needFog ? FOG_TEAM_LABEL[h.needFog] : '—'}
              </div>
              <div className="muted">Panel w lewym dolnym rogu powtórki. Potem kliknij Dalej.</div>
            </div>
            <button type="button" className="btn primary" onClick={onContinue} disabled={pending}>Dalej</button>
          </div>
        )}

        <div className="hv-grid">
          <span className="muted">Przebieg</span>
          <Bar value={h.state === 'done' ? 1 : passProgress(h)} tone={h.state === 'error' ? 'error' : undefined} />
          <span className="num hv-val">{fmtClock(h.positionSec)} / {fmtClock(h.lengthSec)}</span>

          <span className="muted">Ten mecz</span>
          <Bar value={overallProgress(h)} tone={h.state === 'error' ? 'error' : undefined} />
          <span className="num hv-val">{pctText(overallProgress(h))}</span>
        </div>

        <div className="info-row">
          <span><span className="muted">Klatki: </span><span className="num">{h.frames}</span></span>
          <span><span className="muted">W kolejce: </span><span className="num">{queueLen}</span></span>
        </div>
        {h.error && <div className="error-text">{h.error}</div>}
      </div>
    </div>
  );
}
