import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import type { Settings } from '../shared/types';

export const DEFAULT_SETTINGS: Settings = {
  calibration: null,
  minimapFps: 10,
  recordFps: 5,
  sceneEverySec: 5,
  autoRecord: false,
  overlayVisible: true,
  hotkeys: { toggleOverlay: 'Alt+Shift+O', toggleRecording: 'Alt+Shift+R', enemyFlash: 'Alt+Shift+F' },
  // Picked from a 6-model comparison on real match situations: GPT-6.1-Sol = fast, accurate, honest
  // for the in-game call; Opus 5.5 = best reasoning for death reviews and the post-game review.
  coach: { enabled: true, liveModel: 'gpt-6.1-terra', reviewModel: 'claude-opus-5-5', periodSec: 25, researchModel: 'claude-opus-5-5', draftModel: 'gpt-6.1-terra', fast: true },
  riot: { apiKey: '', platform: 'eun1' },
  autoRunes: true,
  launchAtLogin: false,
};

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, Math.round(v)));

export function sanitize(s: Partial<Settings>): Settings {
  const m = { ...DEFAULT_SETTINGS, ...s, hotkeys: { ...DEFAULT_SETTINGS.hotkeys, ...s.hotkeys }, coach: { ...DEFAULT_SETTINGS.coach, ...s.coach }, riot: { ...DEFAULT_SETTINGS.riot, ...s.riot } };
  m.coach.periodSec = clamp(m.coach.periodSec, 10, 120);
  m.riot.apiKey = m.riot.apiKey.trim();
  m.minimapFps = clamp(m.minimapFps, 2, 20);
  m.recordFps = clamp(m.recordFps, 1, m.minimapFps);
  m.sceneEverySec = clamp(m.sceneEverySec, 0, 60);
  return m;
}

export class SettingsStore {
  value: Settings;
  private file: string;

  constructor(dir: string) {
    mkdirSync(dir, { recursive: true });
    this.file = path.join(dir, 'settings.json');
    let raw: Partial<Settings> = {};
    try { raw = JSON.parse(readFileSync(this.file, 'utf8')); } catch { /* first run */ }
    this.value = sanitize(raw);
  }

  update(patch: Partial<Settings>): Settings {
    this.value = sanitize({ ...this.value, ...patch });
    writeFileSync(this.file, JSON.stringify(this.value, null, 2));
    return this.value;
  }
}
