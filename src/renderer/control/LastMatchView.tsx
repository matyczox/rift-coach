import { memo, useCallback, useEffect, useMemo, useState } from 'react';
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

function ChampIcon({ name, version, size = 44 }: { name: string; version: string | null; size?: number }) {
  const src = name ? champIcon(version, champId(name)) : null;
  return <span className="mh-icon" style={{ width: size, height: size }}>{src ? <img src={src} alt={name} onError={e => { e.currentTarget.style.display = 'none'; }} /> : null}</span>;
}

export const LastMatchView = memo(function LastMatchView({ ddragonVersion }: { ddragonVersion: string | null }) {
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

  // New review pushed after a match: put it on top and show it.
  useEffect(() => {
    let unsub: (() => void) | undefined;
    try {
      unsub = rc().onReview((raw) => {
        const r = normalize(raw);
        if (!r) return;
        setItems((prev) => [r, ...(prev ?? []).filter((x) => x.id !== r.id)]);
        setSelectedId(null);
      });
    } catch { /* bridge missing: list error already shown */ }
    return () => { try { unsub?.(); } catch { /* ignore */ } };
  }, []);

  const current = items?.find((x) => x.id === selectedId) ?? items?.[0] ?? null;
  const showingNewest = !!items && current === items[0];

  return (
    <section className="section">
      <div className="section-head">
        <h2 className="section-title">Ostatnie mecze</h2>
        <div className="toolbar">
          {!showingNewest && items && items.length > 0 && (
            <button type="button" className="btn" onClick={() => setSelectedId(null)}>Pokaż najnowszy</button>
          )}
          <button type="button" className="btn" onClick={() => void refresh()} disabled={list.busy}>Odśwież</button>
        </div>
      </div>
      {list.error && <div className="error-text">{list.error}</div>}

      {items === null ? (
        <div className="empty">{list.busy ? 'Wczytywanie…' : 'Nie udało się wczytać analiz.'}</div>
      ) : !current ? (
        <div className="placeholder">Po pierwszym meczu na Summoner’s Rift pojawi się tu analiza Twojej gry: co poprawić, kluczowe momenty i wykres złota.</div>
      ) : (
        <>
          {items.length > 1 && (
            <div className="mh-list">
              {items.map((it) => (
                <button
                  type="button"
                  key={it.id}
                  className={'mh-row' + (it.win === true ? ' win' : it.win === false ? ' loss' : '') + (it.id === current.id ? ' open' : '')}
                  onClick={() => setSelectedId(it.id === items[0]?.id ? null : it.id)}
                >
                  <span className="mh-result">{it.win === null ? '—' : it.win ? 'Wygrana' : 'Przegrana'}<small>{it.durationSec ? fmtClock(it.durationSec) : ''}</small></span>
                  <ChampIcon name={it.champion} version={ddragonVersion} />
                  <span className="mh-champ">{it.champion || '—'}<small>{roleLabel(it.role)}</small></span>
                  <span className="mh-kda num">{it.kda || '—'}<small>KDA</small></span>
                  <span className="mh-cs num">{Number.isFinite(it.csPerMin) ? it.csPerMin.toFixed(1) : '—'}<small>CS/min</small></span>
                  <span className="mh-tip">{it.improve[0] ?? it.summary}</span>
                  <span className="mh-date num">{it.createdAt ? fmtDate(it.createdAt) : ''}</span>
                </button>
              ))}
            </div>
          )}
          <ReviewCard review={current} newest={showingNewest} version={ddragonVersion} />
        </>
      )}
    </section>
  );
});

function ReviewCard({ review: r, newest, version }: { review: MatchReview; newest: boolean; version: string | null }) {
  return (
    <article className={'rv-card' + (r.win === true ? ' win' : r.win === false ? ' loss' : '')}>
      <header className="rv-head">
        <ChampIcon name={r.champion} version={version} size={48} />
        <ResultBadge win={r.win} />
        <span className="rv-champ">{r.champion || 'Nieznana postać'}</span>
        {r.role && <span className="muted">{roleLabel(r.role)}</span>}
        <span className="rv-stats num">
          {r.durationSec > 0 && <span><span className="muted">Czas</span> {fmtClock(r.durationSec)}</span>}
          {r.kda && <span><span className="muted">KDA</span> {r.kda}</span>}
          {Number.isFinite(r.csPerMin) && <span><span className="muted">CS/min</span> {r.csPerMin.toFixed(1)}</span>}
        </span>
        <span className="rv-meta muted small">
          {!newest && r.createdAt ? fmtDate(r.createdAt) : newest && r.createdAt ? `najnowszy · ${fmtDate(r.createdAt)}` : ''}
        </span>
      </header>

      {r.summary && <p className="rv-summary">{r.summary}</p>}

      <div className="rv-cols">
        <div className="rv-improve">
          <div className="rv-col-title">Do poprawy</div>
          {r.improve.length ? (
            <ol>{r.improve.map((t, i) => <li key={i}>{t}</li>)}</ol>
          ) : <div className="muted small">Brak uwag.</div>}
        </div>
        <div className="rv-good">
          <div className="rv-col-title">Dobrze</div>
          {r.good.length ? (
            <ul>{r.good.map((t, i) => <li key={i}>{t}</li>)}</ul>
          ) : <div className="muted small">Brak danych.</div>}
        </div>
      </div>

      <div className="rv-lower">
        <div className="rv-block rv-chart-block">
          <div className="rv-col-title">Różnica złota</div>
          <GoldChart points={r.goldCurve} durationSec={r.durationSec} />
        </div>
        <div className="rv-block rv-moments-block">
          <div className="rv-col-title">Kluczowe momenty</div>
          {r.moments.length ? (
            <ul className="rv-moments">
              {r.moments.map((m, i) => (
                <li key={i} className={'rv-m k-' + m.kind}>
                  <span className="rv-m-t num">{fmtClock(m.t)}</span>
                  <span className="rv-m-icon" aria-label={KIND_LABEL[m.kind]} title={KIND_LABEL[m.kind]}>{KIND_ICON[m.kind]}</span>
                  <span className="rv-m-text">{m.text}</span>
                </li>
              ))}
            </ul>
          ) : <div className="muted small">Brak zapisanych momentów.</div>}
        </div>
      </div>

      {r.model && <div className="rv-foot muted small">Analiza: <span className="mono">{r.model}</span></div>}
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
