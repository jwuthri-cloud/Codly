<!--
  Your requests, side by side: one tab per thing you asked for, each with its own Lead and plan
  running in parallel. Title = your first words; the dot says where its plan is (planning ·
  waiting for you · running · done), amber when it needs you. ⌘[ / ⌘] switch, ⌘N starts one.
-->
<script lang="ts">
  import { requests } from '../lib/state/requests.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';

  const list = $derived(requests.list);
  let bar: HTMLDivElement | undefined = $state();
  $effect(() => {
    void ui.request;
    bar?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
</script>

<nav class="tabs" aria-label="Requests">
  <div class="list" role="tablist" bind:this={bar}>
    {#each list as r (r.thread)}
      {@const on = ui.request === r.thread}
      <button
        role="tab"
        aria-selected={on}
        class="tab p-{r.phase}"
        class:on
        class:amber={r.needsYou}
        onclick={() => ui.openRequest(r.thread)}
        aria-label="{r.title} — {r.status}"
        data-tip="{r.lead && r.lead.name !== 'Lead' ? `${r.lead.name} · ` : ''}{r.status}"
        data-tip-kbd="⌘[ ⌘]"
      >
        <span class="dot" aria-hidden="true"></span>
        <span class="t">{r.title}</span>
        {#if r.waiting}<span class="n num">{r.waiting}</span>{/if}
      </button>
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
  .tab { display: inline-flex; align-items: center; gap: 8px; height: 30px; padding: 0 12px 0 10px; border-radius: var(--r-full); max-width: 230px; flex-shrink: 0; color: var(--ink-3); font-size: var(--t-sm); font-weight: var(--w-med); transition: background-color var(--d-2), color var(--d-2); }
  .tab:hover { background: var(--surface-2); color: var(--ink-1); }
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
