/**
 * Terrain interface used by the predictor. The real implementation comes from
 * the minimap walkable grid (src/main/vision/terrain.ts); `OpenTerrain` is a
 * fallback (everything inside the map square is walkable) for tests and until
 * the grid is available.
 */
export interface Terrain {
  /** Grid resolution (cells per side). */
  readonly size: number;
  walkable(x: number, y: number): boolean;
  /** Geodesic distance field (normalized map units) from a goal; Infinity where unreachable. */
  fieldTo(goal: { x: number; y: number }): Float32Array;
}

export class GridTerrain implements Terrain {
  private cache = new Map<string, Float32Array>();
  constructor(readonly size: number, private grid: Uint8Array) {}

  walkable(x: number, y: number): boolean {
    const gx = Math.floor(x * this.size), gy = Math.floor(y * this.size);
    if (gx < 0 || gy < 0 || gx >= this.size || gy >= this.size) return false;
    return this.grid[gy * this.size + gx] === 1;
  }

  fieldTo(goal: { x: number; y: number }): Float32Array {
    const key = `${goal.x.toFixed(3)},${goal.y.toFixed(3)}`;
    const hit = this.cache.get(key);
    if (hit) return hit;
    const n = this.size, dist = new Float32Array(n * n).fill(Infinity);
    // Start from the nearest walkable cell to the goal.
    let sx = Math.min(n - 1, Math.max(0, Math.floor(goal.x * n))), sy = Math.min(n - 1, Math.max(0, Math.floor(goal.y * n)));
    if (this.grid[sy * n + sx] !== 1) {
      let best = Infinity;
      for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
        if (this.grid[y * n + x] !== 1) continue;
        const d = (x - sx) ** 2 + (y - sy) ** 2;
        if (d < best) { best = d; sx = x; sy = y; }
      }
    }
    // Dijkstra-lite: 8-connected with a bucketed queue (costs 1 and √2).
    const step = 1 / n;
    dist[sy * n + sx] = 0;
    const queue: number[] = [sy * n + sx];
    const nb: Array<[number, number, number]> = [[1, 0, 1], [-1, 0, 1], [0, 1, 1], [0, -1, 1], [1, 1, Math.SQRT2], [1, -1, Math.SQRT2], [-1, 1, Math.SQRT2], [-1, -1, Math.SQRT2]];
    for (let qi = 0; qi < queue.length; qi++) {
      const c = queue[qi]!, cx = c % n, cy = (c - cx) / n, cd = dist[c]!;
      for (const [dx, dy, w] of nb) {
        const x = cx + dx, y = cy + dy;
        if (x < 0 || y < 0 || x >= n || y >= n) continue;
        const j = y * n + x;
        if (this.grid[j] !== 1) continue;
        const nd = cd + w * step;
        if (nd < dist[j]! - 1e-9) { dist[j] = nd; queue.push(j); }
      }
    }
    this.cache.set(key, dist);
    return dist;
  }
}

/** Everything inside the unit square walkable (tests / fallback). */
export function openTerrain(size = 64): GridTerrain {
  return new GridTerrain(size, new Uint8Array(size * size).fill(1));
}

/** Sample a distance field at a normalized point. */
export function sampleField(t: Terrain, field: Float32Array, x: number, y: number): number {
  const gx = Math.min(t.size - 1, Math.max(0, Math.floor(x * t.size))), gy = Math.min(t.size - 1, Math.max(0, Math.floor(y * t.size)));
  return field[gy * t.size + gx]!;
}
