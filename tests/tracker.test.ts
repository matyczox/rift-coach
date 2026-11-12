import { describe, expect, it } from 'vitest';
import { EnemyTracker, type EnemyInput, type FrameInput } from '../src/main/tracker/tracker';
import { MAP_UNITS, ParticleFilter, goalsFor, rng } from '../src/main/tracker/predictor';
import { openTerrain } from '../src/main/tracker/terrain';
import { Learner } from '../src/main/tracker/learning';
import type { Detection } from '../src/main/vision/detect';

const ww: EnemyInput = { champion: 'Warwick', team: 'CHAOS', role: 'JUNGLE', isDead: false, moveSpeed: 345 };
const det = (x: number, y: number): Detection => ({ champion: 'Warwick', team: 'CHAOS', x, y, score: 0.9, margin: 0.5, votes: 1 });
const frame = (t: number, dets: Detection[], extra: Partial<FrameInput> = {}): FrameInput => ({ t, detections: dets, enemies: [ww], dragonIn: null, baronIn: null, ...extra });

describe('EnemyTracker state machine', () => {
  it('visible -> fog after 3 missed frames, ghost near last sighting, gone instantly on reappearance', () => {
    const learner = new Learner(null);
    const tr = new EnemyTracker(openTerrain(64), learner, 200);
    let t = 200;
    for (let i = 0; i < 5; i++, t += 0.1) tr.update(frame(t, [det(0.6, 0.4)]));
    expect(tr.all[0]!.state).toBe('visible');
    expect(tr.all[0]!.estimate).toBeNull();

    for (let i = 0; i < 4; i++, t += 0.1) tr.update(frame(t, []));
    const w = tr.all[0]!;
    expect(w.state).toBe('fog');
    expect(w.estimate).not.toBeNull();
    expect(Math.hypot(w.estimate!.x - 0.6, w.estimate!.y - 0.4)).toBeLessThan(0.05);

    // 8 s in fog: the area may not be wider than move speed allows.
    for (let i = 0; i < 80; i++, t += 0.1) tr.update(frame(t, []));
    const maxReach = (345 / MAP_UNITS) * 9;
    expect(Math.hypot(tr.all[0]!.estimate!.x - 0.6, tr.all[0]!.estimate!.y - 0.4)).toBeLessThan(maxReach);

    // Seen again: no ghost in the same frame, and the model learned from it.
    tr.update(frame(t, [det(0.62, 0.45)]));
    expect(tr.all[0]!.state).toBe('visible');
    expect(tr.all[0]!.estimate).toBeNull();
    expect(learner.summary().samples).toBe(1);
  });

  it('dead enemies have no ghost and restart at their fountain', () => {
    const tr = new EnemyTracker(openTerrain(64), new Learner(null), 100);
    tr.update(frame(300, [det(0.5, 0.5)]));
    tr.update(frame(301, [], { enemies: [{ ...ww, isDead: true }] }));
    expect(tr.all[0]!.state).toBe('dead');
    expect(tr.all[0]!.estimate).toBeNull();
    tr.update(frame(330, [], { enemies: [ww] }));
    const w = tr.all[0]!;
    expect(w.state).toBe('fog');
    expect(w.estimate!.x).toBeGreaterThan(0.85);
    expect(w.estimate!.y).toBeLessThan(0.15);
  });

  it('ignores a single teleport-like detection until confirmed', () => {
    const tr = new EnemyTracker(openTerrain(64), new Learner(null), 100);
    let t = 400;
    for (let i = 0; i < 3; i++, t += 0.1) tr.update(frame(t, [det(0.2, 0.2)]));
    tr.update(frame(t, [det(0.8, 0.8)])); t += 0.1;
    expect(tr.all[0]!.last!.x).toBeCloseTo(0.2, 2);
    tr.update(frame(t, [det(0.8, 0.8)])); // confirmed (e.g. teleport)
    expect(tr.all[0]!.last!.x).toBeCloseTo(0.8, 2);
  });
});

describe('ParticleFilter', () => {
  it('removes hypotheses inside areas we can see', () => {
    const pf = new ParticleFilter(openTerrain(64), goalsFor('CHAOS'), 300, rng(7));
    pf.reset({ x: 0.5, y: 0.5 }, { role: 'JUNGLE', gameTime: 600, dragonIn: null, baronIn: null, likelyRecall: false });
    pf.step(10, 345);
    // We see the whole left half of the map.
    pf.observeAbsent((x) => (x < 0.5 ? 1 : 0));
    const e = pf.estimate();
    expect(e.x).toBeGreaterThan(0.5);
    expect(pf.massNear({ x: 0.4, y: 0.5 }, 0.08)).toBeLessThan(0.05);
  });

  it('objective timer pulls junglers toward dragon', () => {
    const near = (dragonIn: number | null) => {
      const pf = new ParticleFilter(openTerrain(64), goalsFor('CHAOS'), 400, rng(3));
      pf.reset({ x: 0.6, y: 0.5 }, { role: 'JUNGLE', gameTime: 900, dragonIn, baronIn: null, likelyRecall: false });
      for (let i = 0; i < 20; i++) pf.step(1, 345);
      return pf.massNear({ x: 0.67, y: 0.70 }, 0.08);
    };
    expect(near(20)).toBeGreaterThan(near(null));
  });
});

describe('proven recall and probable Flash', () => {
  it('an item purchase while in fog puts the enemy in its fountain', () => {
    const tr = new EnemyTracker(openTerrain(64), new Learner(null), 100);
    let t = 500;
    const e = (gold: number) => [{ ...ww, itemGold: gold }];
    for (let i = 0; i < 3; i++, t += 0.1) tr.update(frame(t, [det(0.3, 0.6)], { enemies: e(2000) }));
    for (let i = 0; i < 40; i++, t += 0.1) tr.update(frame(t, [], { enemies: e(2000) })); // walks into fog
    tr.update(frame(t, [], { enemies: e(3300) })); // bought an item -> was in base
    const w = tr.all[0]!;
    expect(w.recalledAt).toBeCloseTo(t, 3);
    expect(w.estimate!.x).toBeGreaterThan(0.85);
    expect(w.estimate!.y).toBeLessThan(0.15);
  });

  it('a jump longer than walking but within Flash range is flagged', () => {
    const tr = new EnemyTracker(openTerrain(64), new Learner(null), 50);
    let t = 600;
    const e = [{ ...ww, hasFlash: true }];
    for (let i = 0; i < 3; i++, t += 0.1) tr.update(frame(t, [det(0.5, 0.5)], { enemies: e }));
    tr.update(frame(t, [det(0.525, 0.5)], { enemies: e })); // ~370 units in 0.1 s
    expect(tr.all[0]!.flashSuspectAt).toBeCloseTo(t, 3);
  });
});
