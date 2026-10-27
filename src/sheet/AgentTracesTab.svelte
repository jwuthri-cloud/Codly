<!-- Traces: the agent's tool calls, newest first, with the duration the CLI reported or Codly measured. -->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { store } from '../lib/state/store.svelte';
  import { clockSec, dur } from '../lib/util/format';
  import { buildTraces, summarize } from '../lib/util/traces';
  import Icon from '../components/Icon.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const calls = $derived.by(() => {
    void store.rev;
    return buildTraces(store.events, agent.id);
  });
  const sum = $derived(summarize(calls));
  const shown = $derived(calls.slice(-200).reverse());
  /** Longest known call: the bars are relative to it. */
  const slowest = $derived(Math.max(1, ...calls.map((c) => c.durationMs ?? 0)));
</script>

{#if calls.length}
  <p class="sum num">
    {sum.calls} {sum.calls === 1 ? 'call' : 'calls'} · {dur(sum.totalMs)} in tools{sum.failed ? ` · ${sum.failed} failed` : ''}{sum.running ? ` · ${sum.running} running` : ''}
  </p>
  <ul class="tr">
    {#each shown as c (c.callId)}
      <li>
        <div class="row">
          <span class="ic" class:bad={c.ok === false} class:run={c.ok === null}><Icon name={c.ok === null ? 'play' : c.ok ? 'check' : 'x-circle'} size={12} /></span>
          <span class="nm">{c.name}</span>
          <span class="lb truncate">{c.label}</span>
          <time class="num" data-tip={c.source === 'reported' ? 'Duration reported by the CLI' : c.source === 'measured' ? 'Measured by Codly between start and finish' : 'Still running'}>{c.durationMs == null ? 'running' : dur(c.durationMs)}</time>
        </div>
        <div class="bar" aria-hidden="true"><i style:width="{c.durationMs == null ? 0 : Math.max(2, (c.durationMs / slowest) * 100)}%" class:bad={c.ok === false}></i></div>
        <p class="meta num">{clockSec(c.start)}{c.exitCode != null && c.exitCode !== 0 ? ` · exit ${c.exitCode}` : ''}{c.testsPassed != null ? ` · ${c.testsPassed} passed${c.testsFailed ? `, ${c.testsFailed} failed` : ''}` : ''}{c.source === 'measured' ? ' · measured' : ''}</p>
      </li>
    {/each}
  </ul>
{:else}
  <p class="empty">No tool calls captured yet.</p>
{/if}

<style>
  .sum { margin-bottom: 8px; font-size: var(--t-sm); color: var(--ink-3); }
  .tr { display: flex; flex-direction: column; max-height: 420px; overflow-y: auto; margin: 0 -8px; }
  .tr li { padding: 3px 8px 5px; border-radius: var(--r-sm); }
  .tr li:hover { background: var(--surface-1); }
  .row { display: flex; align-items: center; gap: 8px; width: 100%; font-size: var(--t-sm); color: var(--ink-2); }
  .ic { display: grid; color: var(--ok); }
  .ic.bad { color: var(--bad); }
  .ic.run { color: var(--live); }
  .nm { font-weight: var(--w-med); color: var(--ink-1); }
  .lb { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-3); }
  time { font-size: var(--t-xs); color: var(--ink-2); white-space: nowrap; }
  .bar { height: 3px; margin: 3px 0 2px 20px; border-radius: 2px; background: var(--surface-2); overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--ink-3); border-radius: 2px; }
  .bar i.bad { background: var(--bad); }
  .meta { margin-left: 20px; font-size: var(--t-xs); color: var(--ink-3); }
  .empty { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
</style>
