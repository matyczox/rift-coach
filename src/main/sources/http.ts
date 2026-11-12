import https from 'node:https';

// Riot's local endpoints (game :2999 and LCU) use a self-signed Riot root CA.
// We only ever talk to 127.0.0.1, so certificate checks are skipped for them.
const agent = new https.Agent({ rejectUnauthorized: false, keepAlive: true });

export function getJson<T>(url: string, headers: Record<string, string> = {}, timeoutMs = 1500): Promise<T> {
  const u = new URL(url);
  if (u.hostname !== '127.0.0.1') return Promise.reject(new Error('local endpoints only'));
  return new Promise((resolve, reject) => {
    const req = https.get(u, { agent, headers, timeout: timeoutMs }, res => {
      const chunks: Buffer[] = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => {
        const body = Buffer.concat(chunks).toString('utf8');
        if ((res.statusCode ?? 500) >= 400) return reject(new Error(`HTTP ${res.statusCode}`));
        try { resolve(JSON.parse(body) as T); } catch (e) { reject(e); }
      });
    });
    req.on('timeout', () => req.destroy(new Error('timeout')));
    req.on('error', reject);
  });
}

/** POST JSON to a local endpoint; resolves with parsed body (or null on 204). */
export function postJson<T>(url: string, body: unknown, headers: Record<string, string> = {}, timeoutMs = 3000, method = 'POST'): Promise<T | null> {
  const u = new URL(url);
  if (u.hostname !== '127.0.0.1') return Promise.reject(new Error('local endpoints only'));
  const data = JSON.stringify(body);
  return new Promise((resolve, reject) => {
    const req = https.request(u, { agent, method, timeout: timeoutMs, headers: { ...headers, 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(data) } }, res => {
      const chunks: Buffer[] = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => {
        if ((res.statusCode ?? 500) >= 400) return reject(new Error(`HTTP ${res.statusCode}`));
        const text = Buffer.concat(chunks).toString('utf8');
        try { resolve(text ? JSON.parse(text) as T : null); } catch (e) { reject(e); }
      });
    });
    req.on('timeout', () => req.destroy(new Error('timeout')));
    req.on('error', reject);
    req.end(data);
  });
}
