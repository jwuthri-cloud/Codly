<!--
  Checkpoints in an agent's Git tab: the files of its worktree as Codly kept them before each
  turn, and "Restore files to this point". A restore changes files only (the agent still
  remembers the later turns, and Codly tells it what was put back), asks first, and can be
  undone: the files as they were just before become the newest point. A point opens to the
  files that differ from it, and each of them can go back alone ("Restore this file"), with the
  same rules.

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
  /** The restore that waits for your OK: a whole point (`path` null), or one of its files. */
  let asking = $state<{ n: number; path: string | null } | null>(null);
  /** The point whose files are listed. */
  let open = $state<number | null>(null);
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

  async function restore(c: Checkpoint, path: string | null) {
    busy = true;
    done = null;
    try {
      const r = await api.restoreCheckpoint(agent.id, c.n, path);
      done = path
        ? `Put ${path} back; the other files stay as they are. To undo, restore the newest point.`
        : `Put ${plural(r.files.length, 'file')} back. To undo, restore the newest point.`;
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
  {:else}
    {#if view.task}<p class="small">{agent.name} is on task {view.task} and works in that task’s worktree: these are the files there, not the ones listed above.</p>{/if}
    {#if !view.checkpoints.length}
      <p class="small">None yet. Codly keeps the files as they are before each turn of {agent.name}, so you can put them back.</p>
    {:else}
      <p class="small">The files before each turn of {agent.name}. A restore changes files only; {agent.name} still remembers the later turns.</p>
      {#if !resting}<p class="small">{agent.name} is working. This list is read again when its turn ends.</p>{/if}
      {#if view.left_out.length}<p class="small">Too big to keep in a checkpoint: {view.left_out.join(', ')}. A restore leaves {view.left_out.length === 1 ? 'it as it is' : 'them as they are'}.</p>{/if}
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
                onclick={() => (asking = { n: c.n, path: null })}><Icon name="refresh" size={12} />Restore files</button>
            </div>
            {#if c.changed}
              <button class="since files-t" aria-expanded={open === c.n} data-tip="Show the files. Each one can go back alone." onclick={() => (open = open === c.n ? null : c.n)}>
                <span class="chev" class:open={open === c.n}><Icon name="chevron-right" size={11} stroke={2} /></span>{plural(c.changed, 'file')} changed since
              </button>
            {:else}
              <span class="since">No file changed since</span>
            {/if}
            {#if open === c.n && c.changed}
              <ul class="files">
                {#each c.files as f (f)}
                  <li class="row">
                    <code class="path truncate" data-tip={f}>{f}</code>
                    <button
                      class="btn sm ghost"
                      disabled={busy || !resting}
                      data-tip={resting ? 'Put only this file back to this point' : 'Wait until its turn ends'}
                      onclick={() => (asking = { n: c.n, path: f })}>Restore this file</button>
                  </li>
                {/each}
                {#if c.changed > c.files.length}<li class="small">… and {c.changed - c.files.length} more. “Restore files” puts all of them back.</li>{/if}
              </ul>
            {/if}
            {#if asking?.n === c.n}
              {@const path = asking.path}
              <div class="ask">
                {#if path}
                  <p class="small">
                    Put this file back to this point? Only this file changes; {agent.name} still remembers the later turns, and Codly tells it which file you put back. You can undo it: the files as they are now become the newest point.
                  </p>
                  <code class="names">{path}</code>
                {:else}
                  <p class="small">
                    Put {plural(c.changed, 'file')} back to this point? Files only; {agent.name} still remembers the later turns, and Codly tells it what you put back. You can undo it: the files as they are now become the newest point.
                  </p>
                  <code class="names">{names(c)}</code>
                {/if}
                <div class="row end">
                  <button class="btn sm ghost" disabled={busy} onclick={() => (asking = null)}>Cancel</button>
                  <button class="btn sm primary" disabled={busy} onclick={() => restore(c, path)}>{path ? 'Restore this file' : 'Restore files to this point'}</button>
                </div>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
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
  .files-t { display: inline-flex; align-items: center; gap: 4px; }
  .files-t:hover { color: var(--ink-1); }
  .chev { display: inline-flex; transition: transform var(--d-2) var(--ease-out); }
  .chev.open { transform: rotate(90deg); }
  .files { display: flex; flex-direction: column; gap: 2px; margin: 2px 0 2px 15px; }
  .path { flex: 1; min-width: 0; font-size: 11.5px; color: var(--ink-1); }
  .ask { display: flex; flex-direction: column; gap: 6px; margin-top: 4px; padding: 10px 12px; border-radius: var(--r-md, 10px); background: var(--surface-2); }
  .names { font-size: 11px; color: var(--ink-2); white-space: pre-wrap; overflow-wrap: anywhere; max-height: 120px; overflow-y: auto; }
</style>
