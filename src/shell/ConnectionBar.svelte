<!--
  A browser page on `codly serve` that lost its event stream says so here, at the top, over what
  it shows: the conversation stays as it was and can be read. While Codly is down the page
  tries again by itself (src/lib/state/connection.ts: after 1, 2, 5 and 10 s, then every 10 s)
  and "Try now" tries at once. Signed out (a restart of `codly serve --allow-remote` made a new
  link) it does not try: only the printed link signs the page in. Back, it says so for 2 s.
  The app's window never shows it: its link is always "connected".
-->
<script lang="ts">
  import { store } from '../lib/state/store.svelte';
  import { clockSec } from '../lib/util/format';
  import Icon from '../components/Icon.svelte';

  const link = $derived(store.link);
</script>

{#if link.kind !== 'connected'}
  <div class="bar" class:back={link.kind === 'back'} role="status">
    {#if link.kind === 'down'}
      <span class="ic"><Icon name="alert" size={15} /></span>
      <p>Codly is not reachable. Trying again… <span class="when">Last try {clockSec(link.lastTry)}</span></p>
      <button class="btn sm" disabled={link.trying} onclick={() => void store.tryNow()}>{link.trying ? 'Trying…' : 'Try now'}</button>
    {:else if link.kind === 'signedOut'}
      <span class="ic"><Icon name="lock" size={15} /></span>
      <p>This page is signed out of Codly. Open the link that <code>codly serve</code> printed.</p>
    {:else}
      <span class="ic"><Icon name="check-circle" size={15} /></span>
      <p>Connected again</p>
    {/if}
  </div>
{/if}

<style>
  /* Under the top bar, centred, above the shell and its sheets, under a toast and a dialog. */
  .bar { position: fixed; top: calc(var(--top-h) + 4px); left: 50%; transform: translateX(-50%); z-index: calc(var(--z-toast) - 1); display: flex; align-items: center; gap: 10px; max-width: min(620px, calc(100vw - 32px)); min-height: 40px; padding: 4px 8px 4px 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); }
  .bar.back { padding-right: 14px; }
  .ic { flex-shrink: 0; color: var(--amber); display: flex; }
  .back .ic { color: var(--ok); }
  p { min-width: 0; color: var(--ink-1); font-weight: var(--w-med); }
  .when { margin-left: 6px; font-size: var(--t-sm); font-weight: var(--w-reg); color: var(--ink-3); }
  code { font-size: 0.9em; color: var(--ink-2); }
  .btn { flex-shrink: 0; }
</style>
