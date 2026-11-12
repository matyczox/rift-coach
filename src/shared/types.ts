// Shared contract between the main process and all renderer windows.

/** Rectangle in game-window pixels. */
export interface Rect { x: number; y: number; width: number; height: number }

export type SourceState = 'off' | 'waiting' | 'connected' | 'error';

export interface MinimapCalibration {
  /** Minimap square inside the game window, in game-window pixels. */
  rect: Rect;
  /** Game window size the rect was measured against. */
  windowWidth: number;
  windowHeight: number;
  source: 'auto' | 'manual';
  updatedAt: number;
}

export interface Settings {
  calibration: MinimapCalibration | null;
  /** Minimap frames per second while live (used by the tracker). */
  minimapFps: number;
  /** Minimap frames per second written to recordings. */
  recordFps: number;
  /** Seconds between full-screen JPEG snapshots in recordings. */
  sceneEverySec: number;
  /** Start recording automatically when a supported game starts. */
  autoRecord: boolean;
  overlayVisible: boolean;
  hotkeys: { toggleOverlay: string; toggleRecording: string; enemyFlash: string };
  coach: { enabled: boolean; liveModel: string; reviewModel: string; periodSec: number; /** Web research once per patch/game: patch brief, lane matchup. */ researchModel: string; /** Fast web research in champion select: runes, draft roles. */ draftModel: string };
  riot: { apiKey: string; platform: string };
  /** Set a "Rift Coach" rune page automatically in champion select. */
  autoRunes: boolean;
  /** Start with Windows in the background (window hidden; overlays appear when a game starts). */
  launchAtLogin: boolean;
}

export interface GameInfo {
  gameMode: string;
  gameTime: number;
  mapNumber: number;
  activeChampion: string | null;
  /** Champion names: own team first. */
  allies: string[];
  enemies: string[];
  /** Data Dragon ids (same order as allies / enemies), for portraits. */
  allyIds?: string[];
  enemyIds?: string[];
}

export interface CaptureStatus {
  state: SourceState;
  /** Game window client size, when found. */
  windowWidth: number | null;
  windowHeight: number | null;
  fps: number;
  lastFrameAt: number | null;
  error: string | null;
}

export interface RecordingStatus {
  active: boolean;
  sessionId: string | null;
  frames: number;
  apiSamples: number;
  bytes: number;
  startedAt: number | null;
}

export interface ReplayStatus {
  sessionId: string;
  playing: boolean;
  speed: number;
  /** Milliseconds from the start of the recording. */
  positionMs: number;
  durationMs: number;
}

export interface HarvestStatus {
  /** waiting-fog = paused until the user sets the replay HUD 'Mgła wojny' to needFog and confirms. */
  state: 'idle' | 'launching' | 'running' | 'waiting-fog' | 'done' | 'error';
  gameId: number | null;
  /** Team view being recorded (replay HUD fog team). */
  pass: 'blue' | 'red' | 'all' | null;
  needFog?: 'blue' | 'red' | 'all';
  positionSec: number;
  lengthSec: number;
  frames: number;
  error: string | null;
  /** Remaining replay ids in the queue (excluding the current one). */
  queue?: number[];
}

/** A downloaded .rofl replay file. */
export interface ReplayFile {
  gameId: number;
  name: string;
  sizeMb: number;
  modifiedAt: number;
  /** A finished harvest of this replay exists. */
  harvested: boolean;
}

export interface AppStatus {
  version: string;
  /** Data Dragon version for champion portraits (null until static data loads). */
  ddragonVersion: string | null;
  liveApi: SourceState;
  lcu: SourceState;
  lcuPhase: string | null;
  game: GameInfo | null;
  capture: CaptureStatus;
  recording: RecordingStatus;
  replay: ReplayStatus | null;
  /** A League replay is open and the Replay API answers. */
  replayApi: SourceState;
  harvest: HarvestStatus;
  coach: import('./intel').CoachStatus;
  /** Last coach call shown in the top bar, if fresh. */
  lastAdvice: import('./intel').CoachAdvice | null;
  profiles: { state: 'idle' | 'loading' | 'ready' | 'error' | 'no-key'; count: number; error: string | null };
  runes: { state: 'idle' | 'working' | 'set' | 'no-slot' | 'error' | 'off'; label: string | null; error: string | null };
  settings: Settings;
}

export interface RecordingSummary {
  id: string;
  startedAt: number;
  durationMs: number;
  frames: number;
  bytes: number;
  gameMode: string | null;
  champion: string | null;
  allies: string[];
  enemies: string[];
  /** 'replay' = harvested from a League replay (frames = fog view, truth/ = no fog). */
  source?: 'live' | 'replay';
  gameId?: number | null;
}

/** One minimap frame as delivered to renderers (PNG data URL for display). */
export interface MinimapFrame {
  at: number;
  /** For replays: ms from recording start. */
  offsetMs?: number;
  width: number;
  height: number;
  dataUrl: string;
}

/** State pushed to the in-game overlay windows. */
export interface OverlayState {
  visible: boolean;
  /** Briefly outline the calibrated minimap so alignment can be checked. */
  showFrame: boolean;
  recording: boolean;
}

export interface MatchReview {
  id: string;                 // e.g. "2026-11-12T03-10-00"
  createdAt: number;
  champion: string; role: string; win: boolean | null;
  durationSec: number;
  kda: string;                // "5/3/7"
  csPerMin: number;
  /** ≤ 3 sentences. */
  summary: string;
  good: string[];             // 2–4 bullets
  improve: string[];          // 3–5 bullets, most important first
  /** Key moments, chronological. */
  moments: Array<{ t: number; kind: 'death' | 'kill' | 'objective' | 'advice'; text: string }>;
  /** Estimated gold difference (my team - enemy) over time. */
  goldCurve: Array<{ t: number; diff: number }>;
  model: string;
}

/** API exposed on window.rc by the preload script. */
export interface RcApi {
  getStatus(): Promise<AppStatus>;
  onStatus(cb: (s: AppStatus) => void): () => void;
  onMinimapFrame(cb: (f: MinimapFrame) => void): () => void;
  /** Overlay windows only. */
  onOverlay(cb: (s: OverlayState) => void): () => void;

  updateSettings(patch: Partial<Settings>): Promise<Settings>;

  /** Grab one full game-window frame (PNG data URL) for calibration. */
  grabScene(): Promise<{ dataUrl: string; width: number; height: number } | null>;
  autoCalibrate(): Promise<MinimapCalibration | null>;
  setCalibration(c: MinimapCalibration): Promise<void>;

  startRecording(): Promise<void>;
  stopRecording(): Promise<void>;
  listRecordings(): Promise<RecordingSummary[]>;
  deleteRecording(id: string): Promise<void>;
  openRecordingsFolder(): Promise<void>;

  replayOpen(id: string): Promise<ReplayStatus>;
  replayPlay(): Promise<void>;
  replayPause(): Promise<void>;
  replaySeek(positionMs: number): Promise<void>;
  replaySpeed(speed: number): Promise<void>;
  replayClose(): Promise<void>;

  /** Downloaded .rofl files from the League replays folder. */
  listReplayFiles(): Promise<ReplayFile[]>;
  /**
   * Collect fog/no-fog data. With gameIds the app launches each replay through
   * the League client (must not be in a game); without, it uses the replay already open.
   */
  harvestStart(opts: { gameIds?: number[]; speed?: number }): Promise<void>;
  harvestStop(): Promise<void>;
  /** User switched the replay HUD fog team as asked (state waiting-fog). */
  harvestContinue(): Promise<void>;

  /** Post-game reviews, newest first. */
  listReviews(): Promise<MatchReview[]>;
  onReview(cb: (r: MatchReview) => void): () => void;
}

declare global {
  interface Window { rc: RcApi }
}
