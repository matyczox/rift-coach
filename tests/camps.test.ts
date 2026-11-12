import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, resize } from '../src/main/vision/png';
import { CAMPS, CampTracker, readCamps, type CampIconState, type CampReading } from '../src/main/vision/camps';

const FIX = path.join(__dirname, 'fixtures', 'camps');
const load = (f: string) => decodePng(readFileSync(path.join(FIX, f)));

// Labels checked by eye on 5x crops (null = occluded / not asserted). Fixtures are real 341 px replay crops:
// frames-* = blue view with fog, red-* = red view with fog, truth-* = fog-free.
type Expect = Record<string, CampIconState | null>;
const EXPECTED: Record<string, Expect> = {
  // 1:40 – everything up except both blue buffs (taken at ~1:18); blue gromp covered by a portrait.
  'frames-99935.png': {
    blue_wolves: 'up', blue_raptors: 'up', blue_red: 'up', blue_krugs: 'up', red_gromp: 'up', red_blue: 'empty',
    red_wolves: 'up', red_raptors: 'up', red_red: 'up', red_krugs: 'up', scuttle_bot: 'empty',
  },
  // 6:40 – respawn hourglasses on enemy camps are visible through fog.
  'frames-399947.png': {
    blue_gromp: 'empty', blue_blue: 'up', blue_wolves: 'empty', blue_raptors: 'empty', blue_red: 'empty', blue_krugs: 'empty',
    red_gromp: 'respawn_soon', red_blue: 'up', red_wolves: 'respawning', red_raptors: 'empty', red_red: 'empty', red_krugs: 'empty',
  },
  // Same moment from the red side: identical camp icons (camp state is not team-specific in replays).
  'red-399883.png': {
    blue_gromp: 'empty', blue_blue: 'up', blue_wolves: 'empty', blue_raptors: 'empty', blue_red: 'empty', blue_krugs: 'empty',
    red_gromp: 'respawn_soon', red_blue: 'up', red_wolves: 'respawning', red_raptors: 'empty', red_red: 'empty', red_krugs: 'empty',
  },
  // 0:40 – pre-spawn: grey hourglasses everywhere.
  'truth-40073.png': {
    blue_gromp: 'respawning', blue_wolves: 'respawning', blue_raptors: 'respawning', blue_red: 'respawning', blue_krugs: 'respawning',
    red_gromp: 'respawning', red_blue: 'respawning', red_wolves: 'respawning', red_raptors: 'respawning', red_red: 'respawning', red_krugs: 'respawning',
  },
  'frames-1283210.png': {
    blue_gromp: 'up', blue_blue: 'up', blue_raptors: 'empty', red_gromp: 'empty', red_blue: 'empty', red_raptors: 'empty',
    red_red: 'empty', red_krugs: 'respawn_soon', scuttle_top: 'respawning', scuttle_bot: 'empty',
  },
  'red-1699894.png': {
    blue_gromp: 'respawning', blue_blue: 'up', blue_raptors: 'empty', blue_red: 'respawning', red_gromp: 'respawn_soon',
    red_wolves: 'up', red_raptors: 'empty', red_krugs: 'empty', scuttle_top: 'empty',
  },
};

describe('readCamps', () => {
  it.each(Object.keys(EXPECTED))('reads the camp icons of %s', (file) => {
    const got = new Map(readCamps(load(file)).map(r => [r.id, r]));
    expect(got.size).toBe(CAMPS.length);
    for (const [id, icon] of Object.entries(EXPECTED[file]!)) {
      const r = got.get(id)!;
      expect({ id, icon: r.icon }).toEqual({ id, icon });
      expect(r.present).toBe(icon === null ? null : icon === 'up');
    }
  });

  it.each([256, 420])('gives the same confident readings on a %i px crop', (size) => {
    for (const file of ['frames-99935.png', 'frames-399947.png', 'truth-40073.png']) {
      const img = load(file);
      const native = readCamps(img), scaled = readCamps(resize(img, size, size));
      native.forEach((r, i) => {
        const s = scaled[i]!;
        if (r.icon && r.confidence >= 0.5) expect({ file, id: r.id, icon: s.icon }).toEqual({ file, id: r.id, icon: r.icon });
        if (s.icon) expect({ file, id: r.id, icon: s.icon }).toEqual({ file, id: r.id, icon: r.icon ?? s.icon });
      });
    }
  });
});

// ---- tracker -------------------------------------------------------------------------------

const all = (icon: CampIconState | null, over: Record<string, CampIconState | null> = {}): CampReading[] =>
  CAMPS.map(c => { const i = c.id in over ? over[c.id]! : icon; return { id: c.id, side: c.side, present: i === null ? null : i === 'up', icon: i, confidence: i ? 1 : 0 }; });
const st = (tr: CampTracker, id: string) => tr.status().find(s => s.id === id)!;

describe('CampTracker', () => {
  it('turns a vanishing icon into a clear and follows the hourglass to the respawn', () => {
    const tr = new CampTracker();
    let t = 100;
    for (let i = 0; i < 4; i++) tr.update(t++, all('up'));
    expect(st(tr, 'red_raptors').state).toBe('up');
    tr.update(t++, all('up', { red_raptors: null })); // portrait over the camp during the kill
    for (let i = 0; i < 4; i++) tr.update(t++, all('up', { red_raptors: 'empty' }));
    const ev = tr.events.filter(e => e.kind === 'cleared');
    expect(ev).toEqual([{ id: 'red_raptors', side: 'CHAOS', at: 105, kind: 'cleared', respawnAt: 105 + 135, earliest: 103 }]);
    expect(st(tr, 'red_raptors')).toMatchObject({ state: 'down', downSince: 105, respawnAt: 240 });
    // grey hourglass appears 30 s before the real respawn (camp actually died at 104 -> respawn 239)
    for (t = 209; t < 213; t++) tr.update(t, all('up', { red_raptors: 'respawning' }));
    expect(st(tr, 'red_raptors')).toMatchObject({ state: 'down', respawnAt: 239, downSince: 104 });
    for (t = 229; t < 233; t++) tr.update(t, all('up', { red_raptors: 'respawn_soon' }));
    expect(st(tr, 'red_raptors').respawnAt).toBe(239);
    for (t = 239; t < 243; t++) tr.update(t, all('up'));
    expect(st(tr, 'red_raptors')).toMatchObject({ state: 'up', downSince: null, respawnAt: null });
    expect(tr.events.map(e => e.kind)).toEqual(['cleared', 'respawned']);
  });

  it('ignores short flickers', () => {
    const tr = new CampTracker();
    for (let t = 0; t < 4; t++) tr.update(t, all('up'));
    tr.update(4, all('up', { blue_gromp: 'empty' }));
    tr.update(5, all('up', { blue_gromp: 'empty' }));
    for (let t = 6; t < 10; t++) tr.update(t, all('up'));
    expect(tr.events).toEqual([]);
    expect(st(tr, 'blue_gromp').state).toBe('up');
  });

  it('starts unknown and learns respawn times from a first-seen hourglass', () => {
    const tr = new CampTracker();
    for (let t = 300; t < 304; t++) tr.update(t, all(null, { red_blue: 'respawning' }));
    expect(st(tr, 'red_blue')).toMatchObject({ state: 'down', respawnAt: 330, downSince: 30 });
    expect(st(tr, 'red_red').state).toBe('unknown');
    expect(tr.events).toEqual([]);
  });

  it('retracts a clear that is not followed by the hourglass', () => {
    const tr = new CampTracker();
    for (let t = 0; t < 4; t++) tr.update(t, all('up'));
    for (let t = 4; t < 200; t++) tr.update(t, all('up', { blue_krugs: 'empty' }));
    expect(tr.events.map(e => e.kind)).toEqual(['cleared', 'retracted']);
    expect(st(tr, 'blue_krugs').state).toBe('unknown');
  });

  it('does not treat a scuttle leaving vision as a kill', () => {
    const tr = new CampTracker();
    for (let t = 0; t < 4; t++) tr.update(t, all('up'));
    for (let t = 4; t < 8; t++) tr.update(t, all('up', { scuttle_top: 'empty' }));
    expect(tr.events).toEqual([]);
    expect(st(tr, 'scuttle_top').state).toBe('unknown');
    // ...but it is a kill when the spot was in vision
    const tr2 = new CampTracker();
    const vis = () => 1;
    for (let t = 0; t < 4; t++) tr2.update(t, all('up'), vis);
    for (let t = 4; t < 8; t++) tr2.update(t, all('up', { scuttle_top: 'empty' }), vis);
    expect(tr2.events.map(e => [e.id, e.kind])).toEqual([['scuttle_top', 'cleared']]);
  });

  it('with a vision mask, trusts vanishing icons in fog only once icons were seen drawn in fog', () => {
    const fogRed = (x: number, y: number) => (x + (1 - y) > 1.05 ? 0 : 1); // enemy (top-right) half fogged
    const tr = new CampTracker();
    // A client that hides fogged camps: enemy icons never drawn in fog -> no claims.
    for (let t = 0; t < 4; t++) tr.update(t, all('up', { red_gromp: 'up' }), () => 1);
    for (let t = 4; t < 9; t++) tr.update(t, all('up', { red_gromp: 'empty' }), fogRed);
    expect(tr.events).toEqual([]);
    expect(st(tr, 'red_gromp').state).toBe('unknown');
    // Replays / current client: enemy camp icons are drawn inside fog -> vanishing there is a real clear.
    const tr2 = new CampTracker();
    for (let t = 0; t < 5; t++) tr2.update(t, all('up'), fogRed);
    for (let t = 5; t < 10; t++) tr2.update(t, all('up', { red_gromp: 'empty' }), fogRed);
    expect(tr2.events.map(e => [e.id, e.kind])).toEqual([['red_gromp', 'cleared']]);
  });

  it('tracks a clear from real crops', () => {
    const a = readCamps(load('frames-99935.png')), b = readCamps(load('frames-399947.png'));
    const tr = new CampTracker();
    for (let t = 100; t < 104; t++) tr.update(t, a);
    for (let t = 104; t < 108; t++) tr.update(t, b);
    const cleared = tr.events.filter(e => e.kind === 'cleared').map(e => e.id).sort();
    expect(cleared).toEqual(['blue_krugs', 'blue_raptors', 'blue_red', 'blue_wolves', 'red_krugs', 'red_raptors', 'red_red']);
    expect(st(tr, 'red_wolves')).toMatchObject({ state: 'down', respawnAt: 104 + 30 });
    expect(st(tr, 'red_blue').state).toBe('up');
  });
});
