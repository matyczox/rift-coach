// Rift Coach (Tauri): window.rc bridge + own title bar controls.
// Injected into every page before its scripts run. Replaces the Electron preload
// (src/main/preload.ts) without touching the React UI in src/.
(function () {
  'use strict';
  if (window.rc) return;

  const tauri = () => {
    const t = window.__TAURI__;
    if (!t) throw new Error('Brak połączenia z aplikacją (Tauri).');
    return t;
  };
  const asError = (e) => (e instanceof Error ? e : new Error(typeof e === 'string' ? e : (e && e.message) || String(e)));

  // Same method list as src/shared/ipc.ts INVOKE: every call goes to one Rust command.
  const INVOKE = [
    'getStatus', 'updateSettings', 'grabScene', 'autoCalibrate', 'setCalibration',
    'startRecording', 'stopRecording', 'listRecordings', 'deleteRecording', 'openRecordingsFolder',
    'replayOpen', 'replayPlay', 'replayPause', 'replaySeek', 'replaySpeed', 'replayClose',
    'listReplayFiles', 'harvestStart', 'harvestStop', 'harvestContinue',
    'listReviews',
  ];
  const api = {};
  for (const method of INVOKE) {
    api[method] = (...args) => {
      try {
        return tauri().core.invoke('rc_call', { method, args }).catch((e) => { throw asError(e); });
      } catch (e) {
        return Promise.reject(asError(e));
      }
    };
  }

  const subscribe = (event) => (cb) => {
    let unlisten = null;
    let dead = false;
    try {
      tauri().event.listen(event, (e) => cb(e.payload)).then((u) => { if (dead) u(); else unlisten = u; }, () => undefined);
    } catch { /* no bridge: nothing to listen to */ }
    return () => { dead = true; if (unlisten) unlisten(); };
  };
  api.onStatus = subscribe('status');
  api.onMinimapFrame = subscribe('minimapFrame');
  api.onOverlay = subscribe('overlay');
  api.onReview = subscribe('review');

  Object.defineProperty(window, 'rc', { value: Object.freeze(api), writable: false, configurable: false });

  // ---------- title bar ----------
  const win = () => tauri().window.getCurrentWindow();
  const DRAG = '.topbar, .brand, [data-rc-drag]';
  const NO_DRAG = 'button, a, input, select, textarea, label, [role="button"], .sys, .coach-badge, #rc-wincontrols';
  document.addEventListener('mousedown', (e) => {
    if (e.button !== 0 || !(e.target instanceof Element)) return;
    if (!e.target.closest(DRAG) || e.target.closest(NO_DRAG)) return;
    e.preventDefault();
    try {
      if (e.detail === 2) void win().toggleMaximize();
      else void win().startDragging();
    } catch { /* ignore */ }
  }, true);

  // External https links open in the default browser (Electron's setWindowOpenHandler).
  const openExternal = (url) => {
    if (!/^https:\/\//.test(String(url))) return;
    try { void tauri().core.invoke('open_external', { url: String(url) }); } catch { /* ignore */ }
  };
  window.open = (url) => { openExternal(url); return null; };
  document.addEventListener('click', (e) => {
    const a = e.target instanceof Element ? e.target.closest('a[href]') : null;
    if (a && /^https:\/\//.test(a.href)) { e.preventDefault(); openExternal(a.href); }
  }, true);

  // Native min/max/close are drawn by Electron's titleBarOverlay; here we draw them ourselves
  // in the 152 px the UI keeps free on the right of the 44 px title bar.
  const ICONS = {
    min: '<svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" stroke-width="1"/></svg>',
    max: '<svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" stroke-width="1"/></svg>',
    close: '<svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" stroke-width="1.1"/></svg>',
  };
  function addControls() {
    if (!document.body || document.getElementById('rc-wincontrols')) return;
    const box = document.createElement('div');
    box.id = 'rc-wincontrols';
    Object.assign(box.style, { position: 'fixed', top: '0', right: '0', height: '44px', display: 'flex', zIndex: '2147483647' });
    const mk = (kind, title, action, hoverBg) => {
      const b = document.createElement('button');
      b.type = 'button';
      b.title = title;
      b.innerHTML = ICONS[kind];
      Object.assign(b.style, {
        width: '46px', height: '44px', border: '0', margin: '0', padding: '0', display: 'flex',
        alignItems: 'center', justifyContent: 'center', background: 'transparent', color: '#9aa3b2', cursor: 'default',
      });
      b.addEventListener('mouseenter', () => { b.style.background = hoverBg; b.style.color = '#ffffff'; });
      b.addEventListener('mouseleave', () => { b.style.background = 'transparent'; b.style.color = '#9aa3b2'; });
      b.addEventListener('click', (e) => { e.stopPropagation(); try { void action(); } catch { /* ignore */ } });
      box.appendChild(b);
    };
    mk('min', 'Minimalizuj', () => win().minimize(), 'rgba(255,255,255,0.08)');
    mk('max', 'Maksymalizuj', () => win().toggleMaximize(), 'rgba(255,255,255,0.08)');
    mk('close', 'Zamknij', () => win().close(), '#e81123');
    document.body.appendChild(box);
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', addControls);
  else addControls();
})();
