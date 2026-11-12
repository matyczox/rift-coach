import { memo } from 'react';
import type { CoachAdvice, CoachTone } from '../../shared/intel';
import { fmtClock } from './util';

const TONE_LABEL: Record<CoachTone, string> = {
  go: 'rób',
  wait: 'czekaj',
  danger: 'niebezpiecznie',
  farm: 'farm',
};

/** "Ostatnia porada" — the last coach call, with tone edge and death review. */
export const AdviceCard = memo(function AdviceCard({ advice }: { advice: CoachAdvice | null }) {
  return (
    <div className="cal-card advice-card">
      <div className="cal-card-title">Ostatnia porada</div>
      {!advice ? (
        <div className="muted small">Brak porad.</div>
      ) : (
        <div className={`advice tone-${advice.tone}`}>
          <div className="advice-call">{advice.call}</div>
          <div className="advice-meta muted small num">
            {TONE_LABEL[advice.tone]} · gra {fmtClock(advice.gameTime)} ·{' '}
            {new Date(advice.createdAt).toLocaleTimeString('pl-PL', { hour: '2-digit', minute: '2-digit', second: '2-digit' })}
            {' '}· {(advice.latencyMs / 1000).toFixed(1)} s
          </div>
          {advice.item && <div className="small"><span className="muted">Przedmiot: </span>{advice.item}</div>}
          {advice.deathReview && advice.deathReview.length > 0 && (
            <ul className="advice-review">
              {advice.deathReview.map((p, i) => <li key={i}>{p}</li>)}
            </ul>
          )}
        </div>
      )}
    </div>
  );
});
