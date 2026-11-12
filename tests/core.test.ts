import { describe, expect, it } from 'vitest';
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { findMinimap, scaleRect } from '../src/main/calibration';
import { lastAtOrBefore } from '../src/main/replay';
import { Recorder, loadSession } from '../src/main/recorder';
import { sanitize } from '../src/main/settings';
import { toGameInfo, type AllGameData } from '../src/main/sources/liveApi';

const fixture = (n: string) => path.join(__dirname, 'fixtures', n);

describe('minimap calibration', () => {
  it('finds the minimap on a real 1600x900 frame', () => {
    const raw = gunzipSync(readFileSync(fixture('scene-1600x900-br.rgba.gz')));
    const nl = raw.indexOf(10);
    const h = JSON.parse(raw.subarray(0, nl).toString());
    // Rebuild a full-size frame (black elsewhere) so the search runs as in the app.
    const full = Buffer.alloc(h.fullWidth * h.fullHeight * 4);
    const crop = raw.subarray(nl + 1);
    for (let y = 0; y < h.height; y++) crop.copy(full, ((y + h.offsetY) * h.fullWidth + h.offsetX) * 4, y * h.width * 4, (y + 1) * h.width * 4);
    const r = findMinimap(full, h.fullWidth, h.fullHeight);
    expect(r).not.toBeNull();
    // Hand-measured minimap square (inside the rim): x 1370..1591, y 669..890 (±4 px).
    expect(Math.abs(r!.rect.x - 1370)).toBeLessThanOrEqual(4);
    expect(Math.abs(r!.rect.y - 669)).toBeLessThanOrEqual(4);
    expect(Math.abs(r!.rect.width - 221)).toBeLessThanOrEqual(6);
  });

  it('returns null on a featureless frame', () => {
    expect(findMinimap(Buffer.alloc(640 * 360 * 4, 90), 640, 360)).toBeNull();
  });

  it('scales a calibration to another window size, anchored bottom-right', () => {
    const r = scaleRect({ x: 1379, y: 678, width: 211, height: 211 }, 1600, 900, 1920, 1080);
    expect(r.width).toBe(253);
    expect(r.x + r.width).toBe(1920 - 12);
    expect(r.y + r.height).toBe(1080 - 13);
  });
});

describe('live api', () => {
  it('splits teams relative to the active player', () => {
    const d = JSON.parse(readFileSync(fixture('allgamedata.json'), 'utf8')) as AllGameData;
    const g = toGameInfo(d);
    expect(g.activeChampion).toBe('Ambessa');
    expect(g.allies).toContain('Fizz');
    expect(g.enemies).toEqual(['Lulu', 'Tristana', 'Xin Zhao', 'Warwick', 'Twisted Fate']);
  });
});

describe('replay', () => {
  it('lastAtOrBefore', () => {
    const a = [0, 100, 200, 300];
    expect(lastAtOrBefore(a, -1, v => v)).toBe(-1);
    expect(lastAtOrBefore(a, 0, v => v)).toBe(0);
    expect(lastAtOrBefore(a, 250, v => v)).toBe(2);
    expect(lastAtOrBefore(a, 999, v => v)).toBe(3);
  });

  it('records and loads a session round-trip', async () => {
    const root = mkdtempSync(path.join(tmpdir(), 'rc-'));
    try {
      const rec = new Recorder(root, () => 5);
      rec.start();
      const t0 = rec.status.startedAt!;
      const d = JSON.parse(readFileSync(fixture('allgamedata.json'), 'utf8')) as AllGameData;
      rec.frame({ at: t0 + 10, width: 2, height: 2, png: Buffer.from('a') });
      rec.frame({ at: t0 + 50, width: 2, height: 2, png: Buffer.from('b') }); // dropped: > recordFps
      rec.frame({ at: t0 + 260, width: 2, height: 2, png: Buffer.from('c') });
      rec.apiSample(d, t0 + 20);
      rec.apiSample(d, t0 + 1100);
      rec.stop();
      await new Promise(r => setTimeout(r, 100)); // let gzip flush
      const s = loadSession(root, rec.status.sessionId!);
      expect(s.frames).toEqual([10, 260]);
      expect(s.api.map(a => a.o)).toEqual([20, 1100]);
      expect(s.meta.champion).toBe('Ambessa');
      expect(rec.list()).toHaveLength(1);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});

describe('settings', () => {
  it('clamps values and keeps defaults', () => {
    const s = sanitize({ minimapFps: 99, recordFps: 50 });
    expect(s.minimapFps).toBe(20);
    expect(s.recordFps).toBe(20);
    expect(s.hotkeys.toggleOverlay).toBe('Alt+Shift+O');
  });
});

import { pruneRecordings } from '../src/main/recorder';
import { writeFileSync as wf, mkdirSync as md, readdirSync as rd, existsSync as ex } from 'node:fs';
describe('recording housekeeping', () => {
  it('deletes everything but the newest sessions and anything too old', () => {
    const root = mkdtempSync(path.join(tmpdir(), 'rc-prune-'));
    try {
      const mk = (id: string, startedAt: number, source?: string) => {
        md(path.join(root, id, 'frames'), { recursive: true });
        for (let i = 0; i < 10; i++) wf(path.join(root, id, 'frames', `${i * 100}.png`), 'x'.repeat(100));
        wf(path.join(root, id, 'meta.json'), JSON.stringify({ id, startedAt, source }));
      };
      const now = Date.now();
      mk('new-replay', now, 'replay');
      mk('old-replay', now - 1000, 'replay');
      mk('old-live', now - 30 * 86400000);
      const freed = pruneRecordings(root, { keepFull: 1, thinTo: 5, maxAgeDays: 14 });
      expect(freed).toBeGreaterThan(0);
      expect(rd(path.join(root, 'new-replay', 'frames'))).toHaveLength(10);
      expect(ex(path.join(root, 'old-replay'))).toBe(false);
      expect(ex(path.join(root, 'old-live'))).toBe(false);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});

import { normalizeChampionNames } from '../src/main/sources/liveApi';
describe('localised champion names', () => {
  it('rewrites Polish names to English via rawChampionName', () => {
    const d = JSON.parse(readFileSync(fixture('allgamedata.json'), 'utf8')) as AllGameData;
    d.allPlayers[0]!.championName = 'Nunu i Willump';
    d.allPlayers[0]!.rawChampionName = 'game_character_displayname_Nunu';
    const out = normalizeChampionNames(d, id => ({ Nunu: 'Nunu & Willump' } as Record<string, string>)[id]);
    expect(out.allPlayers[0]!.championName).toBe('Nunu & Willump');
    expect(out.allPlayers[1]!.championName).toBe(d.allPlayers[1]!.championName);
  });
});
