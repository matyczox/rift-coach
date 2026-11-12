// Preload for the in-game HUD windows (built as dist/hud-preload.js).
// Exposes window.hud.onState(cb) for the single HUD channel and nothing else.
import { contextBridge, ipcRenderer } from 'electron';
import { HUD_CHANNEL, type HudBridge, type HudPayload } from '../renderer/hud/types';

const bridge: HudBridge = {
  onState(cb) {
    const h = (_e: unknown, v: HudPayload) => cb(v);
    ipcRenderer.on(HUD_CHANNEL, h);
    return () => { ipcRenderer.removeListener(HUD_CHANNEL, h); };
  },
};

contextBridge.exposeInMainWorld('hud', bridge);
