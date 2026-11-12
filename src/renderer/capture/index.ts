// Hidden window: captures the game window via getUserMedia and ships
// minimap crops (PNG) and occasional full scenes (JPEG) to the main process.
import { CAPTURE } from '../../shared/ipc';
import type { Rect } from '../../shared/types';
import { readHealthBars } from '../../main/vision/healthbars';

interface Bridge { on(ch: string, cb: (v: any) => void): void; send(ch: string, v: unknown): void }
const bridge = (window as unknown as { rcCapture: Bridge }).rcCapture;

interface Config { fps: number; crop: Rect | null; sceneEverySec: number; /** ms between health-bar reads (0 = off). */ hpEveryMs?: number; hpColor?: 'red' | 'blue' }
let config: Config = { fps: 10, crop: null, sceneEverySec: 0 };
let lastHp = 0;
const hpCanvas = document.createElement('canvas');
let stream: MediaStream | null = null;
let timer: number | null = null;
let lastScene = 0;
const video = document.createElement('video');
video.muted = true;
const cropCanvas = document.createElement('canvas');
const sceneCanvas = document.createElement('canvas');

const status = (state: string, error: string | null = null) =>
  bridge.send(CAPTURE.status, { state, width: video.videoWidth || null, height: video.videoHeight || null, error });

async function encode(canvas: HTMLCanvasElement, type: string, quality?: number): Promise<Uint8Array> {
  const blob = await new Promise<Blob | null>(r => canvas.toBlob(r, type, quality));
  if (!blob) throw new Error('encode failed');
  return new Uint8Array(await blob.arrayBuffer());
}

function drawScene(): boolean {
  if (!video.videoWidth) return false;
  sceneCanvas.width = video.videoWidth;
  sceneCanvas.height = video.videoHeight;
  sceneCanvas.getContext('2d')!.drawImage(video, 0, 0);
  return true;
}

let busy = false;
async function tick() {
  if (busy || !video.videoWidth) return;
  busy = true;
  try {
    const at = Date.now();
    const c = config.crop;
    if (c && c.width > 8) {
      cropCanvas.width = c.width;
      cropCanvas.height = c.height;
      cropCanvas.getContext('2d')!.drawImage(video, c.x, c.y, c.width, c.height, 0, 0, c.width, c.height);
      bridge.send(CAPTURE.frame, { at, width: c.width, height: c.height, png: await encode(cropCanvas, 'image/png') });
    }
    // Enemy health bars above heads: only what the player sees on screen. Bars only, no image leaves.
    if (config.hpEveryMs && at - lastHp >= config.hpEveryMs && video.videoWidth) {
      lastHp = at;
      hpCanvas.width = video.videoWidth; hpCanvas.height = video.videoHeight;
      const ctx = hpCanvas.getContext('2d', { willReadFrequently: true })!;
      ctx.drawImage(video, 0, 0);
      const img = ctx.getImageData(0, 0, hpCanvas.width, hpCanvas.height);
      const bars = readHealthBars({ width: img.width, height: img.height, data: new Uint8Array(img.data.buffer) }, { enemyColor: config.hpColor ?? 'red' });
      bridge.send(CAPTURE.hp, { at, width: img.width, height: img.height, bars });
    }
    if (config.sceneEverySec > 0 && at - lastScene >= config.sceneEverySec * 1000 && drawScene()) {
      lastScene = at;
      bridge.send(CAPTURE.scene, { at, width: sceneCanvas.width, height: sceneCanvas.height, kind: 'jpeg', data: await encode(sceneCanvas, 'image/jpeg', 0.8) });
    }
  } catch (e) {
    status('error', String(e));
  } finally {
    busy = false;
  }
}

function restartTimer() {
  if (timer) clearInterval(timer);
  timer = window.setInterval(() => void tick(), Math.max(50, 1000 / config.fps));
}

function stop() {
  if (timer) clearInterval(timer);
  timer = null;
  stream?.getTracks().forEach(t => t.stop());
  stream = null;
  video.srcObject = null;
}

bridge.on(CAPTURE.start, async ({ sourceId, ...cfg }: { sourceId: string } & Config) => {
  stop();
  config = cfg;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: false,
      video: { mandatory: { chromeMediaSource: 'desktop', chromeMediaSourceId: sourceId, maxFrameRate: 30 } } as unknown as MediaTrackConstraints,
    });
    stream.getVideoTracks()[0]?.addEventListener('ended', () => { stop(); status('waiting', 'game window closed'); });
    video.srcObject = stream;
    await video.play();
    status('connected');
    restartTimer();
  } catch (e) {
    stop();
    status('error', String(e));
  }
});

bridge.on(CAPTURE.config, (cfg: Config) => { config = cfg; if (stream) restartTimer(); });
bridge.on(CAPTURE.stop, () => { stop(); status('off'); });
bridge.on(CAPTURE.grabScene, async () => {
  if (!drawScene()) return bridge.send(CAPTURE.scene, null);
  bridge.send(CAPTURE.scene, { at: Date.now(), width: sceneCanvas.width, height: sceneCanvas.height, kind: 'png', data: await encode(sceneCanvas, 'image/png') });
});
