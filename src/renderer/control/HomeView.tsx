import { memo, useCallback, useState } from 'react';
import type { AppStatus, GameInfo, MatchReview } from '../../shared/types';
import type { CoachAdvice } from '../../shared/intel';
import { champIcon, champId, fmtClock, lcuPhaseLabel } from './util';
import { Portrait } from './StatusBar';
import { LastMatchView, parseKda } from './LastMatchView';

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
  return (
    <section className="idle-card">
      <div className="idle-icon"><img src="icon.png" alt="" /></div>
      <div className="idle-text">
        <div className="idle-title">Czekam na mecz</div>
        <div className="idle-sub">{phase ? `Klient: ${phase}. ` : 'Uruchom League of Legends. '}Coach włączy się sam po starcie gry, a runy ustawi w wyborze postaci.</div>
      </div>
    </section>
  );
}

const modelName = (m: string) => m.replace(/^gpt-[\d.]+-/, '').replace(/^claude-/, '').replace(/-(\d+)-(\d+)$/, ' $1.$2').replace(/^./, c => c.toUpperCase());

function ReadyCard({ status }: { status: AppStatus }) {
  const rows: Array<[string, boolean, string]> = [
    ['Klient LoL', status.lcu === 'connected', status.lcu === 'connected' ? 'połączony' : 'nie działa'],
    ['Coach AI', status.coach.state === 'idle' || status.coach.state === 'thinking', status.coach.state === 'not-logged-in' ? 'niezalogowany' : status.coach.state === 'off' ? 'wyłączony' : status.coach.state === 'error' ? 'błąd' : 'gotowy'],
    ['Minimapa', Boolean(status.settings.calibration), status.settings.calibration ? 'skalibrowana' : 'brak'],
    ['Klucz Riot', status.profiles.state !== 'no-key', status.profiles.state === 'no-key' ? 'brak' : 'jest'],
    ['Auto runy', status.settings.autoRunes, status.settings.autoRunes ? 'włączone' : 'wyłączone'],
  ];
  return (
    <div className="side-card">
      <div className="side-title">Gotowość</div>
      {rows.map(([l, ok, v]) => (
        <div key={l} className="side-row"><span className={'side-dot' + (ok ? ' ok' : '')} /><span className="side-label">{l}</span><span className={'side-value' + (ok ? '' : ' dim')}>{v}</span></div>
      ))}
    </div>
  );
}

function ModelsCard({ status }: { status: AppStatus }) {
  const c = status.settings.coach;
  const rows: Array<[string, string]> = [['W grze', c.liveModel], ['Po śmierci i meczu', c.reviewModel], ['Patch i matchup', c.researchModel], ['Runy i draft', c.draftModel]];
  return (
    <div className="side-card">
      <div className="side-title">Modele coacha</div>
      {rows.map(([l, m]) => (
        <div key={l} className="side-row"><span className="side-label">{l}</span><span className={'model-chip ' + (m.startsWith('gpt') ? 'gpt' : 'claude')}>{modelName(m)}</span></div>
      ))}
    </div>
  );
}

function ChampsCard({ items, version }: { items: MatchReview[]; version: string | null }) {
  const by = new Map<string, { n: number; w: number; kda: number; cs: number }>();
  for (const r of items) {
    if (!r.champion) continue;
    const e = by.get(r.champion) ?? { n: 0, w: 0, kda: 0, cs: 0 };
    e.n++; if (r.win) e.w++;
    e.kda += parseKda(r.kda)?.ratio ?? 0; e.cs += Number.isFinite(r.csPerMin) ? r.csPerMin : 0;
    by.set(r.champion, e);
  }
  const list = [...by.entries()].sort((a, b) => b[1].n - a[1].n).slice(0, 5);
  return (
    <div className="side-card">
      <div className="side-title">Twoje postacie</div>
      {list.length === 0 ? <div className="muted small">Pojawią się po pierwszych analizowanych meczach.</div> : list.map(([name, e]) => {
        const wr = Math.round((e.w / e.n) * 100);
        const src = champIcon(version, champId(name));
        return (
          <div key={name} className="champ-row">
            <span className="champ-ico">{src && <img src={src} alt="" />}</span>
            <span className="champ-name">{name}<small className="num">{(e.kda / e.n).toFixed(1)} KDA · {(e.cs / e.n).toFixed(1)} CS/min</small></span>
            <span className={'champ-wr num ' + (wr >= 50 ? 'good' : 'bad')}>{wr}% WR<small>{e.w}W - {e.n - e.w}L</small></span>
          </div>
        );
      })}
    </div>
  );
}

export function HomeView({ status }: { status: AppStatus }) {
  const [items, setItems] = useState<MatchReview[]>([]);
  const onItems = useCallback((x: MatchReview[]) => setItems(x), []);
  return (
    <div className="home">
      <aside className="home-side">
        <ReadyCard status={status} />
        <ModelsCard status={status} />
        <ChampsCard items={items} version={status.ddragonVersion} />
      </aside>
      <div className="home-main">
        {status.game ? <LiveCard game={status.game} advice={status.lastAdvice} version={status.ddragonVersion} /> : <IdleCard status={status} />}
        <LastMatchView ddragonVersion={status.ddragonVersion} onItems={onItems} />
      </div>
    </div>
  );
}
