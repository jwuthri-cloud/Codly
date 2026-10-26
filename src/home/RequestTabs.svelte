<!--
  Your requests, side by side: one tab per thing you asked for, each with its own Lead and plan
  running in parallel. Title = your first words; the dot says where its plan is (planning ·
  waiting for you · running · done), amber when it needs you. ⌘[ / ⌘] switch, ⌘N starts one.
  × closes a request (its Lead stops); a running plan asks first, since closing cancels it.
-->
<script lang="ts">
  import { closeRequest } from '../lib/state/actions';
  import { requests } from '../lib/state/requests.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';

  const list = $derived(requests.list);
  let bar: HTMLDivElement | undefined = $state();
  /** The running request waiting for "Close" / "Keep", and the one being closed. */
  let asking = $state<string | null>(null);
  let closing = $state<string | null>(null);

  async function close(thread: string) {
    asking = null;
    closing = thread;
    await closeRequest(thread);
    closing = null;
  }
  $effect(() => {
    void ui.request;
    bar?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
</script>

<nav class="tabs" aria-label="Requests">
  <div class="list" role="tablist" bind:this={bar}>
    {#each list as r (r.thread)}
      {@const on = ui.request === r.thread}
      {#if asking === r.thread}
        <div class="ask" role="group" aria-label="Close {r.title}">
          <span>Cancel its running plan and close?</span>
          <button class="btn sm danger" onclick={() => close(r.thread)}>Close</button>
          <button class="btn sm ghost" onclick={() => (asking = null)}>Keep</button>
        </div>
      {:else}
        <div class="tab p-{r.phase}" class:on class:amber={r.needsYou} class:closing={closing === r.thread} role="presentation">
          <button
            role="tab"
            aria-selected={on}
            class="pick"
            onclick={() => ui.openRequest(r.thread)}
            aria-label="{r.title} — {r.status}"
            data-tip="{r.lead && r.lead.name !== 'Lead' ? `${r.lead.name} · ` : ''}{r.status}"
            data-tip-kbd="⌘[ ⌘]"
          >
            <span class="dot" aria-hidden="true"></span>
            <span class="t">{r.title}</span>
            {#if r.waiting}<span class="n num">{r.waiting}</span>{/if}
          </button>
          <button
            class="x"
            aria-label="Close {r.title}"
            data-tip={r.phase === 'running' ? 'Close — cancels its running plan' : 'Close — its Lead stops'}
            disabled={closing === r.thread}
            onclick={() => (r.phase === 'running' ? (asking = r.thread) : close(r.thread))}
          >
            <Icon name="x" size={12} stroke={2} />
          </button>
        </div>
      {/if}
    {/each}
  </div>
  <button class="new" onclick={() => ui.newRequest()} aria-label="New request" data-tip="New request — its own Lead plans it while the others keep going" data-tip-kbd="⌘N">
    <Icon name="plus" size={14} stroke={1.8} />
  </button>
</nav>

<style>
  .tabs { display: flex; align-items: center; gap: 8px; height: 46px; padding: 0 12px 0 14px; box-shadow: inset 0 -1px 0 var(--line); flex-shrink: 0; }
  .list { flex: 1; min-width: 0; display: flex; align-items: center; gap: 4px; overflow-x: auto; scrollbar-width: none; mask-image: linear-gradient(90deg, #000 calc(100% - 18px), transparent); }
  .list::-webkit-scrollbar { display: none; }
  .tab { display: inline-flex; align-items: center; height: 30px; padding-right: 4px; border-radius: var(--r-full); max-width: 250px; flex-shrink: 0; color: var(--ink-3); font-size: var(--t-sm); font-weight: var(--w-med); transition: background-color var(--d-2), color var(--d-2), opacity var(--d-2); }
  .tab:hover { background: var(--surface-2); color: var(--ink-1); }
  .tab.closing { opacity: 0.45; }
  .pick { display: inline-flex; align-items: center; gap: 8px; height: 100%; min-width: 0; padding: 0 4px 0 10px; color: inherit; font: inherit; }
  .x { display: grid; place-items: center; width: 20px; height: 20px; border-radius: 50%; flex-shrink: 0; color: var(--ink-3); opacity: 0; transition: opacity var(--d-2), background-color var(--d-2); }
  .tab:hover .x, .tab.on .x, .x:focus-visible { opacity: 1; }
  .x:hover { background: var(--surface-3); color: var(--ink-1); }
  .ask { display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 4px 0 12px; border-radius: var(--r-full); flex-shrink: 0; background: var(--bad-soft); color: var(--ink-1); font-size: var(--t-sm); white-space: nowrap; }
  .tab.on { background: var(--surface-3); color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line-2); }
  .tab.amber { color: var(--amber-ink); }
  .tab.amber.on { background: color-mix(in srgb, var(--amber) 16%, var(--surface-2)); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--amber) 45%, transparent); }
  .t { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; box-shadow: inset 0 0 0 1.5px var(--ink-4); }
  .p-planning .dot { background: var(--aurora); box-shadow: 0 0 0 3px var(--live-soft); animation: breathe 1.8s var(--ease-out) infinite; }
  .p-running .dot { background: var(--live); box-shadow: 0 0 0 3px var(--live-soft); }
  .p-done .dot { background: var(--ok); box-shadow: none; }
  .p-merged .dot { box-shadow: inset 0 0 0 1.5px var(--ok); }
  .p-failed .dot { background: var(--bad); box-shadow: none; }
  .tab.amber .dot { background: var(--amber); box-shadow: 0 0 0 3px color-mix(in srgb, var(--amber) 25%, transparent); animation: none; }
  .n { min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--amber); color: #241504; font-size: 10px; font-weight: 700; line-height: 16px; text-align: center; }
  .new { display: grid; place-items: center; width: 30px; height: 30px; border-radius: var(--r-full); color: var(--ink-2); flex-shrink: 0; }
  .new:hover { background: var(--surface-2); color: var(--ink-1); }
  @keyframes breathe { 50% { box-shadow: 0 0 0 5px transparent; } }
  @media (prefers-reduced-motion: reduce) { .p-planning .dot { animation: none; } }
</style>
