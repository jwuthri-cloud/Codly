<!--
  Check the work: the work of an agent you chat with has no gate, so nothing checks it before you
  merge its branch. This runs the repo's required checks (`.codly/checks`, turned on in Settings →
  Safety) in the agent's worktree and shows each command's result, with the output of the ones
  that failed. It stops no merge: the result is yours to read.
-->
<script lang="ts">
  import { api } from '../lib/api';
  import type { Agent } from '../lib/gen/Agent';
  import type { CheckResult } from '../lib/gen/CheckResult';
  import { checkLine, checksLine } from '../lib/util/checks';
  import Icon from '../components/Icon.svelte';

  interface Props {
    agent: Agent;
    /** Its turn is over: the files do not change while the checks run. */
    resting: boolean;
  }
  let { agent, resting }: Props = $props();

  let running = $state(false);
  let results = $state<CheckResult[] | null>(null);
  let error = $state<string | null>(null);
  // A result is about the files as they were: it goes when the agent works again, or another agent is shown.
  const moment = $derived(`${agent.id} ${agent.status}`);
  $effect(() => {
    void moment;
    results = null;
    error = null;
  });

  async function run() {
    const id = agent.id;
    running = true;
    error = null;
    try {
      const r = await api.checkAgent(id);
      if (id === agent.id) results = r;
    } catch (e) {
      if (id === agent.id) {
        results = null;
        error = e instanceof Error ? e.message : String(e);
      }
    } finally {
      running = false;
    }
  }
</script>

<div class="checks">
  <div class="head">
    <span class="what" role="status">{running ? 'Running your required checks…' : results ? checksLine(results) : 'Codly has not checked this work.'}</span>
    <button class="btn sm" disabled={running || !resting} onclick={run} data-tip={resting ? 'Run the repo’s required checks (.codly/checks) in this worktree' : 'Wait until its turn ends'}>
      <Icon name="check" size={12} />Check the work
    </button>
  </div>
  {#if error}<p class="bad">Couldn’t check the work: {error}</p>{/if}
  {#if results}
    <ul>
      {#each results as r (r.command)}
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
