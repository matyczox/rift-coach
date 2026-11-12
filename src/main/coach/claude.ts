import { spawn, execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';

/**
 * Runs Claude Code headless (`claude -p`) with the user's subscription login.
 * No tools, no MCP, no project settings: a pure prompt -> JSON call.
 */

export interface ClaudeCallResult<T> { value: T; latencyMs: number; raw: unknown }

// Spawn the native binary directly (no shell): safe argument quoting on Windows.
function resolveBin(): string {
  const candidates = [
    process.env.APPDATA && path.join(process.env.APPDATA, 'npm', 'node_modules', '@anthropic-ai', 'claude-code', 'bin', 'claude.exe'),
    process.env.USERPROFILE && path.join(process.env.USERPROFILE, '.local', 'bin', 'claude.exe'),
  ].filter(Boolean) as string[];
  return candidates.find(p => existsSync(p)) ?? 'claude';
}
const BIN = resolveBin();

export function claudeAuthStatus(): Promise<{ loggedIn: boolean; error?: string }> {
  return new Promise(resolve => {
    execFile(BIN, ['auth', 'status'], { windowsHide: true, timeout: 15000 }, (err, out) => {
      if (err && !out) return resolve({ loggedIn: false, error: err.code === 'ENOENT' ? 'Claude Code nie jest zainstalowany' : String(err.message) });
      try { resolve({ loggedIn: Boolean(JSON.parse(out).loggedIn) }); } catch { resolve({ loggedIn: false, error: 'nieznana odpowiedź' }); }
    });
  });
}

export function callClaude<T>(opts: {
  model: string;
  system: string;
  prompt: string;
  schema: object;
  timeoutMs?: number;
  /** Built-in tools to allow (e.g. WebSearch, WebFetch). Default: none. */
  tools?: string[];
  /** Extended-thinking budget. 0 = off (fast in-game calls); undefined = CLI default. */
  thinkingTokens?: number;
  effort?: 'low' | 'medium' | 'high' | 'xhigh' | 'max';
}): Promise<ClaudeCallResult<T>> {
  const started = Date.now();
  const args = [
    '-p',
    '--model', opts.model,
    '--output-format', 'json',
    ...(opts.effort ? ['--effort', opts.effort] : []),
    // Structured-output mode costs an extra round trip (~10 s); we ask for raw JSON and validate ourselves.
    '--system-prompt', `${opts.system}

Odpowiedz WYŁĄCZNIE jednym obiektem JSON zgodnym z tym schematem, bez komentarzy i bez bloku kodu:
${JSON.stringify(opts.schema)}`,
    '--tools', (opts.tools ?? []).join(','),
    ...(opts.tools?.length ? ['--allowedTools', ...opts.tools] : []),
    '--setting-sources', '',
    '--strict-mcp-config',
    '--no-session-persistence',
  ];
  return new Promise((resolve, reject) => {
    // Prompt goes through stdin: no command-line length/quoting issues.
    const env = { ...process.env, ...(opts.thinkingTokens !== undefined ? { MAX_THINKING_TOKENS: String(opts.thinkingTokens) } : {}) };
    const child = spawn(BIN, args, { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], env });
    let out = '', err = '';
    const timer = setTimeout(() => { child.kill(); reject(new Error(`Claude: przekroczony czas ${err.slice(-200)} ${out.slice(0, 200)}`)); }, opts.timeoutMs ?? 45000);
    child.stdout.on('data', c => { out += c; });
    child.stderr.on('data', c => { err += c; });
    child.on('error', e => { clearTimeout(timer); reject(e); });
    child.on('close', () => {
      clearTimeout(timer);
      try {
        const j = JSON.parse(out) as { is_error?: boolean; result?: string; structured_output?: unknown };
        if (j.is_error) return reject(new Error(`Claude: ${j.result ?? 'błąd'}`));
        const value = (j.structured_output ?? JSON.parse(extractJson(j.result ?? ''))) as T;
        if (process.env.RC_USAGE) { const u = (j as { usage?: Record<string, number> }).usage ?? {}; console.log('USAGE', JSON.stringify({ in: u.input_tokens, cacheRead: u.cache_read_input_tokens, cacheWrite: u.cache_creation_input_tokens, out: u.output_tokens, ms: Date.now() - started })); }
        resolve({ value, latencyMs: Date.now() - started, raw: j });
      } catch (e) {
        reject(new Error(`Claude: nieczytelna odpowiedź (${String(e).slice(0, 80)}) ${err.slice(0, 160)}`));
      }
    });
    child.stdin.end(opts.prompt);
  });
}

/** First top-level JSON object in the text (tolerates code fences or stray words). */
export function extractJson(s: string): string {
  const start = s.indexOf('{'), end = s.lastIndexOf('}');
  if (start < 0 || end <= start) throw new Error('no JSON object');
  return s.slice(start, end + 1);
}
