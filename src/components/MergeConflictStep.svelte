<!--
  A merge that cannot go straight in. A conflict: the files your branch and the Codly branch
  both changed, and the two ways out. "Have an agent resolve it" brings your branch into Codly's
  own worktree (your checkout is not touched) and an agent resolves the conflict there; "I
  resolve it myself" shows the branch and the git commands to run in your project. While a solo
  agent is on it, "Stop resolving" takes the merge out of its worktree. For a plan, also the
  checks that failed (or could not run) on the merge result, with Merge anyway, and for checks
  that failed "Have an agent fix it": a task of the plan makes them pass in Codly's own
  worktree. Before any of that, one line says how far the Codly branch is behind yours. Shows
  nothing when the branch is built on yours as it is.
-->
<script lang="ts">
  import type { PrSource } from '../lib/gen/PrSource';
  import { copyText } from '../lib/state/actions';
  import { behindLine, fileList, loadMergeState, mergeByHand, mergeKey, mergeUi, resolveMergeConflict, stopResolvingMerge } from '../lib/state/mergeState.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    /** The plan, or the solo agent, whose Codly branch would be merged. */
    source: PrSource;
    /** Changes when a merge was tried or a branch may have moved: the state is read again. */
    refresh?: unknown;
    /** The solo agent's name: it resolves its own conflict. Null for a plan. */
    who?: string | null;
    /** Why an agent cannot start on it, or be stopped, now (the solo agent is in the middle of a turn). */
    wait?: string | null;
    /** Merge anyway, after the plan's checks failed on the merge result. */
    onunchecked?: (() => void) | null;
  }
  let { source, refresh = null, who = null, wait = null, onunchecked = null }: Props = $props();

  const key = $derived(mergeKey(source));
  const merge = $derived(mergeUi.states[key] ?? null);
  const commands = $derived(merge ? mergeByHand(merge) : '');
  let busy = $state(false);
  let byHand = $state(false);

  $effect(() => {
    void key;
    void refresh;
    void loadMergeState(source);
  });

  // The solo agent has the merge open in its worktree: it is on it, or it gave up half-way.
  const agentOnIt = $derived(source.kind === 'agent' && !!merge?.resolving);

  async function resolve() {
    busy = true;
    await resolveMergeConflict(source, who);
    busy = false;
  }
  async function stop() {
    busy = true;
    await stopResolvingMerge(source.id, who);
    busy = false;
  }
</script>

{#if merge && (merge.behind || merge.conflicts.length || merge.failed_checks.length || merge.checks_not_run)}
  <div class="mc">
    {#if merge.behind}
      <p class="what"><Icon name="branch" size={12} /><span>{behindLine(merge, who)}</span></p>
    {/if}
    {#if merge.conflicts.length}
      <p class="what"><Icon name="merge" size={12} /><span><code>{merge.target}</code> and {who ? `${who}’s branch` : 'this plan'} both changed {fileList(merge.conflicts)}. Nothing can be merged until that conflict is resolved.</span></p>
      {#if agentOnIt}
        <p class="note">{who ?? 'The agent'} is resolving it in its own worktree. Codly commits the merge when a turn ends with no conflict marker left; ask {who ?? 'it'} to finish if it stopped half-way.</p>
      {/if}
      <div class="row">
        {#if agentOnIt}
          <button class="btn sm" disabled={busy || !!wait} data-tip={wait ?? `Takes the merge out of ${who ?? 'the agent'}’s worktree. Its branch is as it was before; the conflict stays.`} onclick={stop}>Stop resolving</button>
        {:else}
          <button class="btn sm primary" disabled={busy || !!wait} data-tip={wait ?? `Codly brings ${merge.target} into its own worktree and an agent resolves the conflict there. Your checkout is not touched.`} onclick={resolve}>Have {who ?? 'an agent'} resolve it</button>
        {/if}
        <button class="btn sm" aria-expanded={byHand} onclick={() => (byHand = !byHand)}>I resolve it myself</button>
      </div>
      {#if byHand}
        <p class="note">Merge <code>{merge.branch}</code> into <code>{merge.target}</code> in your project, with these commands. {who ? `Then ${who}’s work is in your branch.` : 'Codly shows the plan as merged once you have committed the merge.'}</p>
        <pre class="cmds">{commands}</pre>
        <div class="row"><button class="btn sm ghost" onclick={() => copyText(commands, 'Commands copied')}>Copy the commands</button></div>
      {/if}
    {/if}
    {#if merge.failed_checks.length}
      <div class="row">
        <span class="what grow"><Icon name="merge" size={12} /><span>With this plan merged into <code>{merge.target}</code>, {#each merge.failed_checks as c, i (c)}{i ? ', ' : ''}<code>{c}</code>{/each} failed, so nothing was merged. <code>{merge.target}</code> has commits the plan was never checked with.</span></span>
        <button class="btn sm primary" disabled={busy} data-tip={`Codly brings ${merge.target} into its own worktree, and a task of the plan makes the checks pass there. Your checkout is not touched.`} onclick={resolve}>Have an agent fix it</button>
        {#if onunchecked}<button class="btn sm" onclick={onunchecked}>Merge anyway</button>{/if}
      </div>
    {/if}
    {#if merge.checks_not_run}
      <div class="row">
        <span class="what grow"><Icon name="merge" size={12} /><span>Codly could not run the plan’s checks on the merge result, so nothing was merged: {merge.checks_not_run} <code>{merge.target}</code> has commits the plan was never checked with.</span></span>
        {#if onunchecked}<button class="btn sm" onclick={onunchecked}>Merge anyway</button>{/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .mc { display: flex; flex-direction: column; gap: 8px; padding: 10px 12px; border-radius: var(--r-md, 10px); background: var(--surface-2); }
  .row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .what { display: flex; align-items: baseline; gap: 6px; min-width: 0; font-size: var(--t-sm); color: var(--ink-2); }
  .what.grow { flex: 1; }
  .what code, .note code { font-size: 11.5px; color: var(--ink-1); }
  .note { font-size: var(--t-sm); color: var(--ink-2); }
  .cmds { margin: 0; padding: 8px 10px; border-radius: var(--r-sm); background: var(--surface-1); font-family: var(--font-mono); font-size: 11.5px; line-height: 1.5; color: var(--ink-1); white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
</style>
