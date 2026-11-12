// Entry of the single HUD window (hud-all.html, Tauri). all-bridge.ts must be imported first;
// the panel modules follow in the order of KINDS.
import { dispatch, type HudMessage } from './all-bridge';
import './minimap';
import './topbar';
import './death';
import './loading';
import './power';
import './build';

interface TauriEvent<T> { payload: T }
interface TauriGlobal { event: { listen<T>(name: string, cb: (e: TauriEvent<T>) => void): Promise<() => void> } }

const tauri = (window as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;
let last: HudMessage | null = null;
void tauri?.event.listen<HudMessage>('hud', (e) => { last = e.payload; dispatch(e.payload); });
// The game window can be resized while nothing is pushed: re-run the layout.
window.addEventListener('resize', (e) => { if (e.isTrusted && last) dispatch(last); });
