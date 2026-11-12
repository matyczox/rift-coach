import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';

// JSON-file cache: <dir>/<namespace>/<key>.json containing { savedAt, data }.
// TTL is decided by the reader, so immutable data (matches, timelines) is
// simply read with ttl = null (never expires). A small in-memory layer avoids
// re-reading big timeline files within one session. All I/O errors are
// swallowed: the cache is an optimisation, never a failure source.

interface Entry<T> { savedAt: number; data: T }

const MEM_MAX = 400;

export class DiskCache {
  private mem = new Map<string, Entry<unknown>>();

  constructor(private readonly dir: string | null, private readonly now: () => number = Date.now) {}

  /** Returns undefined on miss or expiry. */
  async get<T>(ns: string, key: string, ttlMs: number | null): Promise<T | undefined> {
    const id = `${ns}/${key}`;
    let e = this.mem.get(id) as Entry<T> | undefined;
    if (!e && this.dir) {
      try {
        e = JSON.parse(await readFile(this.file(ns, key), 'utf8')) as Entry<T>;
        if (e && typeof e.savedAt === 'number') this.remember(id, e);
        else e = undefined;
      } catch { e = undefined; }
    }
    if (!e) return undefined;
    if (ttlMs !== null && this.now() - e.savedAt > ttlMs) return undefined;
    return e.data;
  }

  async set<T>(ns: string, key: string, data: T): Promise<void> {
    const e: Entry<T> = { savedAt: this.now(), data };
    this.remember(`${ns}/${key}`, e);
    if (!this.dir) return;
    try {
      const f = this.file(ns, key);
      await mkdir(path.dirname(f), { recursive: true });
      const tmp = `${f}.${process.pid}.${Math.random().toString(36).slice(2)}.tmp`;
      await writeFile(tmp, JSON.stringify(e));
      await rename(tmp, f);
    } catch { /* ignore */ }
  }

  private remember(id: string, e: Entry<unknown>): void {
    this.mem.delete(id);
    this.mem.set(id, e);
    if (this.mem.size > MEM_MAX) this.mem.delete(this.mem.keys().next().value!);
  }

  private file(ns: string, key: string): string {
    // ns is an internal constant ("match", "timeline", ...)
    return path.join(this.dir!, ns.replace(/[^a-z0-9-]/gi, '_'), `${safeName(key)}.json`);
  }
}

/**
 * File-system safe, collision-free name. A hash suffix is added whenever the
 * string was altered or contains lowercase letters (puuids are case-sensitive,
 * Windows file names are not).
 */
export function safeName(s: string): string {
  const clean = s.replace(/[^A-Za-z0-9_.-]/g, '_').slice(0, 80);
  if (clean === s && s.length > 0 && !s.startsWith('.') && !/[a-z]/.test(s)) return s;
  return `${clean}-${createHash('sha1').update(s).digest('hex').slice(0, 12)}`;
}
