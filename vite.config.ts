import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

// Tauri expects a fixed port (src-tauri/tauri.conf.json → devUrl) and must not lose it.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Browser-test artefacts (.playwright-mcp) and the Rust side never trigger HMR.
    watch: { ignored: ['**/src-tauri/**', '**/target/**', '**/crates/**', '**/.playwright-mcp/**', '**/dist/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    // WKWebView on macOS 13+ (Safari 16+)
    target: 'safari16',
    chunkSizeWarningLimit: 1200,
  },
})
