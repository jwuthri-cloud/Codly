// The app's portraits, reactive: `portraitOf` returns '' until the portrait is drawn, and
// whatever reads it (a `$derived`, the markup) runs again when it is. portraitCache.ts has the
// rules; this file gives it the browser: the lazy import of portrait.ts (three.js stays out of
// the startup graph), "after the page has painted", and the theme.

import { PortraitCache, type Sitter } from './portraitCache';

let ready = $state(0);

const cache = new PortraitCache(
  {
    load: () => import('./portrait').then((m) => m.agentPortrait),
    // After the next paint: requestIdleCallback is not in WKWebView.
    later: (cb) => requestAnimationFrame(() => setTimeout(cb, 0)),
  },
  () => ready++,
);

// The app resolves the theme onto <html data-theme> (App.svelte), "system" included.
if (typeof MutationObserver !== 'undefined') {
  const html = document.documentElement;
  let theme = html.dataset.theme;
  new MutationObserver(() => {
    if (html.dataset.theme === theme) return;
    theme = html.dataset.theme;
    cache.retheme();
  }).observe(html, { attributes: true, attributeFilter: ['data-theme'] });
}

/** The agent's portrait (a PNG data URL), or '' while it is not drawn yet. */
export function portraitOf(agent: Sitter): string {
  void ready;
  return cache.get(agent);
}
