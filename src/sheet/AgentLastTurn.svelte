<!--
  "Last turn" in an agent's Git tab: what its last turn changed in the files, from the point
  Codly kept before that turn to the files as they are now, committed or not. Each file opens to
  its diff. It is read when you pick it and when the Git tab reads the worktree again (the turn
  ended, you pressed Refresh, files were put back), one read at a time; while the agent works,
  Refresh shows the turn so far.
  A 1:1 agent on a task works in the task's worktree, and its checkpoints are of that one: the
  turn shown is then the last one there, and a line says so, because the branch and the files
  of the rest of the Git tab are the agent's own worktree.
  Each file has "Put this file back": it asks once in place, then puts that one file back to the
  checkpoint taken before this turn, when the checkpoints are of the worktree the turn shown was
  read in (rewind.ts `turnPoint`). A toast says what went back, with Undo. After a restore here or
  from "Undo a turn", and after its Undo, the turn is read again.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '../lib/api';
  import type { Agent } from '../lib/gen/Agent';
  import type { LastTurn } from '../lib/gen/LastTurn';
  import { ui } from '../lib/state/ui.svelte';
  import { parseDiff, type DiffFile } from '../lib/util/diff';
  import { ago, plural } from '../lib/util/format';
  import { idVar } from '../lib/util/labels';
  import { oneAtATime } from '../lib/util/oneAtATime';
  import { turnPoint } from '../home/rewind';
  import { putBack } from '../home/rewindLayer.svelte';
  import DiffView from '../rooms/changes/DiffView.svelte';

  interface Props {
    agent: Agent;
    /** Changes each time the Git tab has read the worktree again. */
    refresh: unknown;
  }
  let { agent, refresh }: Props = $props();

  /** The answer of the last read: the turn, or null when Codly kept none. Undefined: not read yet. */
  let turn = $state.raw<LastTurn | null | undefined>(undefined);
  let files = $state.raw<DiffFile[]>([]);
  let error = $state<string | null>(null);
  /** The file whose diff is open. */
  let open = $state<string | null>(null);
  /** The file whose "Put this file back" waits for your OK. */
  let asking = $state<string | null>(null);
  let restoring = $state(false);
  /** Why the last "Put this file back" did nothing, in the core's words. */
  let refused = $state<string | null>(null);
  const resting = $derived(['idle', 'done', 'failed', 'offline'].includes(agent.status));

  const load = oneAtATime(async () => {
    const id = agent.id;
    try {
      const t = await api.lastTurnDiff({ kind: 'agent', id });
      if (id !== agent.id) return;
      turn = t;
      files = t ? parseDiff(t.diff) : [];
      error = null;
    } catch (e) {
      if (id === agent.id) error = e instanceof Error ? e.message : String(e);
    }
  });
  // The Git tab read the worktree again, also after files were put back here or from "Undo a
  // turn" (and their Undo): the turn is read again.
  $effect(() => {
    void refresh;
    untrack(() => void load());
  });

  /** The one file back to the point before the turn shown, from the checkpoints of the same worktree. */
  async function fileBack(t: LastTurn, path: string) {
    restoring = true;
    refused = null;
    try {
      const n = turnPoint(t, await api.agentCheckpoints(agent.id));
      if (n === null) throw new Error('Codly can’t tell which checkpoint this turn is: Checkpoints, below, can put the file back');
      await putBack(agent.id, n, path, t.label);
    } catch (e) {
      refused = e instanceof Error ? e.message : String(e);
    } finally {
      asking = null;
      restoring = false;
    }
  }
</script>

{#if error}
  <p class="small bad">{error}</p>
{:else if turn === undefined}
  <p class="small">Reading the last turn…</p>
{:else if turn === null}
  <p class="small">No turn to show: Codly has no checkpoint of a turn of {agent.name} in the worktree it works in now (its task’s worktree while it is on a task). Checkpoints, below, says what it keeps.</p>
{:else}
  {#if turn.task}<p class="small">{agent.name} is on task {turn.task}: this is its last turn in that task’s worktree, not in its own worktree (the branch on top).</p>{/if}
  <p class="small">{files.length ? plural(files.length, 'file') : 'No file'} changed since the turn that started with “{turn.label}” ({ago(turn.ts, ui.now)}).</p>
  {#if refused}<p class="small bad" role="alert">{refused}</p>{/if}
  <div class="turn">
    {#each files as f (f.path)}
      {@const shown = turn}
      <div class="file">
        {#if asking === f.path}
          <div class="ask">
            <p class="small">Put {f.path} back to before this turn? The conversation stays: {agent.name} is told which file went back.</p>
            <div class="row end">
              <button class="btn sm ghost" disabled={restoring} onclick={() => (asking = null)}>Cancel</button>
              <button class="btn sm primary" disabled={restoring} onclick={() => fileBack(shown, f.path)}>Put this file back</button>
            </div>
          </div>
        {:else}
          <div class="row end">
            <button class="btn sm ghost" disabled={restoring} aria-disabled={!resting} data-tip={resting ? 'Only this file goes back to before this turn' : 'Wait until its turn ends'} onclick={() => { if (resting) asking = f.path; }}>Put this file back</button>
          </div>
        {/if}
        <DiffView file={f} hue={idVar(agent.color)} collapsed={open !== f.path} ontoggle={() => (open = open === f.path ? null : f.path)} />
      </div>
    {/each}
  </div>
{/if}

<style>
  /* Off with a reason: not `disabled`, which would never show its tip (base.css). */
  .btn[aria-disabled='true'] { opacity: 0.42; }
  .small { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .small.bad { color: var(--bad); }
  .turn { display: flex; flex-direction: column; gap: 6px; }
  .file { display: flex; flex-direction: column; gap: 2px; }
  .row { display: flex; align-items: center; gap: 8px; }
  .row.end { justify-content: flex-end; }
  .ask { display: flex; flex-direction: column; gap: 6px; padding: 10px 12px; border-radius: var(--r-md, 10px); background: var(--surface-2); }
</style>
