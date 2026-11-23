<!--
  Check the work: the work of an agent you chat with has no gate, so nothing checks it before you
  merge its branch. This runs the repo's required checks (`.codly/checks`, turned on in Settings →
  Safety) in the agent's worktree and shows each command's result, with the output of the ones
  that failed. It stops no merge: the result is yours to read.
  A run and its result outlive this tab: the run is in `agentChecks`, the result is the decision
  the core records for it. A result goes when the agent works again: it was about other files.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { agentChecks } from '../lib/state/agentChecks.svelte';
  import { store } from '../lib/state/store.svelte';
  import { checkLine, checksLine, lastAgentCheck } from '../lib/util/checks';
  import { clock } from '../lib/util/format';
  import Icon from '../components/Icon.svelte';

  interface Props {
    agent: Agent;
    /** Its turn is over: the files do not change while the checks run. */
    resting: boolean;
  }
  let { agent, resting }: Props = $props();

  const running = $derived(agentChecks.running[agent.id] ?? false);
  const error = $derived(agentChecks.failed[agent.id] ?? null);
  const last = $derived.by(() => {
    void store.rev;
    return lastAgentCheck(store.decisions, agent.id, store.live[agent.id]?.turn?.startedAt ?? null);
  });
  const shown = $derived(running ? null : last);
</script>

<div class="checks">
  <div class="head">
    <span class="what" role="status">{running ? 'Running your required checks…' : shown ? `${checksLine(shown.results)} · checked at ${clock(shown.ts)}` : 'Codly has not checked this work.'}</span>
    <button class="btn sm" disabled={running || !resting} onclick={() => agentChecks.run(agent.id)} data-tip={resting ? 'Run the repo’s required checks (.codly/checks) in this worktree' : 'Wait until its turn ends'}>
      <Icon name="check" size={12} />Check the work
    </button>
  </div>
  {#if error && !running}<p class="bad">Couldn’t check the work: {error}</p>{/if}
  {#if shown}
    <ul>
      {#each shown.results as r (r.command)}
        <li class:fail={!r.ok}>
          <span class="line"><Icon name={r.ok ? 'check' : 'x'} size={12} stroke={2.2} /><code class="truncate">{r.command}</code><span class="how">{checkLine(r)}</span></span>
          {#if !r.ok && r.tail}<pre class="tail" aria-label="Output of {r.command}">{r.tail}</pre>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .checks { display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; border-radius: var(--r-md); background: var(--surface-1); font-size: var(--t-sm); line-height: var(--lh-sm); }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .what { color: var(--ink-2); }
  .bad { color: var(--bad); }
  ul { display: flex; flex-direction: column; gap: 4px; }
  .line { display: flex; align-items: center; gap: 8px; color: var(--ok); }
  .fail .line { color: var(--bad); }
  .line code { flex: 1; min-width: 0; font-size: 11.5px; color: var(--ink-1); }
  .how { flex: none; color: var(--ink-3); }
  .fail .how { color: var(--bad); }
  .tail { max-height: 180px; overflow: auto; margin: 2px 0 4px 20px; padding: 8px 10px; border-radius: var(--r-sm); background: var(--surface-2); font-size: 11px; line-height: 1.45; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--ink-2); }
</style>
