import { memo } from 'react';
import type { AppStatus, GameInfo, SourceState } from '../../shared/types';
import type { CoachStatus } from '../../shared/intel';
import { fmtClock, fmtMB, lcuPhaseLabel, STATE_LABEL } from './util';
import { isHarvestBusy, overallProgress, PASS_LABEL } from './HarvestView';

interface ChipProps {
  label: string;
  state: SourceState;
  value: string;
  title?: string;
}

const Chip = memo(function Chip({ label, state, value, title }: ChipProps) {
  return (
    <div className={`chip s-${state}`} title={title}>
      <span className="dot" />
      <span className="chip-label">{label}</span>
      <span className="chip-value">{value}</span>
    </div>
  );
});

function coachChipOf(c: CoachStatus): { state: SourceState; value: string } {
  const lat = c.lastLatencyMs != null ? ` · ${(c.lastLatencyMs / 1000).toFixed(1)} s` : '';
  switch (c.state) {
    case 'off': return { state: 'off', value: 'wyłączony' };
    case 'not-logged-in': return { state: 'error', value: 'niezalogowany' };
    case 'idle': return { state: 'connected', value: 'gotowy' + lat };
    case 'thinking': return { state: 'connected', value: 'myśli…' + lat };
    case 'error': return { state: 'error', value: 'błąd' };
  }
}

function profilesChipOf(p: AppStatus['profiles']): { state: SourceState; value: string } {
  switch (p.state) {
    case 'idle': return { state: 'off', value: 'brak' };
    case 'loading': return { state: 'waiting', value: 'pobieranie' };
    case 'ready': return { state: 'connected', value: `${p.count} gotowe` };
    case 'error': return { state: 'error', value: 'błąd' };
    case 'no-key': return { state: 'off', value: 'brak klucza' };
  }
}

function runesChipOf(r: AppStatus['runes']): { state: SourceState; value: string; title?: string } {
  switch (r.state) {
    case 'off': return { state: 'off', value: 'wyłączone' };
    case 'idle': return { state: 'off', value: 'czeka' };
    case 'working': return { state: 'waiting', value: 'dobieram…' };
    case 'set': return { state: 'connected', value: r.label ?? 'ustawione', title: r.label ?? undefined };
    case 'no-slot': return { state: 'waiting', value: 'brak miejsca', title: r.error ?? undefined };
    case 'error': return { state: 'error', value: 'błąd', title: r.error ?? undefined };
  }
}

const GameLine = memo(function GameLine({ game }: { game: GameInfo | null }) {
  if (!game) return <div className="game-line muted">Brak aktywnej gry</div>;
  return (
    <div className="game-line">
      <span className="gl-item"><span className="muted">Tryb</span> {game.gameMode || '—'}</span>
      <span className="gl-item num">{fmtClock(game.gameTime)}</span>
      <span className="gl-item"><span className="muted">Ty</span> {game.activeChampion ?? '—'}</span>
      <span className="gl-item gl-teams" title={`${game.allies.join(', ')} vs ${game.enemies.join(', ')}`}>
        <span className="ally">{game.allies.join(', ') || '—'}</span>
        <span className="muted"> vs </span>
        <span className="enemy">{game.enemies.join(', ') || '—'}</span>
      </span>
    </div>
  );
});

export const StatusBar = memo(function StatusBar({ status, error }: { status: AppStatus | null; error: string | null }) {
  if (!status) {
    return (
      <header className="statusbar">
        <div className="chips">
          <Chip label="Aplikacja" state={error ? 'error' : 'waiting'} value={error ? 'błąd' : 'łączenie'} title={error ?? undefined} />
        </div>
        {error && <div className="game-line error-text">{error}</div>}
      </header>
    );
  }

  const { capture, recording, settings } = status;
  const cal = settings.calibration;

  const lcuPhase = lcuPhaseLabel(status.lcuPhase);
  const lcuValue = status.lcu === 'connected' && lcuPhase ? lcuPhase : STATE_LABEL[status.lcu];

  const capValue = capture.state === 'connected' && capture.windowWidth && capture.windowHeight
    ? `${capture.windowWidth}×${capture.windowHeight} · ${capture.fps.toFixed(0)} fps`
    : STATE_LABEL[capture.state];

  let calState: SourceState = 'waiting';
  let calValue = 'brak';
  let calTitle = 'Minimapa nie jest skalibrowana.';
  if (cal) {
    const mismatch = capture.windowWidth != null && capture.windowHeight != null
      && (capture.windowWidth !== cal.windowWidth || capture.windowHeight !== cal.windowHeight);
    calState = mismatch ? 'waiting' : 'connected';
    calValue = mismatch ? 'inny rozmiar' : (cal.source === 'auto' ? 'auto' : 'ręczna');
    calTitle = `Zmierzona dla ${cal.windowWidth}×${cal.windowHeight}` + (mismatch ? ' — okno gry ma teraz inny rozmiar.' : '');
  }

  const coachChip = coachChipOf(status.coach);
  const profChip = profilesChipOf(status.profiles);
  const runesChip = runesChipOf(status.runes);

  const recValue =recording.active ? `${recording.frames} kl. · ${fmtMB(recording.bytes)}` : 'wyłączone';

  return (
    <header className="statusbar">
      <div className="chips">
        <Chip label="Gra / Live API" state={status.liveApi} value={STATE_LABEL[status.liveApi]} />
        <Chip label="Klient" state={status.lcu} value={lcuValue} title={status.lcuPhase ?? undefined} />
        <Chip label="Przechwytywanie" state={capture.state} value={capValue} title={capture.error ?? undefined} />
        <Chip label="Kalibracja" state={calState} value={calValue} title={calTitle} />
        <Chip label="Nagrywanie" state={recording.active ? 'connected' : 'off'} value={recValue} />
        <Chip label="Coach" state={coachChip.state} value={coachChip.value} title={status.coach.lastError ?? (status.coach.model ? `Model: ${status.coach.model} · porad: ${status.coach.calls}` : undefined)} />
        <Chip label="Profile wrogów" state={profChip.state} value={profChip.value} title={status.profiles.error ?? undefined} />
        <Chip label="Runy" state={runesChip.state} value={runesChip.value} title={runesChip.title} />
        <Chip label="Powtórka" state={status.replayApi} value={status.replayApi === 'connected' ? 'otwarta' : STATE_LABEL[status.replayApi]} />
        {isHarvestBusy(status.harvest) && (
          <Chip
            label="Zbieranie"
            state={status.harvest.state === 'running' ? 'connected' : 'waiting'}
            value={status.harvest.state === 'launching'
              ? 'uruchamianie'
              : status.harvest.state === 'waiting-fog'
              ? 'zmień mgłę'
              : `${status.harvest.pass ? PASS_LABEL[status.harvest.pass] + ' · ' : ''}${Math.round(overallProgress(status.harvest) * 100)}%`
                + (status.harvest.queue?.length ? ` · +${status.harvest.queue.length}` : '')}
            title={status.harvest.gameId != null ? `Gra ${status.harvest.gameId}` : undefined}
          />
        )}
        {status.replay && (
          <Chip label="Odtwarzanie" state="connected" value={status.replay.playing ? 'gra' : 'pauza'} />
        )}
      </div>
      <GameLine game={status.game} />
    </header>
  );
});
