import { memo, useCallback, useEffect, useRef, useState } from 'react';
import type { KeyboardEvent as RKeyboardEvent, PointerEvent as RPointerEvent } from 'react';
import type { MinimapCalibration, Rect, SourceState } from '../../shared/types';
import { clamp, fmtDate, rc, useAction } from './util';
import { MinimapPreview } from './MinimapPreview';
import type { CoachAdvice } from '../../shared/intel';

interface Scene { dataUrl: string; width: number; height: number }
type DragMode = 'move' | 'nw' | 'ne' | 'sw' | 'se';
interface Drag { mode: DragMode; startGx: number; startGy: number; rect: Rect }

const MIN_SIZE = 40;

/** Square rect in game px, clamped inside a W×H frame. */
function normalize(r: Rect, W: number, H: number): Rect {
  const size = Math.round(clamp(Math.min(r.width, r.height), MIN_SIZE, Math.min(W, H)));
  return {
    x: Math.round(clamp(r.x, 0, W - size)),
    y: Math.round(clamp(r.y, 0, H - size)),
    width: size,
    height: size,
  };
}

/** Rescale a calibration rect to a frame of another size. */
function scaleCal(c: MinimapCalibration, W: number, H: number): Rect {
  const sx = W / c.windowWidth;
  const sy = H / c.windowHeight;
  return normalize({ x: c.rect.x * sx, y: c.rect.y * sy, width: c.rect.width * sx, height: c.rect.height * sy }, W, H);
}

/** Bottom-right guess used when nothing is calibrated yet. */
function defaultRect(W: number, H: number): Rect {
  const size = Math.round(H * 0.25);
  return normalize({ x: W - size - 8, y: H - size - 8, width: size, height: size }, W, H);
}

interface Props {
  calibration: MinimapCalibration | null;
  captureState: SourceState;
  lastAdvice: CoachAdvice | null;
  windowWidth: number | null;
  windowHeight: number | null;
}

export const CalibrationView = memo(function CalibrationView({ calibration, captureState, windowWidth, windowHeight, lastAdvice }: Props) {
  const [scene, setScene] = useState<Scene | null>(null);
  const [rect, setRect] = useState<Rect | null>(null);
  const [rectSource, setRectSource] = useState<'auto' | 'manual'>('manual');
  /** Window size the current rect refers to (scene size, or auto result without a scene). */
  const [frameSize, setFrameSize] = useState<{ w: number; h: number } | null>(null);
  const [dirty, setDirty] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const grab = useAction();
  const auto = useAction();
  const save = useAction();

  const calRef = useRef(calibration);
  calRef.current = calibration;

  const onGrab = useCallback(async () => {
    setNote(null);
    const r = await grab.run(() => rc().grabScene());
    if (!r.ok) return;
    if (!r.value) {
      grab.setError('Nie udało się pobrać klatki — czy gra jest uruchomiona?');
      return;
    }
    const s = r.value;
    setScene(s);
    setFrameSize({ w: s.width, h: s.height });
    const cal = calRef.current;
    setRect(cal ? scaleCal(cal, s.width, s.height) : defaultRect(s.width, s.height));
    setRectSource(cal?.source ?? 'manual');
    setDirty(!cal);
  }, [grab]);

  const onAuto = useCallback(async () => {
    setNote(null);
    const r = await auto.run(() => rc().autoCalibrate());
    if (!r.ok) return;
    const c = r.value;
    if (!c) {
      setNote('Nie znaleziono minimapy. Ustaw ramkę ręcznie.');
      return;
    }
    if (scene) {
      setRect(scaleCal(c, scene.width, scene.height));
    } else {
      setRect(normalize(c.rect, c.windowWidth, c.windowHeight));
      setFrameSize({ w: c.windowWidth, h: c.windowHeight });
    }
    setRectSource('auto');
    setDirty(true);
    setNote('Wykryto minimapę. Sprawdź ramkę i zapisz.');
  }, [auto, scene]);

  const onSave = useCallback(async () => {
    if (!rect || !frameSize) return;
    const c: MinimapCalibration = {
      rect: normalize(rect, frameSize.w, frameSize.h),
      windowWidth: frameSize.w,
      windowHeight: frameSize.h,
      source: rectSource,
      updatedAt: Date.now(),
    };
    const r = await save.run(() => rc().setCalibration(c));
    if (r.ok) { setDirty(false); setNote('Zapisano kalibrację.'); }
  }, [rect, frameSize, rectSource, save]);

  const onReset = useCallback(() => {
    if (!frameSize) return;
    const cal = calRef.current;
    setRect(cal ? scaleCal(cal, frameSize.w, frameSize.h) : defaultRect(frameSize.w, frameSize.h));
    setRectSource(cal?.source ?? 'manual');
    setDirty(false);
    setNote(null);
  }, [frameSize]);

  const updateRect = useCallback((r: Rect) => {
    setRect(r);
    setRectSource('manual');
    setDirty(true);
  }, []);

  const err = grab.error ?? auto.error ?? save.error;
  const sizeMismatch = calibration && windowWidth != null && windowHeight != null
    && (calibration.windowWidth !== windowWidth || calibration.windowHeight !== windowHeight);

  return (
    <section className="section">
      <div className="section-head">
        <h1>Kalibracja minimapy</h1>
        <div className="toolbar">
          <button type="button" className="btn" onClick={onGrab} disabled={grab.busy}>
            {grab.busy ? 'Pobieranie…' : 'Pobierz klatkę gry'}
          </button>
          <button type="button" className="btn" onClick={onAuto} disabled={auto.busy}>
            {auto.busy ? 'Szukanie…' : 'Auto'}
          </button>
          <button type="button" className="btn" onClick={onReset} disabled={!dirty || !frameSize}>Cofnij</button>
          <button type="button" className="btn primary" onClick={onSave} disabled={!rect || !frameSize || !dirty || save.busy}>
            {save.busy ? 'Zapisywanie…' : 'Zapisz'}
          </button>
        </div>
      </div>

      <div className="info-row">
        <span>
          <span className="muted">Zapisana: </span>
          {calibration
            ? `${calibration.rect.width} px w (${calibration.rect.x}, ${calibration.rect.y}) dla ${calibration.windowWidth}×${calibration.windowHeight} · ${calibration.source === 'auto' ? 'auto' : 'ręczna'} · ${fmtDate(calibration.updatedAt)}`
            : 'brak'}
        </span>
        {sizeMismatch && <span className="warn-text">Okno gry ma teraz {windowWidth}×{windowHeight} — skalibruj ponownie.</span>}
      </div>
      {err && <div className="error-text">{err}</div>}
      {note && !err && <div className="note-text">{note}</div>}

      <div className="cal-layout">
        <div className="cal-stage-wrap">
          {scene && rect ? (
            <SceneEditor scene={scene} rect={rect} onChange={updateRect} />
          ) : (
            <div className="placeholder">
              {captureState === 'connected'
                ? 'Kliknij „Pobierz klatkę gry”, aby ustawić ramkę minimapy.'
                : 'Uruchom grę, potem pobierz klatkę. Przechwytywanie: ' + (captureState === 'error' ? 'błąd' : captureState === 'waiting' ? 'czeka' : 'wyłączone') + '.'}
              {rect && frameSize && !scene && (
                <div className="mono">Wynik auto: {rect.width} px w ({rect.x}, {rect.y}) dla {frameSize.w}×{frameSize.h}</div>
              )}
            </div>
          )}
          {scene && rect && (
            <div className="cal-hint">
              Przeciągnij ramkę, aby przesunąć; narożniki zmieniają rozmiar. Strzałki: 1 px (Shift: 10 px), +/−: rozmiar.
              <span className="mono cal-readout">x {rect.x} · y {rect.y} · {rect.width} px</span>
            </div>
          )}
        </div>
        <aside className="cal-side">
          <div className="cal-card">
            <div className="cal-card-title">Podgląd na żywo</div>
            <MinimapPreview size={200} />
          </div>
          {scene && rect && (
            <div className="cal-card">
              <div className="cal-card-title">Wycinek z klatki{dirty ? ' (niezapisany)' : ''}</div>
              <CropPreview scene={scene} rect={rect} size={200} />
            </div>
          )}
        </aside>
      </div>
    </section>
  );
});

function SceneEditor({ scene, rect, onChange }: { scene: Scene; rect: Rect; onChange: (r: Rect) => void }) {
  const stageRef = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const W = scene.width;
  const H = scene.height;

  const toGame = (clientX: number, clientY: number) => {
    const b = stageRef.current?.getBoundingClientRect();
    if (!b || b.width === 0 || b.height === 0) return { gx: 0, gy: 0 };
    return { gx: ((clientX - b.left) * W) / b.width, gy: ((clientY - b.top) * H) / b.height };
  };

  const onPointerDown = (mode: DragMode) => (e: RPointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    const { gx, gy } = toGame(e.clientX, e.clientY);
    drag.current = { mode, startGx: gx, startGy: gy, rect };
  };

  const onPointerMove = (e: RPointerEvent<HTMLElement>) => {
    const d = drag.current;
    if (!d) return;
    const { gx, gy } = toGame(e.clientX, e.clientY);
    const r = d.rect;
    const s = r.width;
    let next: Rect;
    switch (d.mode) {
      case 'move':
        next = { ...r, x: r.x + gx - d.startGx, y: r.y + gy - d.startGy };
        break;
      case 'se': {
        const ax = r.x, ay = r.y;
        const size = clamp(Math.max(gx - ax, gy - ay), MIN_SIZE, Math.min(W - ax, H - ay));
        next = { x: ax, y: ay, width: size, height: size };
        break;
      }
      case 'nw': {
        const ax = r.x + s, ay = r.y + s;
        const size = clamp(Math.max(ax - gx, ay - gy), MIN_SIZE, Math.min(ax, ay));
        next = { x: ax - size, y: ay - size, width: size, height: size };
        break;
      }
      case 'ne': {
        const ax = r.x, ay = r.y + s;
        const size = clamp(Math.max(gx - ax, ay - gy), MIN_SIZE, Math.min(W - ax, ay));
        next = { x: ax, y: ay - size, width: size, height: size };
        break;
      }
      case 'sw': {
        const ax = r.x + s, ay = r.y;
        const size = clamp(Math.max(ax - gx, gy - ay), MIN_SIZE, Math.min(ax, H - ay));
        next = { x: ax - size, y: ay, width: size, height: size };
        break;
      }
    }
    onChange(normalize(next, W, H));
  };

  const endDrag = (e: RPointerEvent<HTMLElement>) => {
    if (!drag.current) return;
    drag.current = null;
    try { e.currentTarget.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
  };

  const onKeyDown = (e: RKeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 10 : 1;
    let next: Rect | null = null;
    switch (e.key) {
      case 'ArrowLeft': next = { ...rect, x: rect.x - step }; break;
      case 'ArrowRight': next = { ...rect, x: rect.x + step }; break;
      case 'ArrowUp': next = { ...rect, y: rect.y - step }; break;
      case 'ArrowDown': next = { ...rect, y: rect.y + step }; break;
      case '+': case '=': next = { ...rect, width: rect.width + step, height: rect.height + step }; break;
      case '-': case '_': next = { ...rect, width: rect.width - step, height: rect.height - step }; break;
    }
    if (next) {
      e.preventDefault();
      onChange(normalize(next, W, H));
    }
  };

  const pct = (v: number, total: number) => `${(v / total) * 100}%`;

  return (
    <div className="cal-stage" ref={stageRef}>
      <img src={scene.dataUrl} alt="Klatka gry" draggable={false} />
      <div
        className="cal-rect"
        tabIndex={0}
        role="slider"
        aria-label="Ramka minimapy"
        aria-valuetext={`x ${rect.x}, y ${rect.y}, rozmiar ${rect.width}`}
        style={{ left: pct(rect.x, W), top: pct(rect.y, H), width: pct(rect.width, W), height: pct(rect.height, H) }}
        onPointerDown={onPointerDown('move')}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onKeyDown={onKeyDown}
      >
        {(['nw', 'ne', 'sw', 'se'] as const).map((m) => (
          <span key={m} className={`cal-handle h-${m}`} onPointerDown={onPointerDown(m)} />
        ))}
      </div>
    </div>
  );
}

/** Static crop of the grabbed scene at the current (possibly unsaved) rect. */
function CropPreview({ scene, rect, size }: { scene: Scene; rect: Rect; size: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [img, setImg] = useState<HTMLImageElement | null>(null);

  useEffect(() => {
    const i = new Image();
    let alive = true;
    i.onload = () => { if (alive) setImg(i); };
    i.src = scene.dataUrl;
    return () => { alive = false; };
  }, [scene.dataUrl]);

  useEffect(() => {
    const c = canvasRef.current;
    const ctx = c?.getContext('2d');
    if (!c || !ctx || !img) return;
    ctx.clearRect(0, 0, c.width, c.height);
    // Grabbed image may differ from scene.width/height in pixel size; scale accordingly.
    const kx = img.naturalWidth / scene.width;
    const ky = img.naturalHeight / scene.height;
    ctx.imageSmoothingQuality = 'high';
    ctx.drawImage(img, rect.x * kx, rect.y * ky, rect.width * kx, rect.height * ky, 0, 0, c.width, c.height);
  }, [img, rect.x, rect.y, rect.width, rect.height, scene.width, scene.height]);

  return (
    <div className="mp-box" style={{ width: size, height: size }}>
      <canvas ref={canvasRef} width={size} height={size} />
    </div>
  );
}
