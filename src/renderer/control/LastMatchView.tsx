import { memo, useCallback, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import type { MatchReview } from '../../shared/types';
import { champIcon, champId, fmtClock, fmtDate, rc, useAction } from './util';

type Moment = MatchReview['moments'][number];
type GoldPoint = MatchReview['goldCurve'][number];

const num = (v: unknown, d = 0): number => (typeof v === 'number' && Number.isFinite(v) ? v : d);
const str = (v: unknown): string => (typeof v === 'string' ? v.trim() : '');
const strList = (v: unknown): string[] => (Array.isArray(v) ? v.map(str).filter(Boolean) : []);
const KINDS: Moment['kind'][] = ['death', 'kill', 'objective', 'advice'];

/** Defensive copy: the review comes from a model, any field may be missing or malformed. */
function normalize(r: Partial<MatchReview> | null | undefined): MatchReview | null {
  if (!r || typeof r !== 'object' || !str(r.id)) return null;
  const moments = (Array.isArray(r.moments) ? r.moments : [])
    .filter((m): m is Moment => !!m && typeof m === 'object' && str(m.text) !== '')
    .map((m) => ({ t: Math.max(0, num(m.t)), kind: KINDS.includes(m.kind) ? m.kind : 'advice', text: str(m.text) }))
    .sort((a, b) => a.t - b.t);
  const goldCurve = (Array.isArray(r.goldCurve) ? r.goldCurve : [])
    .filter((p): p is GoldPoint => !!p && Number.isFinite(p.t) && Number.isFinite(p.diff))
    .map((p) => ({ t: Math.max(0, p.t), diff: p.diff }))
    .sort((a, b) => a.t - b.t);
  return {
    id: str(r.id),
    createdAt: num(r.createdAt, 0),
    champion: str(r.champion),
    role: str(r.role),
    win: typeof r.win === 'boolean' ? r.win : null,
    durationSec: Math.max(0, num(r.durationSec)),
    kda: str(r.kda),
    csPerMin: num(r.csPerMin, NaN),
    summary: str(r.summary),
    good: strList(r.good),
    improve: strList(r.improve),
    moments,
    goldCurve,
    model: str(r.model),
  };
}

const ROLE_LABEL: Record<string, string> = {
  top: 'Top', jungle: 'Dżungla', jg: 'Dżungla', mid: 'Mid', middle: 'Mid',
  bottom: 'ADC', bot: 'ADC', adc: 'ADC', carry: 'ADC', utility: 'Support', support: 'Support', sup: 'Support',
};
const roleLabel = (r: string) => (r ? ROLE_LABEL[r.toLowerCase()] ?? r : '');

function ResultBadge({ win }: { win: boolean | null }) {
  if (win === null) return <span className="rv-badge rv-unknown">—</span>;
  return <span className={'rv-badge ' + (win ? 'rv-win' : 'rv-loss')}>{win ? 'W' : 'L'}</span>;
}

function ChampIcon({ name, version, size = 44, tone }: { name: string; version: string | null; size?: number; tone?: 'win' | 'loss' | null }) {
  const src = name ? champIcon(version, champId(name)) : null;
  return (
    <span className={'mh-icon' + (tone ? ` ${tone}` : '')} style={{ width: size, height: size }}>
      {src ? <img src={src} alt={name} onError={e => { e.currentTarget.style.display = 'none'; }} /> : null}
    </span>
  );
}

/** "5/3/7" -> { k, d, a, ratio } */
export function parseKda(kda: string) {
  const m = /(\d+)\s*\/\s*(\d+)\s*\/\s*(\d+)/.exec(kda);
  if (!m) return null;
  const k = +m[1]!, d = +m[2]!, a = +m[3]!;
  return { k, d, a, ratio: (k + a) / Math.max(1, d) };
}

/** Small achievement pills, like Blitz's "First Blood" / "2-Piece". */
function badgesOf(r: MatchReview): string[] {
  const out: string[] = [];
  const k = parseKda(r.kda);
  if (k && k.d === 0) out.push('Bez śmierci');
  else if (k && k.ratio >= 4) out.push('Czysta gra');
  if (Number.isFinite(r.csPerMin) && r.csPerMin >= 8) out.push('Świetna farma');
  else if (Number.isFinite(r.csPerMin) && r.csPerMin >= 7) out.push('Dobra farma');
  const last = r.goldCurve[r.goldCurve.length - 1];
  if (last && last.diff >= 5000) out.push('Dominacja złotem');
  if (r.moments.some(m => m.kind === 'objective')) out.push('Obiektywy');
  return out.slice(0, 3);
}

const tone = (w: boolean | null) => (w === true ? 'win' : w === false ? 'loss' : null);
const resultText = (w: boolean | null) => (w === true ? 'Wygrana' : w === false ? 'Przegrana' : 'Mecz');

export const LastMatchView = memo(function LastMatchView({ ddragonVersion, onItems }: { ddragonVersion: string | null; onItems?: (items: MatchReview[]) => void }) {
  const [items, setItems] = useState<MatchReview[] | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const list = useAction();

  const refresh = useCallback(async () => {
    const r = await list.run(() => rc().listReviews());
    if (r.ok) {
      const norm = (Array.isArray(r.value) ? r.value : []).map(normalize).filter((x): x is MatchReview => !!x);
      setItems(norm.sort((a, b) => b.createdAt - a.createdAt));
    }
  }, [list.run]);

  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => { if (items) onItems?.(items); }, [items, onItems]);

  // New review pushed after a match: put it on top and open it.
  useEffect(() => {
    let unsub: (() => void) | undefined;
    try {
      unsub = rc().onReview((raw) => {
        const r = normalize(raw);
        if (!r) return;
        setItems((prev) => [r, ...(prev ?? []).filter((x) => x.id !== r.id)]);
        setSelectedId(r.id);
      });
    } catch { /* bridge missing: list error already shown */ }
    return () => { try { unsub?.(); } catch { /* ignore */ } };
  }, []);

  const current = items?.find((x) => x.id === selectedId) ?? null;

  if (current) {
    return <ReviewCard review={current} version={ddragonVersion} onBack={() => setSelectedId(null)} />;
  }

  return (
    <section className="section">
      <div className="section-head">
        <h2 className="section-title">Ostatnie mecze</h2>
        <button type="button" className="btn ghost small" onClick={() => void refresh()} disabled={list.busy}>Odśwież</button>
      </div>
      {list.error && <div className="error-text">{list.error}</div>}
      {items === null ? (
        <div className="empty">{list.busy ? 'Wczytywanie…' : 'Nie udało się wczytać analiz.'}</div>
      ) : items.length === 0 ? (
        <div className="placeholder">Po pierwszym meczu na Summoner’s Rift pojawi się tu jego karta: wynik, KDA, farma, a po kliknięciu analiza coacha z osią czasu i wykresem złota.</div>
      ) : (
        <div className="mh-list">
          {items.map((it) => {
            const k = parseKda(it.kda);
            const t = tone(it.win);
            const badges = badgesOf(it);
            return (
              <button type="button" key={it.id} className={'mcard' + (t ? ` ${t}` : '')} onClick={() => setSelectedId(it.id)}>
                <ChampIcon name={it.champion} version={ddragonVersion} size={52} tone={t} />
                <div className="mcard-main">
                  <div className="mcard-head">
                    <span className="mcard-result">{resultText(it.win)}</span>
                    <span className="mcard-meta">{[roleLabel(it.role), it.durationSec ? fmtClock(it.durationSec) : '', it.createdAt ? fmtDate(it.createdAt) : ''].filter(Boolean).join(' • ')}</span>
                  </div>
                  <div className="mcard-stats">
                    <div className="stat"><b className="num">{k ? k.ratio.toFixed(1) : '—'} KDA</b><small className="num">{it.kda || '—'}</small></div>
                    <div className="stat"><b className="num">{Number.isFinite(it.csPerMin) ? it.csPerMin.toFixed(1) : '—'} CS/min</b><small>{it.champion}</small></div>
                    <div className="mcard-tip">{it.improve[0] ?? it.summary}</div>
                  </div>
                  {badges.length > 0 && <div className="pills">{badges.map(b => <span key={b} className="pill">{b}</span>)}</div>}
                </div>
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
});

/** Blitz-style score ring: 0–100. */
function Gauge({ value, label, sub }: { value: number; label: string; sub: string }) {
  const v = Math.max(0, Math.min(100, Math.round(value)));
  const R = 34, C = 2 * Math.PI * R, arc = C * 0.75;
  const col = v >= 70 ? 'var(--win)' : v >= 45 ? 'var(--warn)' : 'var(--loss)';
  return (
    <div className="gauge">
      <svg viewBox="0 0 90 90" width="96" height="96">
        <circle cx="45" cy="45" r={R} fill="none" stroke="var(--border-strong)" strokeWidth="6" strokeDasharray={`${arc} ${C}`} transform="rotate(135 45 45)" strokeLinecap="round" />
        <circle cx="45" cy="45" r={R} fill="none" stroke={col} strokeWidth="6" strokeDasharray={`${(arc * v) / 100} ${C}`} transform="rotate(135 45 45)" strokeLinecap="round" />
        <text x="45" y="52" textAnchor="middle" className="gauge-num">{v}</text>
      </svg>
      <div className="gauge-label">{label}</div>
      <div className="gauge-sub" style={{ color: col }}>{sub}</div>
    </div>
  );
}
const verdict = (v: number) => (v >= 80 ? 'Świetnie' : v >= 60 ? 'Dobrze' : v >= 40 ? 'Przeciętnie' : 'Do poprawy');

type ReviewTab = 'overview' | 'timeline' | 'gold';

function ReviewCard({ review: r, version, onBack }: { review: MatchReview; version: string | null; onBack: () => void }) {
  const [tab, setTab] = useState<ReviewTab>('overview');
  const k = parseKda(r.kda);
  const combat = k ? Math.min(100, k.ratio * 22 - k.d * 2 + 20) : 0;
  const farm = Number.isFinite(r.csPerMin) ? Math.min(100, (r.csPerMin / 9) * 100) : 0;
  const last = r.goldCurve[r.goldCurve.length - 1];
  const gold = last ? Math.max(0, Math.min(100, 50 + last.diff / 200)) : 50;
  const t = tone(r.win);
  const TABS: Array<[ReviewTab, string, ReactNode]> = [['overview', 'Przegląd', null], ['timeline', 'Oś czasu', r.moments.length || null], ['gold', 'Złoto', null]];
  return (
    <article className={'review' + (t ? ` ${t}` : '')}>
      <header className="review-head">
        <button type="button" className="btn ghost small" onClick={onBack}>← Wróć</button>
        <ChampIcon name={r.champion} version={version} size={56} tone={t} />
        <div>
          <div className="review-result">{resultText(r.win)}</div>
          <div className="mcard-meta">{[r.champion, roleLabel(r.role), r.durationSec ? `${fmtClock(r.durationSec)} min` : '', r.createdAt ? fmtDate(r.createdAt) : ''].filter(Boolean).join(' • ')}</div>
        </div>
        <div className="review-kda num"><b>{r.kda || '—'}</b><small>{Number.isFinite(r.csPerMin) ? `${r.csPerMin.toFixed(1)} CS/min` : ''}</small></div>
      </header>

      <div className="gauges">
        <Gauge value={combat} label="Walka" sub={verdict(combat)} />
        <Gauge value={farm} label="Farma" sub={verdict(farm)} />
        <Gauge value={gold} label="Złoto drużyny" sub={last ? fmtGold(last.diff) : '—'} />
        <div className="review-summary">
          <div className="card-label">Coach</div>
          <p>{r.summary || 'Brak podsumowania.'}</p>
          {r.model && <div className="muted small">Analiza: {r.model}</div>}
        </div>
      </div>

      <div className="subtabs">
        {TABS.map(([id, label, n]) => (
          <button key={id} type="button" className={'subtab' + (tab === id ? ' active' : '')} onClick={() => setTab(id)}>{label}{n ? <span className="tab-badge">{n}</span> : null}</button>
        ))}
      </div>

      {tab === 'overview' && (
        <div className="rv-cols">
          <div className="panel rv-improve">
            <div className="card-label warn">Do poprawy</div>
            {r.improve.length ? <ol>{r.improve.map((x, i) => <li key={i}>{x}</li>)}</ol> : <div className="muted small">Brak uwag.</div>}
          </div>
          <div className="panel rv-good">
            <div className="card-label ok">Dobrze</div>
            {r.good.length ? <ul>{r.good.map((x, i) => <li key={i}>{x}</li>)}</ul> : <div className="muted small">Brak danych.</div>}
          </div>
        </div>
      )}
      {tab === 'timeline' && (
        <div className="panel">
          {r.moments.length ? (
            <ul className="timeline">
              {r.moments.map((m, i) => (
                <li key={i} className={'tl k-' + m.kind}>
                  <span className="tl-t num">{fmtClock(m.t)} min</span>
                  <span className="tl-text"><span className="tl-icon">{KIND_ICON[m.kind]}</span>{m.text}</span>
                </li>
              ))}
            </ul>
          ) : <div className="muted small">Brak zapisanych momentów.</div>}
        </div>
      )}
      {tab === 'gold' && (
        <div className="panel"><GoldChart points={r.goldCurve} durationSec={r.durationSec} /></div>
      )}
    </article>
  );
}

const KIND_ICON: Record<Moment['kind'], string> = { death: '✕', kill: '⚔', objective: '◆', advice: '•' };
const KIND_LABEL: Record<Moment['kind'], string> = { death: 'Śmierć', kill: 'Zabójstwo', objective: 'Cel', advice: 'Rada' };

const fmtGold = (v: number) => {
  const a = Math.abs(v);
  const s = a >= 1000 ? `${(a / 1000).toFixed(a >= 10000 ? 0 : 1)}k` : String(Math.round(a));
  return (v > 0 ? '+' : v < 0 ? '−' : '') + s;
};

function GoldChart({ points, durationSec }: { points: GoldPoint[]; durationSec: number }) {
  const W = 560, H = 150, L = 40, R = 8, T = 8, B = 20;
  const geo = useMemo(() => {
    const first = points[0], lastP = points[points.length - 1];
    if (points.length < 2 || !first || !lastP) return null;
    const t0 = 0;
    const t1 = Math.max(lastP.t, durationSec, 60);
    const maxAbs = Math.max(500, ...points.map((p) => Math.abs(p.diff)));
    const x = (t: number) => L + ((t - t0) / (t1 - t0)) * (W - L - R);
    const y = (d: number) => T + ((maxAbs - d) / (2 * maxAbs)) * (H - T - B);
    const line = points.map((p, i) => `${i ? 'L' : 'M'}${x(p.t).toFixed(1)},${y(p.diff).toFixed(1)}`).join(' ');
    const zy = y(0);
    const area = `${line} L${x(lastP.t).toFixed(1)},${zy.toFixed(1)} L${x(first.t).toFixed(1)},${zy.toFixed(1)} Z`;
    const totalMin = t1 / 60;
    const step = totalMin <= 12 ? 2 : totalMin <= 30 ? 5 : 10;
    const ticks: number[] = [];
    for (let m = 0; m <= totalMin + 0.001; m += step) ticks.push(m);
    return { x, y, zy, line, area, ticks, maxAbs, last: lastP };
  }, [points, durationSec]);

  if (!geo) return <div className="rv-chart-empty muted small">Brak danych o złocie.</div>;
  const { x, y, zy, line, area, ticks, maxAbs, last } = geo;

  return (
    <svg className="rv-chart" viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`Różnica złota, na koniec ${fmtGold(last.diff)}`}>
      <defs>
        <clipPath id="rv-above"><rect x={0} y={0} width={W} height={zy} /></clipPath>
        <clipPath id="rv-below"><rect x={0} y={zy} width={W} height={H - zy} /></clipPath>
      </defs>
      {ticks.map((m) => (
        <g key={m}>
          <line x1={x(m * 60)} x2={x(m * 60)} y1={T} y2={H - B} className="rv-grid" />
          <text x={x(m * 60)} y={H - 6} className="rv-axis" textAnchor="middle">{m}</text>
        </g>
      ))}
      <text x={L - 6} y={y(maxAbs) + 4} className="rv-axis" textAnchor="end">{fmtGold(maxAbs)}</text>
      <text x={L - 6} y={zy + 4} className="rv-axis" textAnchor="end">0</text>
      <text x={L - 6} y={y(-maxAbs) + 4} className="rv-axis" textAnchor="end">{fmtGold(-maxAbs)}</text>
      <text x={W - R} y={H - 6} className="rv-axis" textAnchor="end">min</text>

      <path d={area} className="rv-area-pos" clipPath="url(#rv-above)" />
      <path d={area} className="rv-area-neg" clipPath="url(#rv-below)" />
      <line x1={L} x2={W - R} y1={zy} y2={zy} className="rv-zero" />
      <path d={line} className="rv-line-pos" clipPath="url(#rv-above)" />
      <path d={line} className="rv-line-neg" clipPath="url(#rv-below)" />
      <circle cx={x(last.t)} cy={y(last.diff)} r={3} className={last.diff >= 0 ? 'rv-dot-pos' : 'rv-dot-neg'} />
    </svg>
  );
}
