<!--
  A browser page on `codly serve` that lost its event stream says so here, at the top, over what
  it shows: the conversation stays as it was and can be read. While Codly is down the page
  tries again by itself (src/lib/state/connection.ts: after 1, 2, 5 and 10 s, then every 10 s)
  and "Try now" tries at once. Signed out (a restart of `codly serve --allow-remote` made a new
  link) it does not try: only the printed link signs the page in. Back, it says so for 2 s.
  A core that could not start while an office is open (a restart of the server that failed on
  its database) says so here too, with "Try again": the start-up screen says it otherwise.
  The app's window shows only that last one: its link is always "connected".
  The bar has a row of its own: under the Resume work bar when that shows, and the toasts move
  down while it shows (components/Toasts.svelte), so it covers neither.
-->
<script lang="ts" module>
  import { ide } from '../ide/ide.svelte';
  import { resumeWork } from '../lib/state/resumeWork.svelte';
  import { store } from '../lib/state/store.svelte';
  import { barRoom } from '../lib/util/toastPlace';
  import { RESUME_BAR_ROOM } from '../components/ResumeWork.svelte';

  /** The gap over the bar, its height and the gap under it, in px. */
  const OVER = 4;
  const HEIGHT = 40;
  const UNDER = 6;

  /** The bar has something to say: not on the start-up screen (App.svelte), which says it. */
  const shows = (): boolean => store.ready && (store.workspace !== null || !store.error) && (store.link.kind !== 'connected' || store.error !== null);

  /** The room the bar takes under the top bar (and the Resume work bar), in px: 0 while it shows nothing. */
  export const connectionBarRoom = (): number => (shows() ? OVER + HEIGHT + UNDER : 0);
</script>

<script lang="ts">
  import { clockSec } from '../lib/util/format';
  import Icon from '../components/Icon.svelte';

  const link = $derived(store.link);
  const failed = $derived(store.error);
  /** Under the Resume work bar, where the toasts' row also starts (Toasts.svelte). */
  const below = $derived(barRoom(!!resumeWork.offer, ide.visible, RESUME_BAR_ROOM));
  /** "Try again" after a core that failed: it runs. */
  let busy = $state(false);

  async function again() {
    busy = true;
    try {
      await store.retry();
    } finally {
      busy = false;
    }
  }
</script>

{#if shows()}
  <div class="bar" class:back={link.kind === 'back' && !failed} role="status" style:top="calc(var(--top-h) + {OVER + below}px)" style:min-height="{HEIGHT}px">
    {#if link.kind === 'down'}
      <span class="ic"><Icon name="alert" size={15} /></span>
      <p>Codly is not reachable. Trying again… <span class="when">Last try {clockSec(link.lastTry)}</span></p>
      <button class="btn sm" disabled={link.trying} onclick={() => void store.tryNow()}>{link.trying ? 'Trying…' : 'Try now'}</button>
    {:else if link.kind === 'signedOut'}
      <span class="ic"><Icon name="lock" size={15} /></span>
      <p>This page is signed out of Codly. Open the link that <code>codly serve</code> printed.</p>
    {:else if failed}
      <span class="ic"><Icon name="alert" size={15} /></span>
      <p class="truncate" data-tip={failed}>Codly couldn’t start. <span class="when">{failed}</span></p>
      <button class="btn sm" disabled={busy} onclick={again}>{busy ? 'Trying…' : 'Try again'}</button>
    {:else}
      <span class="ic"><Icon name="check-circle" size={15} /></span>
      <p>Connected again</p>
    {/if}
  </div>
{/if}

<style>
  /* Under the top bar, centred, above the shell and its sheets, under a toast and a dialog. */
  .bar { position: fixed; left: 50%; transform: translateX(-50%); z-index: calc(var(--z-toast) - 1); display: flex; align-items: center; gap: 10px; max-width: min(620px, calc(100vw - 32px)); padding: 4px 8px 4px 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); }
  .bar.back { padding-right: 14px; }
  .ic { flex-shrink: 0; color: var(--amber); display: flex; }
  .back .ic { color: var(--ok); }
  p { min-width: 0; color: var(--ink-1); font-weight: var(--w-med); }
  .when { margin-left: 6px; font-size: var(--t-sm); font-weight: var(--w-reg); color: var(--ink-3); }
  code { font-size: 0.9em; color: var(--ink-2); }
  .btn { flex-shrink: 0; }
</style>
