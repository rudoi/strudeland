import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';
import bundleAudioworklet from 'vite-plugin-bundle-audioworklet';

// https://v2.tauri.app/start/frontend/vite/ (macOS desktop only, so no TAURI_DEV_HOST/HMR host handling)
export default defineConfig({
  // bundleAudioworklet inlines superdough's `?audioworklet` import as a data URL.
  plugins: [svelte(), bundleAudioworklet()],
  // Keep Rust compiler errors visible in the terminal.
  clearScreen: false,
  // The oldest WKWebView we support (macOS 13 ships Safari 16).
  build: { target: 'safari16' },
  server: {
    port: 1420,
    strictPort: true, // Tauri's devUrl is fixed, so fail instead of picking another port.
    host: '127.0.0.1',
    watch: { ignored: ['**/src-tauri/**'] },
  },
});
