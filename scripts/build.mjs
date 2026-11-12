import { build } from 'esbuild';
import { cpSync, mkdirSync } from 'node:fs';

const watch = process.argv.includes('--watch');
const common = { bundle: true, sourcemap: true, logLevel: 'info', target: 'es2023' };

mkdirSync('dist/renderer', { recursive: true });
cpSync('src/renderer/static', 'dist/renderer', { recursive: true });

await Promise.all([
  build({ ...common, platform: 'node', format: 'cjs', external: ['electron'],
    entryPoints: { main: 'src/main/index.ts', preload: 'src/main/preload.ts', 'hud-preload': 'src/main/hud-preload.ts', hud: 'src/main/hud.ts', 'vision-worker': 'src/main/vision/worker.ts' }, outdir: 'dist' }),
  build({ ...common, platform: 'browser', format: 'iife',
    entryPoints: {
      control: 'src/renderer/control/index.tsx',
      overlay: 'src/renderer/overlay/index.ts',
      capture: 'src/renderer/capture/index.ts',
      'hud-minimap': 'src/renderer/hud/minimap.ts', 'hud-topbar': 'src/renderer/hud/topbar.ts',
      'hud-death': 'src/renderer/hud/death.ts', 'hud-loading': 'src/renderer/hud/loading.ts',
      'hud-power': 'src/renderer/hud/power.ts', 'hud-build': 'src/renderer/hud/build.ts',
    }, outdir: 'dist/renderer', jsx: 'automatic' }),
]);
if (watch) console.log('watch not implemented; rerun build');
