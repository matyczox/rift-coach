import { memo, useEffect, useMemo, useRef, useState } from 'react';
import type { Settings } from '../../shared/types';
import { clamp, rc, useAction } from './util';

type Draft = Pick<Settings, 'autoRecord' | 'autoRunes' | 'launchAtLogin' | 'minimapFps' | 'recordFps' | 'sceneEverySec' | 'overlayVisible'> & {
  hotkeys: Settings['hotkeys'];
  coach: Settings['coach'];
  riot: Settings['riot'];
};

const LIMITS = {
  minimapFps: [5, 15],
  recordFps: [2, 10],
  sceneEverySec: [2, 30],
} as const;
type NumKey = keyof typeof LIMITS;
const PERIOD_LIMITS = [10, 120] as const;

const LIVE_MODELS = ['haiku', 'sonnet', 'gpt-6.1-terra', 'gpt-6.1-sol'];
const REVIEW_MODELS = ['sonnet', 'opus', 'haiku', 'claude-opus-5-5'];
const RESEARCH_MODELS = ['claude-opus-5-5', 'gpt-6.1-terra', 'claude-sonnet-5-5'];
const DRAFT_MODELS = ['gpt-6.1-terra', 'claude-sonnet-5-5', 'claude-opus-5-5'];
const PLATFORMS: { id: string; label: string }[] = [
  { id: 'eun1', label: 'EUNE (eun1)' },
  { id: 'euw1', label: 'EUW (euw1)' },
  { id: 'na1', label: 'NA (na1)' },
  { id: 'kr', label: 'KR (kr)' },
  { id: 'tr1', label: 'TR (tr1)' },
  { id: 'ru', label: 'RU (ru)' },
  { id: 'br1', label: 'BR (br1)' },
  { id: 'la1', label: 'LAN (la1)' },
  { id: 'la2', label: 'LAS (la2)' },
  { id: 'oc1', label: 'OCE (oc1)' },
  { id: 'jp1', label: 'JP (jp1)' },
];

function toDraft(s: Settings): Draft {
  return {
    autoRecord: s.autoRecord,
    autoRunes: s.autoRunes,
    launchAtLogin: s.launchAtLogin,
    minimapFps: s.minimapFps,
    recordFps: s.recordFps,
    sceneEverySec: s.sceneEverySec,
    overlayVisible: s.overlayVisible,
    hotkeys: { ...s.hotkeys },
    coach: { ...s.coach },
    riot: { ...s.riot },
  };
}

const sameDraft = (a: Draft, b: Draft) =>
  a.autoRecord === b.autoRecord && a.autoRunes === b.autoRunes && a.launchAtLogin === b.launchAtLogin && a.minimapFps === b.minimapFps && a.recordFps === b.recordFps
  && a.sceneEverySec === b.sceneEverySec && a.overlayVisible === b.overlayVisible
  && a.hotkeys.toggleOverlay === b.hotkeys.toggleOverlay && a.hotkeys.toggleRecording === b.hotkeys.toggleRecording
  && a.hotkeys.enemyFlash === b.hotkeys.enemyFlash
  && a.coach.enabled === b.coach.enabled && a.coach.liveModel === b.coach.liveModel
  && a.coach.reviewModel === b.coach.reviewModel && a.coach.periodSec === b.coach.periodSec
  && a.coach.researchModel === b.coach.researchModel && a.coach.draftModel === b.coach.draftModel
  && a.riot.apiKey === b.riot.apiKey && a.riot.platform === b.riot.platform;

const MODIFIERS = new Set([
  'Command', 'Cmd', 'Control', 'Ctrl', 'CommandOrControl', 'CmdOrCtrl', 'Alt', 'Option', 'AltGr', 'Shift', 'Super', 'Meta',
]);
const KEY_RE = /^([0-9A-Z]|F([1-9]|1[0-9]|2[0-4])|Plus|Space|Tab|Capslock|Numlock|Scrolllock|Backspace|Delete|Insert|Return|Enter|Up|Down|Left|Right|Home|End|PageUp|PageDown|Escape|Esc|PrintScreen|VolumeUp|VolumeDown|VolumeMute|MediaNextTrack|MediaPreviousTrack|MediaStop|MediaPlayPause|num[0-9]|numdec|numadd|numsub|nummult|numdiv|[)!@#$%^&*(:;=<,_.>?/~`{}|\\"'[\]-])$/i;

/** Light validation of an Electron accelerator string. Empty = disabled. */
function acceleratorError(v: string): string | null {
  const s = v.trim();
  if (!s) return null;
  const parts = s.split('+');
  // "Alt+Plus" style is fine; a trailing literal "+" yields an empty last part.
  if (parts.some((p) => p === '')) return 'Użyj „Plus” zamiast znaku +.';
  const key = parts[parts.length - 1] ?? '';
  const mods = parts.slice(0, -1);
  if (MODIFIERS.has(key)) return 'Brak klawisza (same modyfikatory).';
  if (mods.some((m) => !MODIFIERS.has(m))) return 'Nieznany modyfikator.';
  if (!KEY_RE.test(key)) return 'Nieznany klawisz.';
  return null;
}

/** Select with known options plus "inny…" that switches to free text. */
function ModelPicker({ id, value, options, onChange }: { id: string; value: string; options: string[]; onChange: (v: string) => void }) {
  const known = options.includes(value);
  const [custom, setCustom] = useState(!known);
  useEffect(() => { if (!options.includes(value) && value !== '') setCustom(true); }, [value, options]);

  return (
    <div className="field-ctl">
      <select
        id={id}
        value={custom ? '__custom' : value}
        onChange={(e) => {
          const v = e.target.value;
          if (v === '__custom') { setCustom(true); return; }
          setCustom(false);
          onChange(v);
        }}
      >
        {options.map((o) => <option key={o} value={o}>{o}</option>)}
        <option value="__custom">inny…</option>
      </select>
      {custom && (
        <input
          type="text"
          className="text-input mono model-input"
          value={value}
          placeholder="np. claude-sonnet-4-5"
          spellCheck={false}
          onChange={(e) => onChange(e.target.value)}
          aria-label="Nazwa modelu"
        />
      )}
    </div>
  );
}

export const SettingsView = memo(function SettingsView({ settings }: { settings: Settings }) {
  const incoming = useMemo(() => toDraft(settings), [settings]);
  const [base, setBase] = useState<Draft>(incoming);
  const [draft, setDraft] = useState<Draft>(incoming);
  const [showKey, setShowKey] = useState(false);
  const save = useAction();
  const [saved, setSaved] = useState(false);

  const dirty = !sameDraft(draft, base);
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;

  // Follow external changes (e.g. hotkey toggled overlay) unless the user is editing.
  useEffect(() => {
    if (sameDraft(incoming, base)) return;
    setBase(incoming);
    if (!dirtyRef.current) setDraft(incoming);
  }, [incoming, base]);

  useEffect(() => {
    if (!saved) return;
    const t = window.setTimeout(() => setSaved(false), 2000);
    return () => window.clearTimeout(t);
  }, [saved]);

  const hkErr: Record<keyof Settings['hotkeys'], string | null> = {
    toggleOverlay: acceleratorError(draft.hotkeys.toggleOverlay),
    toggleRecording: acceleratorError(draft.hotkeys.toggleRecording),
    enemyFlash: acceleratorError(draft.hotkeys.enemyFlash),
  };
  const modelErr = !draft.coach.liveModel.trim() || !draft.coach.reviewModel.trim();
  const invalid = Object.values(hkErr).some(Boolean) || modelErr;

  const setNum = (k: NumKey, raw: string) => {
    const n = Number(raw);
    if (raw === '' || !Number.isFinite(n)) return;
    setDraft((d) => ({ ...d, [k]: n }));
  };
  const setCoach = (patch: Partial<Settings['coach']>) => setDraft((d) => ({ ...d, coach: { ...d.coach, ...patch } }));
  const setRiot = (patch: Partial<Settings['riot']>) => setDraft((d) => ({ ...d, riot: { ...d.riot, ...patch } }));

  const onSave = async () => {
    const patch: Partial<Settings> = {
      autoRecord: draft.autoRecord,
      autoRunes: draft.autoRunes,
      launchAtLogin: draft.launchAtLogin,
      overlayVisible: draft.overlayVisible,
      minimapFps: Math.round(clamp(draft.minimapFps, ...LIMITS.minimapFps)),
      recordFps: Math.round(clamp(draft.recordFps, ...LIMITS.recordFps)),
      sceneEverySec: Math.round(clamp(draft.sceneEverySec, ...LIMITS.sceneEverySec)),
      hotkeys: {
        toggleOverlay: draft.hotkeys.toggleOverlay.trim(),
        toggleRecording: draft.hotkeys.toggleRecording.trim(),
        enemyFlash: draft.hotkeys.enemyFlash.trim(),
      },
      coach: {
        enabled: draft.coach.enabled,
        liveModel: draft.coach.liveModel.trim(),
        reviewModel: draft.coach.reviewModel.trim(),
        periodSec: Math.round(clamp(draft.coach.periodSec, ...PERIOD_LIMITS)),
        researchModel: (draft.coach.researchModel ?? 'claude-opus-5-5').trim() || 'claude-opus-5-5',
        draftModel: (draft.coach.draftModel ?? 'gpt-6.1-terra').trim() || 'gpt-6.1-terra',
      },
      riot: { apiKey: draft.riot.apiKey.trim(), platform: draft.riot.platform },
    };
    const r = await save.run(() => rc().updateSettings(patch));
    if (r.ok) {
      const next = toDraft(r.value);
      setBase(next);
      setDraft(next);
      setSaved(true);
    }
  };

  const numField = (k: NumKey, label: string, unit: string, hint: string) => {
    const [lo, hi] = LIMITS[k];
    return (
      <div className="field">
        <label htmlFor={`f-${k}`}>{label}</label>
        <div className="field-ctl">
          <input
            id={`f-${k}`}
            type="range"
            min={lo}
            max={hi}
            step={1}
            value={clamp(draft[k], lo, hi)}
            onChange={(e) => setNum(k, e.target.value)}
          />
          <input
            type="number"
            className="num-input"
            min={lo}
            max={hi}
            step={1}
            value={draft[k]}
            onChange={(e) => setNum(k, e.target.value)}
            aria-label={label}
          />
          <span className="unit muted">{unit}</span>
        </div>
        <div className="field-hint">{hint} ({lo}–{hi})</div>
      </div>
    );
  };

  const toggle = (checked: boolean, onChange: (v: boolean) => void, label: string, hint: string) => (
    <div className="field">
      <label className="switch-row">
        <input type="checkbox" className="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
        <span>{label}</span>
      </label>
      <div className="field-hint">{hint}</div>
    </div>
  );

  const hotkey = (k: keyof Settings['hotkeys'], label: string) => (
    <div className="field">
      <label htmlFor={`hk-${k}`}>{label}</label>
      <div className="field-ctl">
        <input
          id={`hk-${k}`}
          type="text"
          className={'text-input mono' + (hkErr[k] ? ' invalid' : '')}
          value={draft.hotkeys[k]}
          placeholder="np. Alt+Shift+O"
          spellCheck={false}
          onChange={(e) => { const v = e.target.value; setDraft((d) => ({ ...d, hotkeys: { ...d.hotkeys, [k]: v } })); }}
        />
      </div>
      <div className={hkErr[k] ? 'field-hint error-text' : 'field-hint'}>
        {hkErr[k] ?? 'Format Electron, np. Alt+Shift+O. Puste = wyłączony.'}
      </div>
    </div>
  );

  const [pLo, pHi] = PERIOD_LIMITS;
  const platformKnown = PLATFORMS.some((p) => p.id === draft.riot.platform);

  return (
    <section className="section">
      <div className="section-head">
        <h1>Ustawienia</h1>
        <div className="toolbar">
          {saved && !dirty && <span className="note-text">Zapisano</span>}
          <button type="button" className="btn" onClick={() => setDraft(base)} disabled={!dirty || save.busy}>Cofnij</button>
          <button type="button" className="btn primary" onClick={() => void onSave()} disabled={!dirty || invalid || save.busy}>
            {save.busy ? 'Zapisywanie…' : 'Zapisz'}
          </button>
        </div>
      </div>
      {save.error && <div className="error-text">{save.error}</div>}

      <div className="settings-grid">
        <div className="panel">
          <h2>Ogólne</h2>
          {toggle(draft.autoRecord, (v) => setDraft((d) => ({ ...d, autoRecord: v })), 'Nagrywaj automatycznie', 'Startuje nagranie, gdy zaczyna się obsługiwana gra.')}
          {toggle(draft.autoRunes, (v) => setDraft((d) => ({ ...d, autoRunes: v })), 'Automatyczne runy w wyborze postaci', 'Aplikacja ustawia stronę run „Rift Coach” po zablokowaniu postaci i poprawia ją, gdy pojawi się przeciwnik z linii. Twoich innych stron nie zmienia.')}
          {toggle(draft.overlayVisible, (v) => setDraft((d) => ({ ...d, overlayVisible: v })), 'Pokazuj nakładkę', 'Warstwa na grze (minimapa, komunikaty).')}
          {toggle(draft.launchAtLogin, (v) => setDraft((d) => ({ ...d, launchAtLogin: v })), 'Uruchamiaj z Windows (w tle)', 'Coach startuje po zalogowaniu, okno pozostaje ukryte; nakładka pojawi się w meczu. Okno otworzysz ikoną na pulpicie.')}
        </div>
        <div className="panel">
          <h2>Coach</h2>
          {toggle(draft.coach.enabled, (v) => setCoach({ enabled: v }), 'Porady Claude', 'Krótkie porady na pasku u góry i podsumowanie po śmierci.')}
          <div className="field">
            <label htmlFor="coach-live">Model w grze</label>
            <ModelPicker id="coach-live" value={draft.coach.liveModel} options={LIVE_MODELS} onChange={(v) => setCoach({ liveModel: v })} />
          </div>
          <div className="field">
            <label htmlFor="coach-review">Model po śmierci</label>
            <ModelPicker id="coach-review" value={draft.coach.reviewModel} options={REVIEW_MODELS} onChange={(v) => setCoach({ reviewModel: v })} />
            {modelErr && <div className="field-hint error-text">Podaj nazwę modelu.</div>}
          </div>
          <div className="field">
            <label htmlFor="coach-research">Research (patch, matchup)</label>
            <ModelPicker id="coach-research" value={draft.coach.researchModel ?? 'claude-opus-5-5'} options={RESEARCH_MODELS} onChange={(v) => setCoach({ researchModel: v })} />
          </div>
          <div className="field">
            <label htmlFor="coach-draft">Draft (runy, role)</label>
            <ModelPicker id="coach-draft" value={draft.coach.draftModel ?? 'gpt-6.1-terra'} options={DRAFT_MODELS} onChange={(v) => setCoach({ draftModel: v })} />
          </div>
          <div className="field">
            <label htmlFor="coach-period">Porada co ile sekund</label>
            <div className="field-ctl">
              <input
                id="coach-period"
                type="range"
                min={pLo}
                max={pHi}
                step={5}
                value={clamp(draft.coach.periodSec, pLo, pHi)}
                onChange={(e) => setCoach({ periodSec: Number(e.target.value) })}
              />
              <input
                type="number"
                className="num-input"
                min={pLo}
                max={pHi}
                step={1}
                value={draft.coach.periodSec}
                onChange={(e) => { const n = Number(e.target.value); if (e.target.value !== '' && Number.isFinite(n)) setCoach({ periodSec: n }); }}
                aria-label="Porada co ile sekund"
              />
              <span className="unit muted">s</span>
            </div>
            <div className="field-hint">({pLo}–{pHi})</div>
          </div>
          <div className="help-line">
            Coach używa Claude Code zalogowanego Twoją subskrypcją. Jeśli status pokazuje „niezalogowany”, uruchom w terminalu: <code>claude</code> i zaloguj się.
          </div>
        </div>
        <div className="panel">
          <h2>Riot API</h2>
          <div className="field">
            <label htmlFor="riot-key">Klucz API</label>
            <div className="field-ctl">
              <input
                id="riot-key"
                type={showKey ? 'text' : 'password'}
                className="text-input mono key-input"
                value={draft.riot.apiKey}
                placeholder="RGAPI-…"
                spellCheck={false}
                autoComplete="off"
                onChange={(e) => setRiot({ apiKey: e.target.value })}
              />
              <button type="button" className="btn small" onClick={() => setShowKey((v) => !v)}>
                {showKey ? 'Ukryj' : 'Pokaż'}
              </button>
            </div>
          </div>
          <div className="field">
            <label htmlFor="riot-platform">Serwer</label>
            <div className="field-ctl">
              <select id="riot-platform" value={draft.riot.platform} onChange={(e) => setRiot({ platform: e.target.value })}>
                {!platformKnown && <option value={draft.riot.platform}>{draft.riot.platform || '—'}</option>}
                {PLATFORMS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
              </select>
            </div>
          </div>
          <div className="help-line">
            Klucz weźmiesz z{' '}
            <a href="https://developer.riotgames.com" target="_blank" rel="noreferrer">developer.riotgames.com</a>
            {' '}— klucz deweloperski jest darmowy i ważny 24 h.
          </div>
        </div>
        <div className="panel">
          <h2>Przechwytywanie</h2>
          {numField('minimapFps', 'Minimapa na żywo', 'fps', 'Klatki minimapy na sekundę dla trackera')}
          {numField('recordFps', 'Minimapa w nagraniu', 'fps', 'Klatki minimapy zapisywane do nagrania')}
          {numField('sceneEverySec', 'Pełny ekran w nagraniu co', 's', 'Odstęp między zrzutami całego ekranu')}
        </div>
        <div className="panel">
          <h2>Skróty</h2>
          {hotkey('toggleOverlay', 'Pokaż / ukryj nakładkę')}
          {hotkey('toggleRecording', 'Nagrywanie start / stop')}
          {hotkey('enemyFlash', 'Flash wroga użyty')}
        </div>
      </div>
    </section>
  );
});
