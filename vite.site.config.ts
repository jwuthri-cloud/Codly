import { defineConfig } from 'vite'

// The marketing site (site/) → dist-site/. `pnpm build:site` then builds the app's browser
// demo (the scripted fake repo, no Rust core) into dist-site/demo/. Vercel serves dist-site.
export default defineConfig({
  root: 'site',
  publicDir: false,
  build: { outDir: '../dist-site', emptyOutDir: true, target: 'es2022' },
})
