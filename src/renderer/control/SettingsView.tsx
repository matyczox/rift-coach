import { memo, useEffect, useMemo, useRef, useState } from 'react';
import type { Settings } from '../../shared/types';
import { clamp, rc, useAction, errText } from './util';
import { createRiotAutosave, cleanRiotConfig, sameRiotConfig } from './riotAutosave';

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
  && a.coach.reviewModel === b.coach.reviewModel && a.coach.periodSec === b.coach.periodSec && (a.coach.fast !== false) === (b.coach.fast !== false)
  && a.coach.researchModel === b.coach.researchModel && a.coach.draftModel === b.coach.draftModel
  && a.riot.apiKey === b.riot.apiKey && a.riot.platform === b.riot.platform;

const MODIFIERS = new Set([
  'Command', 'Cmd', 'Control', 'Ctrl', 'CommandOrControl', 'CmdOrCtrl', 'Alt', 'Option', 'AltGr', 'Shift', 'Super', 'Meta',
]);
const KEY_RE = /^([0-9A-Z]|F([1-9]|1[0-9]|2[0-4])|Plus|Space|Tab|Capslock|Numlock|Scrolllock|Backspace|Delete|Insert|Return|Enter|Up|Down|Left|Right|Home|End|PageUp|PageDown|Escape|Esc|PrintScreen|VolumeUp|VolumeDown|VolumeMute|MediaNextTrack|MediaPreviousTrack|MediaStop|MediaPlayPause|num[0-9]|numdec|numadd|numsub|nummult|numdiv|[)!@#$%^&*(:;=<,_.>?/~`{}|\\"'[\]-])$/i;

/** Light validation of a hotkey string (e.g. Alt+Shift+O). Empty = disabled. */
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
        {options.map((o) => <option key={o} value={o}>{prettyModel(o)}</option>)}
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

/** Friendly model names in the pickers: gpt-6.1-terra -> "Terra (Codex)", claude-opus-5-5 -> "Opus 5.5 (Claude)". */
const prettyModel = (m: string) => {
  const name = m.replace(/^gpt-[\d.]+-/, '').replace(/^claude-/, '').replace(/-(\d+)-(\d+)$/, ' $1.$2').replace(/^./, c => c.toUpperCase());
  return m.startsWith('gpt') ? name + ' · Codex' : m.startsWith('claude') ? name + ' · Claude' : m;
};

/** Row of choice buttons (instead of sliders). */
function Seg<T extends number>({ value, options, onChange }: { value: T; options: Array<{ v: T; label: string }>; onChange: (v: T) => void }) {
  const near = options.reduce((b, o) => (Math.abs(o.v - value) < Math.abs(b.v - value) ? o : b), options[0]!).v;
  return (
    <div className="seg">
      {options.map(o => <button key={o.v} type="button" className={'seg-btn' + (o.v === near ? ' on' : '')} onClick={() => onChange(o.v)}>{o.label}</button>)}
    </div>
  );
}

/** Click, then press the key combination (Backspace clears). */
function KeyCapture({ id, value, onChange }: { id: string; value: string; onChange: (v: string) => void }) {
  const [rec, setRec] = useState(false);
  return (
    <button
      id={id}
      type="button"
      className={'key-cap' + (rec ? ' rec' : '')}
      onClick={() => setRec(true)}
      onBlur={() => setRec(false)}
      onKeyDown={(e) => {
        if (!rec) return;
        e.preventDefault();
        if (e.key === 'Escape') { setRec(false); return; }
        if (e.key === 'Backspace' || e.key === 'Delete') { onChange(''); setRec(false); return; }
        if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return;
        const k = /^F\d{1,2}$/.test(e.key) ? e.key : e.code.startsWith('Key') ? e.code.slice(3) : e.code.startsWith('Digit') ? e.code.slice(5) : e.key.length === 1 ? e.key.toUpperCase() : e.key;
        const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift'].filter(Boolean);
        if (!mods.length && !/^F\d{1,2}$/.test(k)) return; // need a modifier (except F-keys)
        onChange([...mods, k].join('+'));
        setRec(false);
      }}
    >
      {rec ? 'Naciśnij skrót…' : value ? value.split('+').map((p, i) => <kbd key={i}>{p}</kbd>) : <span className="muted">brak</span>}
    </button>
  );
}

export const SettingsView = memo(function SettingsView({ settings }: { settings: Settings }) {
  const incoming = useMemo(() => toDraft(settings), [settings]);
  const [base, setBase] = useState<Draft>(incoming);
  const [draft, setDraft] = useState<Draft>(incoming);
  const [showKey, setShowKey] = useState(false);
  const save = useAction();
  const [saved, setSaved] = useState(false);
  const [riotState, setRiotState] = useState<'idle' | 'pending' | 'saving' | 'saved'>('idle');
  const [riotError, setRiotError] = useState<string | null>(null);
  const riotAutosave = useRef<ReturnType<typeof createRiotAutosave> | null>(null);
  const draftRef=useRef(draft);
  draftRef.current=draft;
  useEffect(() => {
    const controller=createRiotAutosave(incoming.riot,
    async (riot) => (await rc().updateSettings({ riot })).riot, {
      state: (state) => { setRiotState(state); setRiotError(null); },
      saved: (value, submitted) => {
        setBase((old) => ({ ...old, riot: value }));
        setDraft((old) => sameRiotConfig(cleanRiotConfig(old.riot), submitted) ? { ...old, riot: value } : old);
      },
      error: (error) => { setRiotState('idle'); setRiotError(errText(error)); },
    });
    riotAutosave.current=controller;
    controller.update(draftRef.current.riot);
    return () => {
      controller.update(draftRef.current.riot);
      controller.dispose();
      if(riotAutosave.current===controller)riotAutosave.current=null;
    };
  }, []);
  useEffect(() => { riotAutosave.current?.update(draft.riot); }, [draft.riot.apiKey, draft.riot.platform]);

  const dirty = !sameDraft(draft, base);
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  const lastIncoming = useRef(incoming);

  // Follow external changes (e.g. hotkey toggled overlay) unless the user is editing.
  useEffect(() => {
    // A successful local save can precede the next status poll. Do not restore
    // its stale settings merely because our local base has just changed.
    if (sameDraft(incoming, lastIncoming.current)) return;
    lastIncoming.current=incoming;
    setBase(incoming);
    if (!dirtyRef.current) setDraft(incoming);
  }, [incoming]);

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
    if (!sameRiotConfig(cleanRiotConfig(draft.riot), cleanRiotConfig(base.riot))) await riotAutosave.current?.flush();
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
        fast: draft.coach.fast !== false,
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
      <KeyCapture id={`hk-${k}`} value={draft.hotkeys[k]} onChange={(v) => setDraft((d) => ({ ...d, hotkeys: { ...d.hotkeys, [k]: v } }))} />
      {hkErr[k] && <div className="field-hint error-text">{hkErr[k]}</div>}
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
          {toggle(draft.autoRecord, (v) => setDraft((d) => ({ ...d, autoRecord: v })), 'Nagrywaj automatycznie', 'Zapisuje mecze do nauki trackera.')}
          {toggle(draft.autoRunes, (v) => setDraft((d) => ({ ...d, autoRunes: v })), 'Automatyczne runy w wyborze postaci', 'Strona „Rift Coach” pod Twoją postać i przeciwnika. Innych stron nie rusza.')}
          {toggle(draft.overlayVisible, (v) => setDraft((d) => ({ ...d, overlayVisible: v })), 'Pokazuj nakładkę', 'Warstwa na grze (minimapa, komunikaty).')}
          {toggle(draft.launchAtLogin, (v) => setDraft((d) => ({ ...d, launchAtLogin: v })), 'Uruchamiaj z Windows (w tle)', 'Działa w tle od włączenia komputera; okno otworzysz ikoną na pulpicie.')}
        </div>
        <div className="panel">
          <h2>Coach</h2>
          {toggle(draft.coach.enabled, (v) => setCoach({ enabled: v }), 'Coach AI', 'Porady w grze, analiza po śmierci i po meczu.')}
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
          {toggle(draft.coach.fast !== false, (v) => setCoach({ fast: v }), 'Tryb fast (Terra)', 'Szybsze porady w grze; zużywa więcej limitu ChatGPT.')}
          <div className="field">
            <label>Porady w grze</label>
            <Seg value={draft.coach.periodSec} options={[{ v: 15, label: 'Często' }, { v: 25, label: 'Normalnie' }, { v: 45, label: 'Rzadko' }]} onChange={(v) => setCoach({ periodSec: v })} />
            <div className="field-hint">Co {draft.coach.periodSec} s, plus zawsze po śmierci i przed obiektami.</div>
          </div>
          <div className="help-line">
            Coach korzysta z Twoich subskrypcji: Codex (ChatGPT) i/lub Claude Code. Aplikacja sama wybiera najlepsze modele do tego, co masz zainstalowane. Gdy coach pokazuje „niezalogowany”, uruchom w terminalu <code>claude</code> i zaloguj się.
          </div>
        </div>
        <div className="panel">
          <h2>Riot API</h2>
          <div className="field-hint" role="status">
            {riotError ? `Nie zapisano klucza: ${riotError}` : riotState === 'saving' || riotState === 'pending'
              ? 'Zapisywanie klucza…' : riotState === 'saved'
                ? (draft.riot.apiKey.trim() ? 'Klucz zapisany — aplikacja używa go od razu.' : 'Klucz usunięty.')
                : 'Wklejenie klucza i zmiana serwera zapisują się automatycznie.'}
          </div>
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
          <h2>Wydajność</h2>
          <div className="field">
            <label>Śledzenie minimapy</label>
            <Seg value={draft.minimapFps} options={[{ v: 5, label: 'Oszczędnie' }, { v: 10, label: 'Normalnie' }, { v: 15, label: 'Dokładnie' }]} onChange={(v) => setDraft((d) => ({ ...d, minimapFps: v }))} />
            <div className="field-hint">Ile razy na sekundę aplikacja patrzy na minimapę ({draft.minimapFps}/s).</div>
          </div>
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
