import type { Effort } from './llm';

// Models (+ reasoning effort) used for web research. Set from the active profile at runtime.
//  - deep (patch brief, lane matchup): once per patch / game -> quality first.
//  - fast (runes, draft roles): inside champion select -> speed first.
let deep = { model: process.env.RC_RESEARCH_MODEL ?? 'claude-opus-5-5', effort: 'low' as Effort };
let fast = { model: process.env.RC_RESEARCH_MODEL ?? 'gpt-6.1-terra', effort: 'high' as Effort };
export const researchModel = () => deep.model;
export const researchEffort = () => deep.effort;
export const fastResearchModel = () => fast.model;
export const fastResearchEffort = () => fast.effort;
export function setResearchModel(m: string, effort?: Effort) { if (m) deep = { model: m, effort: effort ?? deep.effort }; }
export function setFastResearchModel(m: string, effort?: Effort) { if (m) fast = { model: m, effort: effort ?? fast.effort }; }
