import { resolve } from 'node:path';
import { defineConfig } from 'vite';
import bundleAudioworklet from 'vite-plugin-bundle-audioworklet';

// The headless render page (render/index.html -> dist-render/), served by scripts/render/chromium.ts.
// Standalone: no Svelte, no Tauri dev server. Never imported by the app.
export default defineConfig({
  root: resolve(import.meta.dirname, 'render'), // so the HTML lands at dist-render/index.html
  base: './',
  // bundleAudioworklet inlines superdough's `?audioworklet` import as a data URL.
  plugins: [bundleAudioworklet()],
  clearScreen: false,
  build: {
    target: 'chrome120',
    outDir: resolve(import.meta.dirname, 'dist-render'),
    emptyOutDir: true,
    chunkSizeWarningLimit: 4096, // one page, loaded once from localhost
  },
});
