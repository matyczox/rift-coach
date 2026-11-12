import { execFile, spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { callClaude, extractJson, type ClaudeCallResult } from './claude';

/**
 * One entry point for every coach model:
 *   claude-*            -> Claude Code (`claude -p`, your Claude subscription)
 *   gpt-* / codex:*     -> Codex CLI (`codex exec`, your ChatGPT/Codex subscription)
 */
export type Effort = 'low' | 'medium' | 'high' | 'xhigh' | 'max';

export interface ModelCall {
  model: string;
  system: string;
  prompt: string;
  schema: object;
  timeoutMs?: number;
  tools?: string[];
  thinkingTokens?: number;
  effort?: Effort;
}

export const isCodexModel = (m: string) => m.startsWith('gpt-') || m.startsWith('codex:');

export function callModel<T>(opts: ModelCall): Promise<ClaudeCallResult<T>> {
  return isCodexModel(opts.model) ? callCodex<T>(opts) : callClaude<T>(opts);
}

let codexPath: Promise<string | null> | null = null;

/** codex.exe ships inside the Codex desktop app (Microsoft Store package); find it once. */
export function resolveCodex(): Promise<string | null> {
  codexPath ??= new Promise(resolve => {
    if (process.env.CODEX_EXE && existsSync(process.env.CODEX_EXE)) return resolve(process.env.CODEX_EXE);
    execFile('powershell.exe', ['-NoProfile', '-Command', "(Get-AppxPackage OpenAI.Codex | Sort-Object Version -Descending | Select-Object -First 1).InstallLocation"], { windowsHide: true, timeout: 15000 }, (err, out) => {
      const dir = out?.trim();
      const exe = dir ? path.join(dir, 'app', 'resources', 'codex.exe') : '';
      resolve(!err && exe && existsSync(exe) ? exe : null);
    });
  });
  return codexPath;
}

/**
 * Codex non-interactive call. No tools: read-only sandbox, ephemeral session, user
 * config ignored (no MCP/notify hooks), answer written to a temp file by -o.
 */
/** Models called in Codex fast mode by default (service_tier="fast"; Terra advertises support for it). */
export const FAST_TIER_MODELS = ['gpt-6.1-terra'];

async function callCodex<T>(opts: ModelCall): Promise<ClaudeCallResult<T>> {
  const exe = await resolveCodex();
  if (!exe) throw new Error('Codex nie jest zainstalowany (aplikacja Codex z Microsoft Store)');
  const model = opts.model.replace(/^codex:/, '');
  const dir = mkdtempSync(path.join(tmpdir(), 'rc-codex-'));
  const out = path.join(dir, 'answer.txt');
  const started = Date.now();
  const effort = opts.effort ?? 'low';
  const args = [
    'exec', '-m', model, '-c', `model_reasoning_effort="${effort}"`,
    // Web research (patch notes, matchup stats): Codex's native web_search tool.
    ...(opts.tools?.some(t => /Web/i.test(t)) ? ['-c', 'tools.web_search=true'] : []),
    // Codex fast mode (service tier) for the in-game model: lower latency, uses more of the plan's quota.
    ...(FAST_TIER_MODELS.includes(model) ? ['-c', 'service_tier="fast"'] : []),
    '-s', 'read-only', '--skip-git-repo-check', '--ephemeral', '--ignore-user-config', '--color', 'never',
    '-C', dir, '-o', out, '-',
  ];
  const prompt = `${opts.system}\n\nNie uruchamiaj poleceń powłoki ani nie zmieniaj plików${opts.tools?.length ? '; możesz korzystać z wyszukiwania w internecie' : ' ani narzędzi'}. Odpowiedz WYŁĄCZNIE jednym obiektem JSON zgodnym ze schematem:\n${JSON.stringify(opts.schema)}\n\n---\n${opts.prompt}`;
  try {
    await new Promise<void>((resolve, reject) => {
      const child = spawn(exe, args, { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
      let err = '';
      const timer = setTimeout(() => { child.kill(); reject(new Error('Codex: przekroczony czas')); }, opts.timeoutMs ?? 60000);
      child.stderr.on('data', c => { err += c; });
      child.stdout.on('data', () => { /* progress events */ });
      child.on('error', e => { clearTimeout(timer); reject(e); });
      child.on('close', code => { clearTimeout(timer); code === 0 ? resolve() : reject(new Error(`Codex: kod ${code} ${err.slice(-200)}`)); });
      child.stdin.end(prompt);
    });
    const text = readFileSync(out, 'utf8');
    return { value: JSON.parse(extractJson(text)) as T, latencyMs: Date.now() - started, raw: text };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
