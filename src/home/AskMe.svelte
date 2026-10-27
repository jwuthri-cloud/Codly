<!--
  Ask me: the top-bar button and the inbox it opens. Agents raise a card (codly_ask_user) when
  they need a decision only you can make; blocking ones stop their task until you reply. The
  button shows how many wait (amber when something is blocked). Esc or a click outside closes.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { blockers } from '../lib/state/blockers.svelte';
  import { store } from '../lib/state/store.svelte';
  import Icon from '../components/Icon.svelte';
  import AskMeCard from './AskMeCard.svelte';

  let btn: HTMLButtonElement | undefined = $state();
  let panel: HTMLElement | undefined = $state();
  let showDone = $state(false);

  onMount(() => blockers.start());
  // A snapshot (first connect, workspace switch) means the cards may have changed under us.
  $effect(() => {
    void store.epoch;
    void blockers.load();
  });

  const count = $derived(blockers.open.length);
  const blocked = $derived(blockers.blocking.length);

  function outside(e: PointerEvent) {
    const t = e.target as Node;
    if (blockers.panel && panel && !panel.contains(t) && !btn?.contains(t)) blockers.panel = false;
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape' && blockers.panel) {
      blockers.panel = false;
      btn?.focus();
    }
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={key} />

<button class="ask" class:blocked class:has={count > 0} bind:this={btn} onclick={() => (blockers.panel = !blockers.panel)} aria-haspopup="dialog" aria-expanded={blockers.panel}
  aria-label={count ? `Ask me: ${count} waiting` : 'Ask me'} data-tip={count ? (blocked ? `${blocked} blocking · ${count} waiting for you` : `${count} heads-up, nothing blocked`) : 'Nothing waiting for you'}>
  <Icon name="inbox" size={14} />{#if count}<b class="num">{count}</b>{/if}
</button>

{#if blockers.panel}
  <div class="panel glass" bind:this={panel} role="dialog" aria-label="Ask me">
    <header>
      <h2>Ask me</h2>
      <span class="sub">{count ? `${count} waiting for you` : 'Nothing waiting for you'}</span>
    </header>
    <div class="list">
      {#each blockers.open as b (b.id)}<AskMeCard blocker={b} />{:else}
        <p class="empty">All clear. When an agent needs a decision only you can make, it shows up here and its task waits for your answer.</p>
      {/each}
      {#if blockers.closed.length}
        <button class="more" onclick={() => (showDone = !showDone)} aria-expanded={showDone}>{showDone ? 'Hide' : 'Show'} answered ({blockers.closed.length})</button>
        {#if showDone}{#each blockers.closed.slice(0, 20) as b (b.id)}<AskMeCard blocker={b} />{/each}{/if}
      {/if}
      {#if blockers.error}<p class="err">{blockers.error}</p>{/if}
    </div>
  </div>
{/if}

<style>
  .ask { display: inline-flex; align-items: center; justify-content: center; gap: 6px; height: 34px; min-width: 34px; padding: 0 10px; border-radius: var(--r-full); color: var(--ink-3); background: var(--glass); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--glass-edge); transition: color var(--d-2) var(--ease-out); }
  .ask:hover, .ask[aria-expanded='true'] { color: var(--ink-1); }
  .ask.has { color: var(--ink-1); }
  .ask b { font-size: var(--t-sm); font-weight: var(--w-semi); }
  .ask.blocked { color: var(--amber-ink); background: color-mix(in srgb, var(--amber) 16%, var(--glass)); box-shadow: var(--glass-edge), inset 0 0 0 1px color-mix(in srgb, var(--amber) 45%, transparent); }
  .panel { position: fixed; z-index: var(--z-pop); top: calc(var(--top-h) + 6px); right: var(--gutter); width: min(440px, calc(100vw - 2 * var(--gutter))); max-height: calc(100vh - var(--top-h) - 24px); display: flex; flex-direction: column; border-radius: var(--r-xl); background: var(--glass-solid); overflow: hidden; animation: rise-in var(--d-3) var(--spring); }
  header { display: flex; align-items: baseline; gap: 10px; padding: 14px 18px 8px; }
  h2 { font-size: var(--t-lg); font-weight: var(--w-semi); }
  .sub { font-size: var(--t-sm); color: var(--ink-3); }
  .list { display: flex; flex-direction: column; gap: 10px; padding: 6px 12px 14px; overflow-y: auto; }
  .empty { padding: 18px 8px; font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-3); text-align: center; }
  .more { align-self: center; height: 26px; padding: 0 12px; border-radius: var(--r-full); font-size: var(--t-sm); color: var(--ink-3); }
  .more:hover { color: var(--ink-1); background: var(--surface-2); }
  .err { font-size: var(--t-sm); color: var(--bad); }
</style>
