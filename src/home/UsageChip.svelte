<!--
  The context meter and the cost of the agent you talk to, beside the composer's pickers: how
  full its context was in its last turn (amber from 80%) and what its session cost so far, as
  its CLI reports them. Nothing is shown for a number the CLI does not report. Click: the
  agent's sheet, open at Context.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { ui } from '../lib/state/ui.svelte';
  import { contextFraction, costLine, costShort } from '../lib/util/cost';
  import { pct, tokens } from '../lib/util/format';

  interface Props {
    agent: Agent | null;
  }
  let { agent }: Props = $props();

  const frac = $derived(agent ? contextFraction(agent.usage) : null);
  const cost = $derived(agent ? costShort(agent) : null);
  const tip = $derived.by(() => {
    if (!agent) return '';
    const u = agent.usage;
    const context = frac != null ? `${tokens(u.context_used)} of ${tokens(u.context_window)} context in ${agent.name}’s last turn · ` : '';
    return `${context}${costLine(agent)}`;
  });

  function open() {
    if (!agent) return;
    ui.sheetFocus = 'cost';
    ui.selectAgent(agent.id);
  }
</script>

{#if agent && (frac != null || cost)}
  <button class="chip num" class:near={frac != null && frac >= 0.8} onclick={open} data-tip={tip} aria-label="{agent.name}: context and cost">
    {#if frac != null}
      <span class="bar"><i style:width="{Math.min(100, frac * 100)}%"></i></span>{pct(frac)} context
    {/if}
    {#if cost}<span class="sub">{frac != null ? '· ' : ''}{cost}</span>{/if}
  </button>
{/if}

<style>
  .chip { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 9px; margin-left: auto; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-3); white-space: nowrap; }
  .chip:hover { background: var(--surface-2); color: var(--ink-1); }
  .chip.near { color: var(--amber-ink); }
  .sub { font-weight: var(--w-reg); }
  .bar { width: 28px; height: 5px; border-radius: 3px; background: var(--surface-3); overflow: hidden; }
  .bar i { display: block; height: 100%; border-radius: 3px; background: var(--ink-3); transition: width var(--d-4) var(--ease-out); }
  .chip.near .bar i { background: var(--amber); }
</style>
