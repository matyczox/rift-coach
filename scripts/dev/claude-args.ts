// Dev check: does callClaude work with prompts of a given size / with the real coach system prompt?
import { callClaude } from '../../src/main/coach/claude';

const n = Number(process.argv[2] ?? 100);
const big = 'x'.repeat(n);
const t = Date.now();
callClaude<{ call: string }>({
  model: 'haiku',
  system: 'Coach LoL. ' + big,
  prompt: 'Stan: ' + big,
  schema: { type: 'object', properties: { call: { type: 'string' } } },
  timeoutMs: 40000,
}).then(
  r => console.log('OK', n, Date.now() - t, JSON.stringify(r.value)),
  e => console.log('ERR', n, Date.now() - t, String(e).slice(0, 400)),
);
