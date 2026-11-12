import { memo, useEffect, useRef, useState } from 'react';
import type { MinimapFrame } from '../../shared/types';
import { fmtMs, rc } from './util';

/**
 * Live minimap preview fed by onMinimapFrame. Frames update the DOM directly
 * (no React re-render per frame) to keep the window calm at 10+ fps.
 */
export const MinimapPreview = memo(function MinimapPreview({ size = 220 }: { size?: number }) {
  const imgRef = useRef<HTMLImageElement>(null);
  const metaRef = useRef<HTMLSpanElement>(null);
  const [hasFrame, setHasFrame] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const lastAt = useRef(0);

  useEffect(() => {
    let unsub: (() => void) | undefined;
    try {
      unsub = rc().onMinimapFrame((f: MinimapFrame) => {
        lastAt.current = Date.now();
        if (imgRef.current) imgRef.current.src = f.dataUrl;
        if (metaRef.current) {
          metaRef.current.textContent = `${f.width}×${f.height}`
            + (f.offsetMs != null ? ` · ${fmtMs(f.offsetMs)}` : '');
        }
        setHasFrame(true);
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    // Mark the preview stale when frames stop arriving.
    const t = window.setInterval(() => {
      if (lastAt.current && Date.now() - lastAt.current > 3000) setHasFrame(false);
    }, 1000);
    return () => {
      window.clearInterval(t);
      try { unsub?.(); } catch { /* ignore */ }
    };
  }, []);

  return (
    <div className="minimap-preview">
      <div className="mp-box" style={{ width: size, height: size }}>
        <img ref={imgRef} alt="" className={hasFrame ? '' : 'stale'} />
        {!hasFrame && <div className="mp-empty">{error ?? 'Brak klatek minimapy'}</div>}
      </div>
      <div className="mp-meta">
        <span className="muted">Podgląd minimapy</span>
        <span ref={metaRef} className="num" />
      </div>
    </div>
  );
});
