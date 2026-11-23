<!--
  "Last turn" in an agent's Git tab: what its last turn changed in the files, from the point
  Codly kept before that turn to the files as they are now, committed or not. Each file opens to
  its diff. It is read when you pick it and when the Git tab reads the worktree again (the turn
  ended, you pressed Refresh, files were put back), one read at a time; while the agent works,
  Refresh shows the turn so far.
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
  $effect(() => {
    void refresh;
    untrack(() => void load());
  });
</script>

{#if error}
  <p class="small bad">{error}</p>
{:else if turn === undefined}
  <p class="small">Reading the last turn…</p>
{:else if turn === null}
  <p class="small">No turn to show: Codly has no checkpoint of a turn of {agent.name} in this worktree. Checkpoints, below, says what it keeps.</p>
{:else}
  <p class="small">{files.length ? plural(files.length, 'file') : 'No file'} changed since the turn that started with “{turn.label}” ({ago(turn.ts, ui.now)}).</p>
  <div class="turn">
    {#each files as f (f.path)}
      <DiffView file={f} hue={idVar(agent.color)} collapsed={open !== f.path} ontoggle={() => (open = open === f.path ? null : f.path)} />
    {/each}
  </div>
{/if}

<style>
  .small { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .small.bad { color: var(--bad); }
  .turn { display: flex; flex-direction: column; gap: 6px; }
</style>
