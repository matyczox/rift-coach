// IPC channel names. Invoke channels map 1:1 to RcApi methods.
export const INVOKE = [
  'getStatus', 'updateSettings', 'grabScene', 'autoCalibrate', 'setCalibration',
  'startRecording', 'stopRecording', 'listRecordings', 'deleteRecording', 'openRecordingsFolder',
  'replayOpen', 'replayPlay', 'replayPause', 'replaySeek', 'replaySpeed', 'replayClose',
  'listReplayFiles', 'harvestStart', 'harvestStop', 'harvestContinue',
  'listReviews', 'getLearning',
] as const;
export type InvokeChannel = typeof INVOKE[number];

export const EVENT = { status: 'evt:status', minimapFrame: 'evt:minimapFrame', overlay: 'evt:overlay', review: 'evt:review' } as const;

/** Channels used only between main and the hidden capture renderer. */
export const CAPTURE = {
  start: 'cap:start',          // main -> capture: { sourceId, fps, crop }
  stop: 'cap:stop',            // main -> capture
  config: 'cap:config',        // main -> capture: { fps, crop, sceneEverySec }
  grabScene: 'cap:grabScene',  // main -> capture (request), reply via cap:scene
  frame: 'cap:frame',          // capture -> main: minimap crop
  scene: 'cap:scene',          // capture -> main: full frame
  status: 'cap:status',        // capture -> main: { state, width, height, error }
  hp: 'cap:hp',                // capture -> main: { at, bars } enemy health bars read on screen
} as const;
