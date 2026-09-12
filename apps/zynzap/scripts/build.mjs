import { cpSync, copyFileSync, mkdirSync, rmSync } from 'node:fs';

rmSync('dist', { recursive: true, force: true });
mkdirSync('dist', { recursive: true });
copyFileSync('index.html', 'dist/index.html');
copyFileSync('_headers', 'dist/_headers');
cpSync('../../zynzapd/ui/fonts', 'dist/fonts', { recursive: true });

console.log('dist/index.html ← index.html');
console.log('dist/fonts ← zynzapd/ui/fonts');
