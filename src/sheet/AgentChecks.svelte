<!--
  Check the work: the work of an agent you chat with has no gate, so nothing checks it before you
  merge its branch. This runs the repo's required checks (`.codly/checks`, turned on in Settings →
  Safety) in the agent's worktree and shows each command's result, with the output of the ones
  that failed. It stops no merge: the result is yours to read.
  The core keeps the run and its result (`agentChecks`), so both outlive this tab and a reload of
  the window. A result goes when the agent's files are not the checked ones any more: the agent
  worked, you put files back, a branch was merged in, or you changed a file yourself.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { agentChecks } from '../lib/state/agentChecks.svelte';
  import { checkLine, checksLine } from '../lib/util/checks';
  import { clock } from '../lib/util/format';
  import Icon from '../components/Icon.svelte';

  interface Props {
    agent: Agent;
    /** Its turn is over: the files do not change while the checks run. */
    resting: boolean;
    /** Changes when the Git tab read the worktree again (opened, refreshed, after a merge or
     *  restored files): the checked files may not be the ones in it any more. */
    refresh: unknown;
  }
  let { agent, resting, refresh }: Props = $props();

  const running = $derived(agentChecks.state[agent.id]?.running ?? false);
  const shown = $derived(running ? null : (agentChecks.state[agent.id]?.last ?? null));
  const error = $derived(agentChecks.failed[agent.id] ?? null);
  const unread = $derived(agentChecks.unread[agent.id] ?? null);

  $effect(() => {
    void refresh;
    void resting;
    void agentChecks.load(agent.id);
  });
  // A run on its way (also one started before the window was reloaded): ask until it ends.
  $effect(() => {
    if (!running) return;
    const id = agent.id;
    const timer = setInterval(() => void agentChecks.load(id), 2000);
    return () => clearInterval(timer);
  });
</script>

<div class="checks">
  <div class="head">
    <span class="what" role="status">{running ? 'Running your required checks…' : shown ? `${checksLine(shown.results)} · checked at ${clock(shown.at)}` : 'Codly has not checked this work as it is now.'}</span>
    <button class="btn sm" disabled={running || !resting} onclick={() => agentChecks.run(agent.id)} data-tip={resting ? 'Run the repo’s required checks (.codly/checks) in this worktree' : 'Wait until its turn ends'}>
      <Icon name="check" size={12} />Check the work
    </button>
  </div>
  {#if error && !running}<p class="bad">Couldn’t check the work: {error}</p>{/if}
  {#if unread}<p class="bad">Couldn’t read the last check: {unread}</p>{/if}
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
