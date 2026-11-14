export interface RiotConfig { apiKey: string; platform: string }
export const cleanRiotConfig = (v: RiotConfig): RiotConfig => ({ apiKey: v.apiKey.replace(/\s+/g, ''), platform: v.platform });
export const sameRiotConfig = (a: RiotConfig, b: RiotConfig) => a.apiKey === b.apiKey && a.platform === b.platform;

/** Debounced, ordered writes: an older response cannot overwrite a newer edit. */
export function createRiotAutosave(initial: RiotConfig, save: (v: RiotConfig) => Promise<RiotConfig>, events: {
  state: (state: 'pending' | 'saving' | 'saved') => void;
  saved: (value: RiotConfig, submitted: RiotConfig) => void;
  error: (error: unknown) => void;
}, delay = 600) {
  let saved = cleanRiotConfig(initial);
  let latest = saved;
  let revision = 0;
  let writes = 0;
  let active = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let queue = Promise.resolve();
  const enqueue = () => {
    const rev = revision;
    const submitted = { ...latest };
    writes++;
    queue = queue.then(async () => {
      try {
        if (rev !== revision) return;
        if (!sameRiotConfig(saved, submitted)) {
          if (active) events.state('saving');
          saved = cleanRiotConfig(await save(submitted));
        }
        if (active && rev === revision) {
          events.saved(saved, submitted);
          events.state('saved');
        }
      } catch (error) {
        if (active && rev === revision) events.error(error);
      } finally { writes--; }
    });
    return queue;
  };
  const flush = () => { clearTimeout(timer); timer = undefined; return enqueue(); };
  return {
    update(value: RiotConfig) {
      if (!active) return;
      latest = cleanRiotConfig(value);
      revision++;
      clearTimeout(timer);
      if (sameRiotConfig(latest, saved) && writes === 0) return;
      events.state('pending');
      timer = setTimeout(enqueue, delay);
    },
    flush,
    // Finish a paste even if the player immediately leaves the settings tab.
    dispose() { void flush(); active = false; },
  };
}
