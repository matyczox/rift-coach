import { memo, useCallback, useEffect, useRef, useState } from 'react';
import type { RecordingStatus, RecordingSummary, ReplayStatus } from '../../shared/types';
import { fmtDate, fmtMB, fmtMs, rc, useAction } from './util';
import { MinimapPreview } from './MinimapPreview';

const SPEEDS = [0.5, 1, 2, 4, 8];

interface Props { recording: RecordingStatus; replay: ReplayStatus | null }

export const RecordingsView = memo(function RecordingsView({ recording, replay }: Props) {
  const [items, setItems] = useState<RecordingSummary[] | null>(null);
  const list = useAction();
  const rec = useAction();
  const act = useAction();
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [openingId, setOpeningId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const r = await list.run(() => rc().listRecordings());
    if (r.ok) setItems([...r.value].sort((a, b) => b.startedAt - a.startedAt));
  }, [list.run]);

  useEffect(() => { void refresh(); }, [refresh]);

  // Refresh when a recording starts or finishes.
  const wasActive = useRef(recording.active);
  useEffect(() => {
    if (wasActive.current !== recording.active) {
      wasActive.current = recording.active;
      void refresh();
    }
  }, [recording.active, refresh]);

  // Auto-cancel the delete confirmation.
  useEffect(() => {
    if (!confirmId) return;
    const t = window.setTimeout(() => setConfirmId(null), 4000);
    return () => window.clearTimeout(t);
  }, [confirmId]);

  const toggleRecording = () => {
    void rec.run(() => (recording.active ? rc().stopRecording() : rc().startRecording()));
  };

  const onDelete = async (id: string) => {
    if (confirmId !== id) { setConfirmId(id); return; }
    setConfirmId(null);
    const r = await act.run(() => rc().deleteRecording(id));
    if (r.ok) setItems((prev) => prev?.filter((x) => x.id !== id) ?? null);
  };

  const onOpen = async (id: string) => {
    if (replay?.sessionId === id) return;
    setOpeningId(id);
    await act.run(() => rc().replayOpen(id));
    setOpeningId(null);
  };

  const err = list.error ?? rec.error ?? act.error;

  return (
    <section className="section">
      <div className="section-head">
        <h1>Nagrania</h1>
        <div className="toolbar">
          <button type="button" className={'btn' + (recording.active ? ' danger' : ' primary')} onClick={toggleRecording} disabled={rec.busy}>
            {recording.active ? 'Zatrzymaj nagrywanie' : 'Nagrywaj'}
          </button>
          <button type="button" className="btn" onClick={() => void refresh()} disabled={list.busy}>Odśwież</button>
          <button type="button" className="btn" onClick={() => void act.run(() => rc().openRecordingsFolder())}>Otwórz folder</button>
        </div>
      </div>

      {recording.active && (
        <div className="info-row">
          <span className="rec-dot" />
          <span>Nagrywanie trwa: <span className="num">{recording.frames}</span> klatek · <span className="num">{fmtMB(recording.bytes)}</span>
            {recording.startedAt != null && <> · od {fmtDate(recording.startedAt)}</>}
          </span>
        </div>
      )}
      {err && <div className="error-text">{err}</div>}

      {replay && <ReplayPlayer replay={replay} title={items?.find((x) => x.id === replay.sessionId)} />}

      <div className="table-wrap">
        <table className="rec-table">
          <thead>
            <tr>
              <th className="c-date">Data</th>
              <th className="c-num">Czas</th>
              <th className="c-champ">Postać</th>
              <th>Przeciwnicy</th>
              <th className="c-num">Klatki</th>
              <th className="c-num">Rozmiar</th>
              <th className="c-act" aria-label="Akcje" />
            </tr>
          </thead>
          <tbody>
            {items === null ? (
              <tr><td colSpan={7} className="empty-row">{list.busy ? 'Wczytywanie…' : 'Nie udało się wczytać listy.'}</td></tr>
            ) : items.length === 0 ? (
              <tr><td colSpan={7} className="empty-row">Brak nagrań.</td></tr>
            ) : items.map((it) => {
              const isOpen = replay?.sessionId === it.id;
              const isLive = recording.active && recording.sessionId === it.id;
              return (
                <tr
                  key={it.id}
                  className={'rec-row' + (isOpen ? ' open' : '') + (isLive ? ' live' : '')}
                  onClick={() => { if (!isLive) void onOpen(it.id); }}
                  title={isLive ? 'Nagrywanie w toku' : 'Kliknij, aby odtworzyć'}
                >
                  <td className="c-date num">{fmtDate(it.startedAt)}</td>
                  <td className="c-num num">{fmtMs(it.durationMs)}</td>
                  <td className="c-champ">
                    {it.champion ?? '—'}
                    {it.gameMode && <span className="muted small"> · {it.gameMode}</span>}
                  </td>
                  <td className="c-enemies">{it.enemies.join(', ') || '—'}</td>
                  <td className="c-num num">{it.frames}</td>
                  <td className="c-num num">{fmtMB(it.bytes)}</td>
                  <td className="c-act" onClick={(e) => e.stopPropagation()}>
                    {openingId === it.id ? <span className="muted small">Otwieranie…</span> : (
                      <button
                        type="button"
                        className={'btn small' + (confirmId === it.id ? ' danger' : ' ghost')}
                        disabled={isOpen || isLive || act.busy}
                        onClick={() => void onDelete(it.id)}
                      >
                        {confirmId === it.id ? 'Na pewno?' : 'Usuń'}
                      </button>
                    )}
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

function ReplayPlayer({ replay, title }: { replay: ReplayStatus; title: RecordingSummary | undefined }) {
  const ctl = useAction();
  const [scrub, setScrub] = useState<number | null>(null);
  const pos = scrub ?? replay.positionMs;
  const dur = Math.max(1, replay.durationMs);

  const commitSeek = () => {
    if (scrub === null) return;
    const v = scrub;
    void ctl.run(() => rc().replaySeek(v)).then(() => setScrub(null));
  };

  return (
    <div className="player">
      <div className="player-main">
        <div className="player-title">
          <span>Odtwarzanie</span>
          <span className="muted">
            {title ? `${fmtDate(title.startedAt)} · ${title.champion ?? '—'}` : replay.sessionId}
          </span>
        </div>
        <div className="player-controls">
          <button
            type="button"
            className="btn primary play-btn"
            onClick={() => void ctl.run(() => (replay.playing ? rc().replayPause() : rc().replayPlay()))}
          >
            {replay.playing ? 'Pauza' : 'Odtwórz'}
          </button>
          <span className="num time">{fmtMs(pos)}</span>
          <input
            type="range"
            className="seek"
            min={0}
            max={dur}
            step={250}
            value={Math.min(pos, dur)}
            onChange={(e) => setScrub(Number(e.target.value))}
            onPointerUp={commitSeek}
            onKeyUp={commitSeek}
            onBlur={commitSeek}
            aria-label="Pozycja"
          />
          <span className="num time">{fmtMs(replay.durationMs)}</span>
          <label className="speed">
            <span className="muted">Prędkość</span>
            <select
              value={String(replay.speed)}
              onChange={(e) => { const v = Number(e.target.value); void ctl.run(() => rc().replaySpeed(v)); }}
            >
              {!SPEEDS.includes(replay.speed) && <option value={String(replay.speed)}>{replay.speed}×</option>}
              {SPEEDS.map((s) => <option key={s} value={String(s)}>{s}×</option>)}
            </select>
          </label>
          <button type="button" className="btn" onClick={() => void ctl.run(() => rc().replayClose())}>Zamknij</button>
        </div>
        {ctl.error && <div className="error-text">{ctl.error}</div>}
      </div>
      <MinimapPreview size={180} />
    </div>
  );
}
