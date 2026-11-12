// Frontend for the Tauri app (rust/app): control panel + the single HUD window.
import { build } from 'esbuild';
import { cpSync, mkdirSync } from 'node:fs';

const common = { bundle: true, sourcemap: true, logLevel: 'info', target: 'es2023' };

mkdirSync('dist/renderer', { recursive: true });
cpSync('src/renderer/static', 'dist/renderer', { recursive: true });

await build({ ...common, platform: 'browser', format: 'iife',
  entryPoints: {
    control: 'src/renderer/control/index.tsx',
    'hud-all': 'src/renderer/hud/all.ts',
  }, outdir: 'dist/renderer', jsx: 'automatic' });
