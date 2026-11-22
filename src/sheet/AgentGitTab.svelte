<!-- Git: the agent's worktree. Changed files, local branches, recent commits; a solo agent's branch merges into yours from here, and a conflict with your branch can go back to the agent. -->
<script lang="ts">
  import { api } from '../lib/api';
  import type { Agent } from '../lib/gen/Agent';
  import type { AgentGit } from '../lib/gen/AgentGit';
  import { ui } from '../lib/state/ui.svelte';
  import { ago } from '../lib/util/format';
  import { mergeAgent } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import Icon from '../components/Icon.svelte';
  import MergeConflictStep from '../components/MergeConflictStep.svelte';
  import RepoSyncBar from '../components/RepoSyncBar.svelte';
  import PullRequestStep from '../components/PullRequestStep.svelte';
  import AgentCheckpoints from './AgentCheckpoints.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  let view = $state<AgentGit | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  async function load(id: string) {
    busy = true;
    try {
      const v = await api.agentGit(id);
      if (id === agent.id) {
        view = v;
        error = null;
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
  // Load when opened, and again when the agent changes or its status does (it just did something).
  $effect(() => {
    void agent.status;
    void load(agent.id);
  });

  const target = $derived(store.workspace?.current_branch ?? store.workspace?.default_branch ?? 'your branch');
  const resting = $derived(['idle', 'done', 'failed', 'offline'].includes(agent.status));
  let merging = $state(false);
  let merges = $state(0);
  async function merge() {
    merging = true;
    if (await mergeAgent(agent.id)) merges++;
    merging = false;
    void load(agent.id);
  }

  const STATE: Record<string, string> = { M: 'modified', A: 'added', D: 'deleted', R: 'renamed', C: 'copied', U: 'conflict', '?': 'new' };
  const word = (code: string) => STATE[code.trim()[0] ?? ''] ?? 'changed';
</script>

<div class="git">
  <div class="top">
    <span class="br"><Icon name="branch" size={12} /><code>{view?.branch ?? (view?.worktree ? 'detached HEAD' : 'no worktree')}</code></span>
    <span class="acts">
      {#if agent.role === 'solo' && view?.branch}
        <button class="btn sm primary" disabled={merging || !resting} onclick={merge} data-tip={resting ? `Merge ${view.branch} into ${target}` : 'Wait until its turn ends'}><Icon name="merge" size={12} />Merge into {target}</button>
      {/if}
      <button class="icon-btn" aria-label="Refresh" data-tip="Refresh" disabled={busy} onclick={() => load(agent.id)}><Icon name="refresh" size={13} /></button>
    </span>
  </div>
  {#if agent.role === 'solo' && view?.branch}<MergeConflictStep source={{ kind: 'agent', id: agent.id }} refresh={`${merges}:${agent.status}`} who={agent.name} wait={resting ? null : 'Wait until its turn ends'} />{/if}
  {#if agent.role === 'solo'}<RepoSyncBar refresh={merges} agentId={agent.id} />{/if}
  <!-- Only for Codly's own branch: the core pushes no other one from here. -->
  {#if agent.role === 'solo' && view?.branch?.startsWith('codly/')}<PullRequestStep source={{ kind: 'agent', id: agent.id }} wait={resting ? null : 'Wait until its turn ends'} />{/if}
  {#if error}
    <p class="small bad">{error}</p>
  {:else if !view}
    <p class="small">Reading the worktree…</p>
  {:else if !view.worktree || (view.note && !view.log.length && !view.files.length && !view.branches.length)}
    <p class="small">{view.note ?? 'This agent has no worktree.'}</p>
  {:else}
    {#if view.note}<p class="small bad">{view.note}</p>{/if}
    <h4 class="eyebrow">Changes · {view.files.length}</h4>
    {#if view.files.length}
      <ul class="files">
        {#each view.files as f (f.path)}
          <li><button class="file" onclick={() => ui.selectFile(f.path)}><code class="code">{f.code.replace(' ', '·')}</code><span class="path truncate">{f.path}</span><span class="st">{word(f.code)}</span></button></li>
        {/each}
      </ul>
    {:else}
      <p class="small">Working tree clean.</p>
    {/if}
    {#if view.branches.length}
      <h4 class="eyebrow">Branches · {view.branches.length}</h4>
      <ul class="brs">
        {#each view.branches as b (b.name)}
          <li class:cur={b.current}><span class="dot"></span><code class="truncate">{b.name}</code><code class="sha">{b.sha}</code><time class="num">{ago(b.ts, ui.now)}</time></li>
        {/each}
      </ul>
    {/if}
    <h4 class="eyebrow">Recent commits</h4>
    {#if view.log.length}
      <ul class="log">
        {#each view.log as c (c.sha + c.ts)}
          <li><code class="sha">{c.sha}</code><span class="sub truncate" data-tip="{c.author}: {c.subject}">{c.subject}</span><time class="num">{ago(c.ts, ui.now)}</time></li>
        {/each}
      </ul>
    {:else}
      <p class="small">No commits yet.</p>
    {/if}
  {/if}
  {#if view?.worktree}<AgentCheckpoints {agent} refresh={view} onrestored={() => load(agent.id)} />{/if}
</div>

<style>
  .git { display: flex; flex-direction: column; gap: 6px; }
  .top { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .acts { display: inline-flex; align-items: center; gap: 6px; flex: none; }
  .br { display: inline-flex; align-items: center; gap: 6px; min-width: 0; color: var(--ink-3); }
  .br code { font-size: 11.5px; color: var(--ink-1); overflow-wrap: anywhere; }
  .eyebrow { margin-top: 8px; }
  .small { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .small.bad { color: var(--bad); }
  ul { display: flex; flex-direction: column; }
  .file { display: flex; align-items: center; gap: 8px; width: 100%; padding: 3px 6px; margin: 0 -6px; border-radius: var(--r-sm); text-align: left; font-size: var(--t-sm); }
  .file:hover { background: var(--surface-2); }
  .code { width: 18px; font-size: 11px; color: var(--hue, var(--ink-2)); }
  .path { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-1); }
  .st { font-size: var(--t-xs); color: var(--ink-3); }
  .brs li, .log li { display: flex; align-items: center; gap: 8px; min-height: 24px; font-size: var(--t-sm); color: var(--ink-2); }
  .brs code { font-size: 11.5px; }
  .brs li > code:first-of-type { flex: 1; min-width: 0; }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--ink-4); flex-shrink: 0; }
  .cur .dot { background: var(--ok); }
  .cur code { color: var(--ink-1); }
  .sha { font-size: 11px; color: var(--ink-3); flex-shrink: 0; }
  .sub { flex: 1; min-width: 0; }
  time { font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; }
</style>
