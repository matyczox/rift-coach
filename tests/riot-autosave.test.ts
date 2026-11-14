import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRiotAutosave, type RiotConfig } from '../src/renderer/control/riotAutosave';

const initial: RiotConfig = { apiKey: '', platform: 'eun1' };
const events = () => ({ state: vi.fn(), saved: vi.fn(), error: vi.fn() });
describe('Riot key autosave', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  it('does not write unchanged settings on mount', async () => {
    const save=vi.fn(async (value:RiotConfig)=>value);
    const c=createRiotAutosave(initial,save,events());
    c.update(initial);
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).not.toHaveBeenCalled();
  });
  it('activates a pasted key after the debounce and removes copied whitespace', async () => {
    const save=vi.fn(async (value:RiotConfig)=>value);const e=events();
    const c=createRiotAutosave(initial,save,e);
    c.update({apiKey:' \nRGAPI-test-key\t ',platform:'eun1'});
    await vi.advanceTimersByTimeAsync(600);
    expect(save).toHaveBeenCalledExactlyOnceWith({apiKey:'RGAPI-test-key',platform:'eun1'});
    expect(e.state).toHaveBeenLastCalledWith('saved');
  });
  it('saves the latest edit rather than every partial key', async () => {
    const save=vi.fn(async (value:RiotConfig)=>value);
    const c=createRiotAutosave(initial,save,events());
    c.update({apiKey:'partial',platform:'eun1'});
    vi.advanceTimersByTime(300);
    c.update({apiKey:'complete',platform:'euw1'});
    await vi.advanceTimersByTimeAsync(600);
    expect(save).toHaveBeenCalledExactlyOnceWith({apiKey:'complete',platform:'euw1'});
  });
  it('a delayed old save cannot restore a key the player has removed', async () => {
    let finish!:(value:RiotConfig)=>void;
    const first=new Promise<RiotConfig>((resolve)=>{finish=resolve;});
    const save=vi.fn().mockImplementationOnce(()=>first).mockImplementation(async (v:RiotConfig)=>v);
    const e=events();const c=createRiotAutosave(initial,save,e);
    c.update({apiKey:'old-key',platform:'eun1'});
    vi.advanceTimersByTime(600);await Promise.resolve();
    expect(save).toHaveBeenCalledTimes(1);
    c.update(initial);
    finish({apiKey:'old-key',platform:'eun1'});
    await vi.advanceTimersByTimeAsync(600);
    expect(save).toHaveBeenLastCalledWith(initial);
    expect(e.saved).toHaveBeenCalledExactlyOnceWith(initial,initial);
  });
  it('finishes a paste when the settings tab is immediately closed', async () => {
    const save=vi.fn(async (v:RiotConfig)=>v);const e=events();
    const c=createRiotAutosave(initial,save,e);
    c.update({apiKey:'new-key',platform:'eun1'});c.dispose();await c.flush();
    expect(save).toHaveBeenCalledTimes(1);
    expect(e.saved).not.toHaveBeenCalled();
  });
  it('reports a failed save and permits retry of the same value', async () => {
    const save=vi.fn().mockRejectedValueOnce(new Error('storage unavailable')).mockImplementation(async (v:RiotConfig)=>v);
    const e=events();const c=createRiotAutosave(initial,save,e);
    c.update({apiKey:'new-key',platform:'eun1'});await c.flush();
    expect(e.error).toHaveBeenCalledTimes(1);
    c.update({apiKey:'new-key',platform:'eun1'});await c.flush();
    expect(save).toHaveBeenCalledTimes(2);
    expect(e.saved).toHaveBeenCalledTimes(1);
  });
});
