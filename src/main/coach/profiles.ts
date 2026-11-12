import { claudeAuthStatus } from './claude';
import { isCodexModel, resolveCodex, type Effort } from './llm';

/**
 * Model profiles for different setups. The app checks at startup what this PC has
 * (Claude Code logged in? Codex app installed?) and, if a configured model's backend
 * is missing, falls back to the profile of the backend that IS available.
 * Picks come from the model × reasoning benchmark (docs/MODELE.md).
 */
export type Task = 'live' | 'death' | 'review' | 'build' | 'research' | 'draft';
export interface Pick { model: string; effort: Effort }
export type Profile = Record<Task, Pick>;

export const PROFILES: Record<'mixed' | 'claude' | 'codex', Profile> = {
  // Both installed: fastest accurate in-game model + Opus where there is time to read.
  mixed: {
    live: { model: 'gpt-6.1-terra', effort: 'high' },
    death: { model: 'claude-opus-5-5', effort: 'low' },
    review: { model: 'claude-opus-5-5', effort: 'medium' },
    build: { model: 'gpt-6.1-terra', effort: 'high' },
    research: { model: 'claude-opus-5-5', effort: 'low' },
    draft: { model: 'gpt-6.1-terra', effort: 'high' },
  },
  // Claude Code only (Claude subscription).
  claude: {
    live: { model: 'claude-sonnet-5-5', effort: 'medium' },
    death: { model: 'claude-opus-5-5', effort: 'low' },
    review: { model: 'claude-opus-5-5', effort: 'medium' },
    build: { model: 'claude-sonnet-5-5', effort: 'medium' },
    research: { model: 'claude-opus-5-5', effort: 'low' },
    draft: { model: 'claude-sonnet-5-5', effort: 'medium' },
  },
  // Codex only (ChatGPT subscription).
  codex: {
    live: { model: 'gpt-6.1-terra', effort: 'high' },
    death: { model: 'gpt-6.1-sol', effort: 'low' },
    review: { model: 'gpt-6.1-sol', effort: 'medium' },
    build: { model: 'gpt-6.1-terra', effort: 'high' },
    research: { model: 'gpt-6.1-terra', effort: 'high' },
    draft: { model: 'gpt-6.1-terra', effort: 'high' },
  },
};

export interface Backends { claude: boolean; codex: boolean }

export async function detectBackends(): Promise<Backends> {
  const [c, x] = await Promise.all([claudeAuthStatus().then(a => a.loggedIn).catch(() => false), resolveCodex().then(p => Boolean(p)).catch(() => false)]);
  return { claude: c, codex: x };
}

export function profileFor(b: Backends): Profile | null {
  if (b.claude && b.codex) return PROFILES.mixed;
  if (b.claude) return PROFILES.claude;
  if (b.codex) return PROFILES.codex;
  return null;
}

/** Keep the configured model if its backend exists; otherwise use the available profile's pick. */
export function usableModel(configured: string, task: Task, b: Backends): Pick | null {
  const ok = isCodexModel(configured) ? b.codex : b.claude;
  if (ok) {
    // Keep the configured model; use the effort the benchmark found best for that model and task.
    const same = Object.values(PROFILES).map(pr => pr[task]).find(x => x.model === configured);
    return { model: configured, effort: same?.effort ?? 'low' };
  }
  return profileFor(b)?.[task] ?? null;
}
