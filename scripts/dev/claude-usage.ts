// Dev check: raw claude -p call with the real coach prompt; prints timing and token usage.
import { spawn } from 'node:child_process';
import path from 'node:path';
import { SYSTEM, SCHEMA } from '../../src/main/coach/coach';

const BIN = path.join(process.env.APPDATA!, 'npm', 'node_modules', '@anthropic-ai', 'claude-code', 'bin', 'claude.exe');
const effort = process.argv[2] ?? 'low';
const model = process.argv[3] ?? 'haiku';
const system = `${SYSTEM}\n\nOdpowiedz WYŁĄCZNIE jednym obiektem JSON zgodnym ze schematem:\n${JSON.stringify(SCHEMA)}`;
const args = ['-p', '--model', model, '--effort', effort, '--output-format', 'json', '--system-prompt', system, '--tools', '', '--setting-sources', '', '--strict-mcp-config', '--no-session-persistence'];
const t = Date.now();
const c = spawn(BIN, args, { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
let out = '', err = '';
c.stdout.on('data', d => { out += d; });
c.stderr.on('data', d => { err += d; });
const k = setTimeout(() => { console.log('TIMEOUT', Date.now() - t, err.slice(-300)); c.kill(); }, 120000);
c.on('close', () => {
  clearTimeout(k);
  try {
    const j = JSON.parse(out);
    console.log(effort, model, 'wall', Date.now() - t, 'api', j.duration_api_ms, 'turns', j.num_turns, 'in', j.usage.input_tokens, 'out', j.usage.output_tokens);
    console.log(j.result);
  } catch { console.log('RAW', out.slice(0, 500), err.slice(0, 300)); }
});
c.stdin.end('Stan meczu: 12:30, równo w złocie, smok za 40 s, jungler wroga niewidoczny 20 s. Daj poradę.');
