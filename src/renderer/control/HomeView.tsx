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

const LiveCard = memo(function LiveCard({ game, advice, version, coach }: { game: GameInfo; advice: CoachAdvice | null; version: string | null; coach: AppStatus['coach'] }) {
  return (
    <section className="live-card">
      <header className="live-head">
        <span className="live-pill"><i />NA ŻYWO</span>
        <span className="live-mode">{game.gameMode === 'CLASSIC' ? 'Summoner’s Rift' : game.gameMode}</span>
        <span className="live-clock num">{fmtClock(game.gameTime)}</span>
      </header>
      <div className="live-body">
        <TeamList names={game.allies} ids={game.allyIds} side="ally" me={game.activeChampion} version={version} />
        <div className={'live-coach' + (coach.state === 'thinking' ? ' thinking' : '')}>
          <div className="coach-head"><CoachMark size={26} active={coach.state === 'thinking'} /><span>Coach</span>{coach.state === 'thinking' && <span className="dots"><i /><i /><i /></span>}</div>
          {advice ? (
            <div className={`coach-call tone-${advice.tone}`}>
              <span className="tone-tag">{TONE[advice.tone]}</span>
              <div className="coach-call-text">{advice.call}</div>
              <div className="coach-call-meta num">{fmtClock(advice.gameTime)}{advice.item ? ` · kup: ${advice.item}` : ''}</div>
              {advice.deathReview?.length ? <ul className="coach-review">{advice.deathReview.map((p, i) => <li key={i}>{p}</li>)}</ul> : null}
            </div>
          ) : <div className="coach-wait">Pierwsza porada pojawi się po kilkunastu sekundach gry.</div>}
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

/** Coach glyph: a spark inside a ring (pulses while the coach is thinking). */
function CoachMark({ size = 40, active = false }: { size?: number; active?: boolean }) {
  return (
    <span className={'coach-mark' + (active ? ' active' : '')} style={{ width: size, height: size }}>
      <svg viewBox="0 0 40 40" width={size} height={size}>
        <defs><linearGradient id="cm" x1="0" y1="1" x2="1" y2="0"><stop offset="0" stopColor="#22c7bd" /><stop offset="1" stopColor="#e8b65a" /></linearGradient></defs>
        <circle cx="20" cy="20" r="18" fill="none" stroke="url(#cm)" strokeWidth="2.5" />
        <path d="M22 7 L12 22 H19 L17 33 L28 17 H21 Z" fill="url(#cm)" />
      </svg>
    </span>
  );
}

/** Polish plural: 1 porada, 2–4 porady, 5+ porad (12–14 porad). */
const plural = (n: number, one: string, few: string, many: string) => (n === 1 ? one : n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 12 || n % 100 > 14) ? few : many);

const PROVIDER = (m: string) => (m.startsWith('gpt') ? 'Codex' : 'Claude');

function CoachCard({ status }: { status: AppStatus }) {
  const c = status.coach, s = status.settings.coach;
  const state = c.state === 'thinking' ? 'Myśli…' : c.state === 'idle' ? 'Gotowy' : c.state === 'off' ? (status.game && status.game.mapNumber !== 11 && s.enabled ? 'Tylko na Summoner’s Rift' : 'Wyłączony') : c.state === 'not-logged-in' ? 'Niezalogowany' : 'Błąd';
  const tone = c.state === 'idle' || c.state === 'thinking' ? 'ok' : c.state === 'off' ? 'off' : 'bad';
  const rows: Array<[string, string]> = [['W grze', s.liveModel], ['Po śmierci i meczu', s.reviewModel], ['Patch i matchup', s.researchModel], ['Runy', s.draftModel]];
  return (
    <div className={'side-card coach-card' + (c.state === 'thinking' ? ' thinking' : '')}>
      <div className="coach-top">
        <CoachMark active={c.state === 'thinking'} />
        <div className="coach-top-text">
          <div className="side-title">Twój coach</div>
          <div className={'coach-state ' + tone}><i />{state}</div>
          {c.lastLatencyMs != null && tone === 'ok' ? <div className="coach-latency">odpowiedź w {(c.lastLatencyMs / 1000).toFixed(1).replace('.', ',')} s</div> : null}
        </div>
        {c.calls > 0 && <div className="coach-calls num"><b>{c.calls}</b><small>{plural(c.calls, 'porada', 'porady', 'porad')}</small></div>}
      </div>
      {c.lastError && tone === 'bad' && <div className="error-text small">{c.lastError}</div>}
      <div className="coach-models">
        {rows.map(([l, m]) => (
          <div key={l} className="coach-model"><span>{l}</span><b>{modelName(m)}{m === 'gpt-6.1-terra' ? <em> fast</em> : null}</b><small>{PROVIDER(m)}</small></div>
        ))}
      </div>
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

const TIER_PL: Record<string, string> = { IRON: 'Żelazo', BRONZE: 'Brąz', SILVER: 'Srebro', GOLD: 'Złoto', PLATINUM: 'Platyna', EMERALD: 'Szmaragd', DIAMOND: 'Diament', MASTER: 'Mistrz', GRANDMASTER: 'Arcymistrz', CHALLENGER: 'Pretendent' };
const ROMAN: Record<string, string> = { I: '1', II: '2', III: '3', IV: '4' };

function RanksCard({ me }: { me: AppStatus['me'] }) {
  const rows: Array<['solo' | 'flex', string]> = [['solo', 'Ranked Solo'], ['flex', 'Ranked Flex']];
  return (
    <div className="side-card ranks">
      {rows.map(([q, label]) => {
        const r = me?.ranks.find(x => x.queue === q);
        const tier = r?.tier.toLowerCase() ?? 'none';
        const apex = r && ['MASTER', 'GRANDMASTER', 'CHALLENGER'].includes(r.tier);
        return (
          <div key={q} className="rank-row">
            <span className={`rank-gem t-${tier}`} />
            <span className="rank-q">{label}</span>
            <span className={`rank-v t-${tier}`}>{r ? `${TIER_PL[r.tier] ?? r.tier}${apex ? '' : ' ' + (ROMAN[r.division] ?? r.division)} · ${r.lp} LP` : 'brak rangi'}</span>
          </div>
        );
      })}
    </div>
  );
}

function ProfileHeader({ me, version, splash }: { me: AppStatus['me']; version: string | null; splash: string | null }) {
  const icon = me && version ? `https://ddragon.leagueoflegends.com/cdn/${version}/img/profileicon/${me.iconId}.png` : null;
  return (
    <header className="profile">
      {splash && <div className="profile-splash" style={{ backgroundImage: `url(https://ddragon.leagueoflegends.com/cdn/img/champion/splash/${champId(splash)}_0.jpg)` }} />}
      <div className="profile-inner">
        <div className="profile-avatar">
          {icon ? <img src={icon} alt="" /> : <img src="icon.png" alt="" />}
          {me ? <span className="profile-level num">{me.level}</span> : null}
        </div>
        <div>
          <div className="profile-name">{me ? me.gameName : 'Rift Coach'}{me?.tagLine ? <span className="profile-tag"> #{me.tagLine}</span> : null}</div>
          {!me && <div className="profile-sub">Zaloguj się w kliencie League of Legends</div>}
        </div>
      </div>
    </header>
  );
}

export function HomeView({ status }: { status: AppStatus }) {
  const [items, setItems] = useState<MatchReview[]>([]);
  const onItems = useCallback((x: MatchReview[]) => setItems(x), []);
  const splash = status.game?.activeChampion ?? items[0]?.champion ?? status.me?.topChampion ?? null;
  return (
    <div className="home-wrap">
      <ProfileHeader me={status.me} version={status.ddragonVersion} splash={splash} />
      <div className="home">
        <aside className="home-side">
          <RanksCard me={status.me} />
          <ReadyCard status={status} />
          <CoachCard status={status} />
          <ChampsCard items={items} version={status.ddragonVersion} />
        </aside>
        <div className="home-main">
          {status.game ? <LiveCard game={status.game} advice={status.lastAdvice} version={status.ddragonVersion} coach={status.coach} /> : <IdleCard status={status} />}
          <LastMatchView ddragonVersion={status.ddragonVersion} onItems={onItems} />
        </div>
      </div>
    </div>
  );
}
