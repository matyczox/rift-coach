import { memo } from 'react';
import type { AppStatus, GameInfo } from '../../shared/types';
import type { CoachAdvice } from '../../shared/intel';
import { fmtClock, lcuPhaseLabel } from './util';
import { Portrait } from './StatusBar';
import { LastMatchView } from './LastMatchView';

const TONE: Record<CoachAdvice['tone'], string> = { go: 'Rób', wait: 'Czekaj', danger: 'Uwaga', farm: 'Farm' };

function TeamList({ names, ids, side, me, version }: { names: string[]; ids?: string[]; side: 'ally' | 'enemy'; me?: string | null; version: string | null }) {
  return (
    <div className={`team-list ${side}`}>
      <div className="team-head">{side === 'ally' ? 'Twoja drużyna' : 'Przeciwnicy'}</div>
      {names.map((n, i) => (
        <div key={n + i} className={'team-row' + (me && n === me ? ' me' : '')}>
          <Portrait id={ids?.[i]} name={n} version={version} side={side} />
          <span className="team-name">{n}</span>
          {me && n === me && <span className="you-tag">TY</span>}
        </div>
      ))}
    </div>
  );
}

const LiveCard = memo(function LiveCard({ game, advice, version }: { game: GameInfo; advice: CoachAdvice | null; version: string | null }) {
  return (
    <section className="live-card">
      <header className="live-head">
        <span className="live-pill"><i />NA ŻYWO</span>
        <span className="live-mode">{game.gameMode === 'CLASSIC' ? 'Summoner’s Rift' : game.gameMode}</span>
        <span className="live-clock num">{fmtClock(game.gameTime)}</span>
      </header>
      <div className="live-body">
        <TeamList names={game.allies} ids={game.allyIds} side="ally" me={game.activeChampion} version={version} />
        <div className="live-coach">
          <div className="card-label">Coach</div>
          {advice ? (
            <div className={`coach-call tone-${advice.tone}`}>
              <span className="tone-tag">{TONE[advice.tone]}</span>
              <div className="coach-call-text">{advice.call}</div>
              <div className="coach-call-meta num">{fmtClock(advice.gameTime)} · {(advice.latencyMs / 1000).toFixed(1)} s{advice.item ? ` · kup: ${advice.item}` : ''}</div>
              {advice.deathReview?.length ? <ul className="coach-review">{advice.deathReview.map((p, i) => <li key={i}>{p}</li>)}</ul> : null}
            </div>
          ) : <div className="muted small">Pierwsza porada pojawi się po kilkunastu sekundach gry.</div>}
        </div>
        <TeamList names={game.enemies} ids={game.enemyIds} side="enemy" version={version} />
      </div>
    </section>
  );
});

function IdleCard({ status }: { status: AppStatus }) {
  const phase = status.lcu === 'connected' ? lcuPhaseLabel(status.lcuPhase) : null;
  const coachReady = status.coach.state === 'idle' || status.coach.state === 'thinking';
  return (
    <section className="idle-card">
      <div className="idle-icon"><img src="icon.png" alt="" /></div>
      <div className="idle-text">
        <div className="idle-title">Czekam na mecz</div>
        <div className="idle-sub">{phase ? `Klient: ${phase}. ` : 'Uruchom League of Legends. '}Coach włączy się sam po starcie gry, a runy ustawi w wyborze postaci.</div>
      </div>
      <div className="idle-checks">
        <span className={status.lcu === 'connected' ? 'ok' : ''}>Klient</span>
        <span className={coachReady ? 'ok' : ''}>Coach AI</span>
        <span className={status.settings.calibration ? 'ok' : ''}>Minimapa</span>
        <span className={status.profiles.state !== 'no-key' ? 'ok' : ''}>Klucz Riot</span>
      </div>
    </section>
  );
}

export function HomeView({ status }: { status: AppStatus }) {
  return (
    <div className="home">
      {status.game ? <LiveCard game={status.game} advice={status.lastAdvice} version={status.ddragonVersion} /> : <IdleCard status={status} />}
      <LastMatchView ddragonVersion={status.ddragonVersion} />
    </div>
  );
}
