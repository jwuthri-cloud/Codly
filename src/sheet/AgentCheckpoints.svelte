<!--
  Checkpoints in an agent's Git tab: the files of its worktree as Codly kept them before each
  turn, and "Restore files to this point". A restore changes files only (the agent still
  remembers the later turns, and Codly tells it what was put back), asks first, and can be
  undone: the files as they were just before become the newest point.

  Reading the list starts git in the worktree, so it is read when the tab opens and when the Git
  tab reads the worktree again while the agent rests (its turn ended, you pressed Refresh, files
  were put back), one read at a time. It is not read on each step of a turn: the agent's files
  are half-written then, and nothing can be restored anyway.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '../lib/api';
  import type { Agent } from '../lib/gen/Agent';
  import type { AgentCheckpoints } from '../lib/gen/AgentCheckpoints';
  import type { Checkpoint } from '../lib/gen/Checkpoint';
  import { ui } from '../lib/state/ui.svelte';
  import { ago, plural } from '../lib/util/format';
  import { oneAtATime } from '../lib/util/oneAtATime';
  import Icon from '../components/Icon.svelte';

  interface Props {
    agent: Agent;
    /** Changes each time the Git tab has read the worktree again (see the note on top). */
    refresh: unknown;
    /** Files were put back: the Git tab reads the worktree again. */
    onrestored: () => void;
  }
  let { agent, refresh, onrestored }: Props = $props();

  let view = $state<AgentCheckpoints | null>(null);
  let error = $state<string | null>(null);
  /** The point whose restore waits for your OK. */
  let asking = $state<number | null>(null);
  let busy = $state(false);
  /** What the last restore did, until the next one. */
  let done = $state<string | null>(null);

  const resting = $derived(['idle', 'done', 'failed', 'offline'].includes(agent.status));

  const load = oneAtATime(async () => {
    const id = agent.id;
    try {
      const v = await api.agentCheckpoints(id);
      if (id === agent.id) {
        view = v;
        error = null;
      }
    } catch (e) {
      if (id === agent.id) error = e instanceof Error ? e.message : String(e);
    }
  });
  // Only `refresh` starts a read. The agent object is new on every step of its turn (each tool
  // call, each usage report): that alone must not start git.
  $effect(() => {
    void refresh;
    untrack(() => {
      if (resting || !view) void load();
    });
  });

  const title = (c: Checkpoint) => (c.kind === 'restore' ? `Before you restored “${c.label}”` : c.label);
  const names = (c: Checkpoint) => c.files.join('\n') + (c.changed > c.files.length ? `\n… and ${c.changed - c.files.length} more` : '');

  async function restore(c: Checkpoint) {
    busy = true;
    done = null;
    try {
      const r = await api.restoreCheckpoint(agent.id, c.n);
      done = `Put ${plural(r.files.length, 'file')} back. To undo, restore the newest point.`;
      error = null;
      onrestored();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      void load(); // the point may be gone, or the agent at work again
    } finally {
      asking = null;
      busy = false;
    }
  }
</script>

{#if view}
  <h4 class="eyebrow">Checkpoints{view.off ? '' : ` · ${view.checkpoints.length}`}</h4>
  {#if view.off}
    <p class="small">{view.off}</p>
  {:else if !view.checkpoints.length}
    <p class="small">None yet. Codly keeps the files as they are before each turn of {agent.name}, so you can put them back.</p>
  {:else}
    <p class="small">The files before each turn of {agent.name}. A restore changes files only; {agent.name} still remembers the later turns.</p>
    {#if !resting}<p class="small">{agent.name} is working. This list is read again when its turn ends.</p>{/if}
    <ul class="points">
      {#each view.checkpoints as c (c.n)}
        <li>
          <div class="row">
            <span class="what truncate" data-tip={title(c)}>{title(c)}</span>
            <time>{ago(c.ts, ui.now)}</time>
            <button
              class="btn sm"
              disabled={busy || !resting || !c.changed}
              data-tip={!c.changed ? 'The files are as they were at this point' : resting ? 'Restore files to this point' : 'Wait until its turn ends'}
              onclick={() => (asking = c.n)}><Icon name="refresh" size={12} />Restore files</button>
          </div>
          <span class="since" data-tip={c.changed ? names(c) : null}>{c.changed ? `${plural(c.changed, 'file')} changed since` : 'No file changed since'}</span>
          {#if asking === c.n}
            <div class="ask">
              <p class="small">
                Put {plural(c.changed, 'file')} back to this point? Files only; {agent.name} still remembers the later turns, and Codly tells it what you put back. You can undo it: the files as they are now become the newest point.
              </p>
              <code class="names">{names(c)}</code>
              <div class="row end">
                <button class="btn sm ghost" disabled={busy} onclick={() => (asking = null)}>Cancel</button>
                <button class="btn sm primary" disabled={busy} onclick={() => restore(c)}>Restore files to this point</button>
              </div>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if done}<p class="small">{done}</p>{/if}
{/if}
{#if error}<p class="small bad">{error}</p>{/if}

<style>
  .eyebrow { margin-top: 8px; }
  .small { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .small.bad { color: var(--bad); }
  .points { display: flex; flex-direction: column; gap: 6px; }
  .row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .row.end { justify-content: flex-end; }
  .what { flex: 1; font-size: var(--t-sm); color: var(--ink-1); }
  time, .since { font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; }
  .ask { display: flex; flex-direction: column; gap: 6px; margin-top: 4px; padding: 10px 12px; border-radius: var(--r-md, 10px); background: var(--surface-2); }
  .names { font-size: 11px; color: var(--ink-2); white-space: pre-wrap; overflow-wrap: anywhere; max-height: 120px; overflow-y: auto; }
</style>
