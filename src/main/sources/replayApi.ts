import https from 'node:https';
import { getJson } from './http';

// Riot Replay API (127.0.0.1:2999/replay/*). Needs EnableReplayApi=1 in game.cfg.
export interface Playback { length: number; paused: boolean; seeking: boolean; speed: number; time: number }

const BASE = 'https://127.0.0.1:2999';
const agent = new https.Agent({ rejectUnauthorized: false, keepAlive: true });

function post<T>(path: string, body: unknown, timeoutMs = 3000): Promise<T> {
  return new Promise((resolve, reject) => {
    const data = JSON.stringify(body);
    const req = https.request(`${BASE}${path}`, {
      agent, method: 'POST', timeout: timeoutMs,
      headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(data) },
    }, res => {
      const chunks: Buffer[] = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => {
        if ((res.statusCode ?? 500) >= 400) return reject(new Error(`HTTP ${res.statusCode}`));
        const text = Buffer.concat(chunks).toString('utf8');
        try { resolve(text ? JSON.parse(text) : (null as T)); } catch (e) { reject(e); }
      });
    });
    req.on('timeout', () => req.destroy(new Error('timeout')));
    req.on('error', reject);
    req.end(data);
  });
}

export const replayApi = {
  playback: () => getJson<Playback>(`${BASE}/replay/playback`),
  setPlayback: (p: Partial<Pick<Playback, 'time' | 'paused' | 'speed'>>) => post<Playback>('/replay/playback', p),
  setRender: (r: Record<string, unknown>) => post<Record<string, unknown>>('/replay/render', r),
  game: () => getJson<{ processID: number }>(`${BASE}/replay/game`),

  /** Seek while paused and wait until the engine has landed on `time`. */
  async seek(time: number, timeoutMs = 8000): Promise<Playback> {
    await post('/replay/playback', { time, paused: true });
    const end = Date.now() + timeoutMs;
    for (;;) {
      const pb = await getJson<Playback>(`${BASE}/replay/playback`);
      if (!pb.seeking && Math.abs(pb.time - time) < 0.25) return pb;
      if (Date.now() > end) throw new Error(`seek to ${time} timed out`);
      await new Promise(r => setTimeout(r, 30));
    }
  },
};
