// Dev check: bisect which part of the coach call hangs.
import { callClaude } from '../../src/main/coach/claude';
import { SYSTEM, SCHEMA } from '../../src/main/coach/coach';

const mode = process.argv[2] ?? 'full';
const system = mode === 'nosystem' ? 'Coach LoL.' : SYSTEM;
const schema = mode === 'noschema' ? { type: 'object', properties: { call: { type: 'string' } } } : SCHEMA;
const t = Date.now();
callClaude<unknown>({ model: 'haiku', system, prompt: 'Stan meczu: 12:30, równo w złocie, smok za 40 s. Daj poradę.', schema, timeoutMs: 40000 }).then(
  r => console.log('OK', mode, Date.now() - t, JSON.stringify(r.value).slice(0, 300)),
  e => console.log('ERR', mode, Date.now() - t, String(e).slice(0, 400)),
);
