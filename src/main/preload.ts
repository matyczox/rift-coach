import { contextBridge, ipcRenderer } from 'electron';
import { INVOKE, EVENT, CAPTURE } from '../shared/ipc';

const api: Record<string, unknown> = {};
for (const ch of INVOKE) api[ch] = (...args: unknown[]) => ipcRenderer.invoke(ch, ...args);

const subscribe = (channel: string) => (cb: (v: unknown) => void) => {
  const h = (_e: unknown, v: unknown) => cb(v);
  ipcRenderer.on(channel, h);
  return () => { ipcRenderer.removeListener(channel, h); };
};
api.onStatus = subscribe(EVENT.status);
api.onMinimapFrame = subscribe(EVENT.minimapFrame);
api.onOverlay = subscribe(EVENT.overlay);
api.onReview = subscribe(EVENT.review);

contextBridge.exposeInMainWorld('rc', api);

// Capture renderer bridge (only used by the hidden capture window).
contextBridge.exposeInMainWorld('rcCapture', {
  on: (ch: string, cb: (v: unknown) => void) => {
    if (!Object.values(CAPTURE).includes(ch as never)) return;
    ipcRenderer.on(ch, (_e, v) => cb(v));
  },
  send: (ch: string, v: unknown) => {
    if (Object.values(CAPTURE).includes(ch as never)) ipcRenderer.send(ch, v);
  },
});
