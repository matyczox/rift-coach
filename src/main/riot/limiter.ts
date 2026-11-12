// Client-side rate limiter for the Riot dev key (20 req / 1 s and 100 req / 120 s).
//
// Each limit is a token bucket whose spent tokens come back exactly `windowMs`
// after they were spent (a "token return" bucket, equivalent to a sliding log).
// This is stricter than a continuously refilling bucket: it guarantees that no
// window of length `windowMs` — however Riot aligns its own windows — ever sees
// more than `count` requests, so we do not trip 429s at window edges.
// A small `padMs` covers clock skew / network latency between us and Riot.

export interface RateLimit { count: number; windowMs: number }

export const DEV_KEY_LIMITS: RateLimit[] = [
  { count: 20, windowMs: 1_000 },
  { count: 100, windowMs: 120_000 },
];

export class DeadlineError extends Error {
  constructor(msg = 'Riot request would exceed the deadline') { super(msg); this.name = 'DeadlineError'; }
}

export class RateLimiter {
  /** Grant timestamps, ascending. */
  private log: number[] = [];
  private pausedUntil = 0;
  private tail: Promise<void> = Promise.resolve();
  private readonly maxWindow: number;

  constructor(
    private readonly limits: RateLimit[] = DEV_KEY_LIMITS,
    private readonly now: () => number = Date.now,
    private readonly padMs = 50,
  ) {
    this.maxWindow = Math.max(0, ...limits.map(l => l.windowMs));
  }

  /** Milliseconds until the next token could be granted (0 = now). */
  delay(): number {
    const t = this.now();
    this.prune(t);
    let wait = Math.max(0, this.pausedUntil - t);
    for (const l of this.limits) {
      const inWindow = this.countSince(t - l.windowMs);
      if (inWindow >= l.count) {
        // The token we need back is the `count`-th most recent grant.
        const blocking = this.log[this.log.length - l.count]!;
        wait = Math.max(wait, blocking + l.windowMs + this.padMs - t);
      }
    }
    return wait;
  }

  /** Tokens available right now in the tightest bucket (for budgeting/inspection). */
  available(): number {
    const t = this.now();
    this.prune(t);
    if (this.pausedUntil > t) return 0;
    return Math.min(...this.limits.map(l => l.count - this.countSince(t - l.windowMs)));
  }

  /**
   * How many requests the longest-window limit can still grant by time `t`
   * (counting tokens that return before then). Used to size request budgets.
   */
  availableBy(t: number): number {
    const l = this.limits.reduce((a, b) => (b.windowMs > a.windowMs ? b : a), this.limits[0]!);
    if (!l) return Infinity;
    return Math.max(0, l.count - this.countSince(t - l.windowMs - this.padMs));
  }

  /** Block all requests for `ms` (Retry-After from a 429). */
  pause(ms: number): void {
    this.pausedUntil = Math.max(this.pausedUntil, this.now() + Math.max(0, ms));
  }

  /**
   * Wait for a token. Requests are granted FIFO. Rejects with DeadlineError
   * (without consuming a token) if the wait would end after `deadline`.
   */
  acquire(deadline?: number, signal?: AbortSignal): Promise<void> {
    const run = this.tail.then(() => this.waitTurn(deadline, signal));
    this.tail = run.catch(() => undefined);
    return run;
  }

  private async waitTurn(deadline?: number, signal?: AbortSignal): Promise<void> {
    for (;;) {
      if (signal?.aborted) throw signal.reason ?? new Error('aborted');
      const wait = this.delay();
      if (wait <= 0) { this.log.push(this.now()); return; }
      if (deadline !== undefined && this.now() + wait > deadline) throw new DeadlineError();
      await sleep(wait, signal);
    }
  }

  private countSince(t: number): number {
    // log is ascending; count entries strictly after t
    let n = 0;
    for (let i = this.log.length - 1; i >= 0 && this.log[i]! > t; i--) n++;
    return n;
  }

  private prune(t: number): void {
    const cut = t - this.maxWindow - this.padMs;
    let i = 0;
    while (i < this.log.length && this.log[i]! <= cut) i++;
    if (i) this.log.splice(0, i);
  }
}

export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) { reject(signal.reason ?? new Error('aborted')); return; }
    const onAbort = () => { clearTimeout(id); reject(signal!.reason ?? new Error('aborted')); };
    const id = setTimeout(() => { signal?.removeEventListener('abort', onAbort); resolve(); }, Math.max(0, ms));
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}
