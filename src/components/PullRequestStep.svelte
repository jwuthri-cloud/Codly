<!--
  The step after the work is done: a pull request for a plan's branch, or a solo agent's.
  Create pull request… drafts it and opens the confirm; nothing is sent before you say so there.
  Once one is open: its link, its checks as last read, and Refresh, which reads its state and
  checks once. While it is open and the row is in view, its state and checks are read again
  every 30 seconds and when the window gets the focus: one timer for the pull request however
  many of its rows show, none for a row out of view, none after you merged its work here
  (`readsByItself`, `readWhileShown`). When it is merged or closed, Create pull
  request… comes back for the next one. After you merged the plan here, the row says where the
  pull request stands, as last read. Codly never pushes on its own.
-->
<script lang="ts">
  import type { PrSource } from '../lib/gen/PrSource';
  import { untrack } from 'svelte';
  import { checksLine, checksState, loadPullRequest, preparePullRequest, prKey, prUi, readsByItself, readWhileShown, refreshPullRequest } from '../lib/state/pullRequest.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { ago } from '../lib/util/format';
  import { openExternal } from '../lib/util/markdown';
  import { afterLocalMerge } from '../home/mergeChoice';
  import Icon from './Icon.svelte';

  interface Props {
    /** The plan, or the solo agent, whose Codly branch the pull request is for. */
    source: PrSource;
    /** Whether a push still makes sense (not once a plan is merged into your branch). */
    canPush?: boolean;
    /** Why a push has to wait (the agent is in the middle of a turn). */
    wait?: string | null;
    /** The branch you merged the plan into here (null: not merged here). */
    mergedInto?: string | null;
  }
  let { source, canPush = true, wait = null, mergedInto = null }: Props = $props();

  const key = $derived(prKey(source));
  const pr = $derived(prUi.opened[key]);
  let busy = $state(false);

  $effect(() => {
    void key;
    void loadPullRequest(source);
  });

  // An open pull request's checks change on GitHub: read them again by themselves while this
  // row is in view (the page shows), and as soon as the window comes back.
  let el: HTMLDivElement | undefined = $state();
  let inView = $state(false);
  $effect(() => {
    if (!el) return;
    const io = new IntersectionObserver(([e]) => (inView = e.isIntersecting));
    io.observe(el);
    return () => {
      io.disconnect();
      inView = false;
    };
  });
  const auto = $derived(readsByItself(pr, inView, !!mergedInto));
  $effect(() => {
    void key;
    if (!auto) return;
    return untrack(() => readWhileShown(source));
  });

  async function prepare() {
    busy = true;
    await preparePullRequest(source);
    busy = false;
  }

  async function refresh() {
    busy = true;
    await refreshPullRequest(source);
    busy = false;
  }
</script>

{#if pr}
  {@const url = pr.url}
  <div class="pr" bind:this={el}>
    <div class="row">
      <span class="what"><Icon name="merge" size={12} />Pull request into <code>{pr.base}</code>{pr.state === 'open' ? '' : ` (${pr.state})`}:</span>
      <button class="link truncate" onclick={() => openExternal(url)} data-tip="Open in your browser">{url.replace(/^https:\/\//, '')}</button>
    </div>
    {#if mergedInto}<p class="what merged">{afterLocalMerge(pr.state, mergedInto)}</p>{/if}
    <div class="row">
      <span class="what grow c-{pr.checks ? checksState(pr.checks) : 'none'}">
        Checks: {pr.checks ? `${checksLine(pr.checks)} · read ${ago(pr.checks.read_at, ui.now)}` : 'not read yet'}
      </span>
      <button class="btn sm ghost" disabled={busy} onclick={refresh}><Icon name="refresh" size={12} />Refresh</button>
      {#if canPush}
        <button class="btn sm" disabled={busy || !!wait} data-tip={wait} onclick={prepare}>{pr.state === 'open' ? 'Push new commits…' : 'Create pull request…'}</button>
      {/if}
    </div>
  </div>
{:else if pr === null && canPush}
  <div class="pr">
    <div class="row">
      <span class="what grow"><Icon name="branch" size={12} />For a review on GitHub, push this branch and open a pull request.</span>
      <button class="btn sm" disabled={busy || !!wait} data-tip={wait} onclick={prepare}>{busy ? 'Preparing…' : 'Create pull request…'}</button>
    </div>
  </div>
{/if}

<style>
  .pr { display: flex; flex-direction: column; gap: 6px; padding: 10px 12px; border-radius: var(--r-md, 10px); background: var(--surface-2); }
  .row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .what { display: inline-flex; align-items: center; gap: 6px; min-width: 0; font-size: var(--t-sm); color: var(--ink-2); }
  .what.grow { flex: 1; }
  .what.merged { display: block; }
  .what code { font-size: 11.5px; color: var(--ink-1); }
  .link { flex: 1; min-width: 0; text-align: left; font-size: var(--t-sm); color: var(--aurora-m); }
  .link:hover { text-decoration: underline; }
  .c-bad { color: var(--bad); }
  .c-ok { color: var(--ok); }
</style>
