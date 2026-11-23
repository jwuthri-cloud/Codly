<!--
  Try the work before you merge it: a plan's integration worktree, or a solo agent's worktree.
  Run… shows the command kept for this repo (none the first time) and runs it when you press
  Run on that text; nothing is guessed, and nothing runs by itself. The output is shown while
  the command runs. Stop ends it with everything it started; so do a merge of this work and
  quitting Codly. Open in terminal is Codly's own terminal in that folder.
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { ide } from '../ide/ide.svelte';
  import type { PreviewSource } from '../lib/gen/PreviewSource';
  import { loadPreview, previewKey, previewUi, startPreview, stopPreview } from '../lib/state/preview.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { openExternal } from '../lib/util/markdown';
  import Icon from './Icon.svelte';

  /** While a command runs, its new output is asked for this often. */
  const REFRESH_MS = 1000;

  interface Props {
    /** The plan, or the agent, whose worktree this is. */
    source: PreviewSource;
    /** That worktree: where Open in terminal goes. */
    worktree: string;
    /** The agent whose worktree it is: Codly's editor opens an agent's worktree only. */
    agentId?: string | null;
  }
  let { source, worktree, agentId = null }: Props = $props();

  const key = $derived(previewKey(source));
  const shown = $derived(previewUi.shown[key]);
  const run = $derived(shown?.view.run ?? null);
  const live = $derived(!!run && run.ended === null);
  const port = $derived(shown?.view.port_base ?? null);
  // As values of their own: `shown` and `run` are new objects at every answer.
  const runId = $derived(run?.id ?? null);
  const printed = $derived(run?.printed ?? 0);
  /** The Run… form is open. */
  let asking = $state(false);
  let command = $state('');
  let busy = $state(false);
  /** The ended run whose output you closed. */
  let closed = $state<string | null>(null);
  let log = $state<HTMLPreElement>();
  let field = $state<HTMLInputElement>();

  // Read it when this step appears, and again for another plan or agent. The read is not
  // tracked: it reads `previewUi.shown` and writes it, so a tracked one starts itself again
  // without end.
  $effect(() => {
    void key;
    untrack(() => void loadPreview(source));
  });
  $effect(() => {
    if (!live) return;
    const timer = setInterval(() => void loadPreview(source), REFRESH_MS);
    return () => clearInterval(timer);
  });
  // New output keeps the last line in view only while you are there (measured before the new
  // lines land), so you can scroll back while the command runs. A new run starts at its end.
  let followed: string | null = null;
  $effect.pre(() => {
    void printed;
    const box = untrack(() => log);
    const stick = !box || runId !== followed || box.scrollHeight - box.scrollTop - box.clientHeight < 24;
    followed = runId;
    if (stick) void tick().then(() => log && (log.scrollTop = log.scrollHeight));
  });

  function ask() {
    command = shown?.view.command ?? '';
    asking = true;
    void tick().then(() => field?.focus());
  }
  async function start() {
    busy = true;
    if (await startPreview(source, command)) asking = false;
    busy = false;
  }
  async function stop() {
    busy = true;
    await stopPreview(source);
    busy = false;
  }
  function terminal() {
    ui.terminalCwd = worktree;
    ui.go('terminal');
  }
  function editor() {
    if (agentId) void ide.open(agentId);
  }
</script>

<div class="pv">
  <div class="row">
    <span class="what grow"><Icon name="play" size={12} />Try it before you merge.</span>
    <button class="btn sm" disabled={live || asking} data-tip={live ? 'Stop the command that runs first' : 'Run a command of yours in this worktree'} onclick={ask}>Run…</button>
    <button class="btn sm ghost" onclick={terminal} data-tip="Codly’s terminal, in this worktree"><Icon name="terminal" size={12} />Open in terminal</button>
    {#if agentId}
      <button class="btn sm ghost" onclick={editor} data-tip="Browse and edit this worktree"><Icon name="code" size={12} />Open in editor</button>
    {/if}
  </div>
  {#if asking}
    <form class="row" onsubmit={(e) => { e.preventDefault(); void start(); }}>
      <input class="cmd" bind:this={field} bind:value={command} placeholder="pnpm dev --port $CODLY_PORT_BASE" aria-label="Command to run in this worktree" spellcheck="false" autocapitalize="off" autocomplete="off" disabled={busy} />
      <button class="btn sm primary" type="submit" disabled={busy || !command.trim()}>{busy ? 'Starting…' : 'Run'}</button>
      <button class="btn sm ghost" type="button" onclick={() => (asking = false)}>Cancel</button>
    </form>
    <p class="hint">
      Codly runs this text in the worktree, as your terminal would, and keeps it for this repo. <code>$CODLY_PORT_BASE</code>
      {port === null ? 'is a port' : `is ${port}, a port`} of this worktree’s own: start your dev server on it, so it does not collide with the one of your checkout.
      {source.kind === 'agent'
        ? 'Files the command writes become part of this agent’s work, unless git ignores them.'
        : 'Files the command writes are not merged with the plan. If git does not ignore them, Codly keeps this worktree after the merge.'}
    </p>
  {/if}
  {#if run && run.id !== closed}
    {@const url = run.url}
    {@const id = run.id}
    <div class="row">
      <span class="dot" class:live></span>
      <code class="ran truncate" data-tip={run.command}>{run.command}</code>
      <span class="state">{run.ended ?? 'running'}</span>
      <!-- The address answers only while the command runs: after that it is text, not a link. -->
      {#if url && live}<button class="link truncate" onclick={() => openExternal(url)} data-tip="Open in your browser">{url.replace(/^https?:\/\//, '')}</button>
      {:else if url}<span class="was truncate" data-tip="The command ended: nothing answers at this address now">{url.replace(/^https?:\/\//, '')}</span>{/if}
      {#if live}
        <button class="btn sm" disabled={busy} onclick={stop}><Icon name="stop" size={12} />Stop</button>
      {:else}
        <button class="icon-btn" aria-label="Close the output" data-tip="Close the output" onclick={() => (closed = id)}><Icon name="x" size={13} /></button>
      {/if}
    </div>
    <pre class="log" bind:this={log} aria-label="Output of the command">{shown?.lines.length ? shown.lines.join('\n') : live ? 'No output yet.' : 'No output.'}</pre>
  {/if}
</div>

<style>
  .pv { display: flex; flex-direction: column; gap: 6px; padding: 10px 12px; border-radius: var(--r-md, 10px); background: var(--surface-2); }
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; min-width: 0; }
  .what { display: inline-flex; align-items: center; gap: 6px; min-width: 0; font-size: var(--t-sm); color: var(--ink-2); }
  .grow { flex: 1; }
  .cmd { flex: 1; min-width: 160px; font-family: var(--font-mono); font-size: 12px; }
  .hint { font-size: var(--t-xs); line-height: var(--lh-xs); color: var(--ink-3); }
  .hint code, .ran { font-size: 11.5px; color: var(--ink-1); }
  .ran { flex: 1; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--ink-4); flex-shrink: 0; }
  .dot.live { background: var(--ok); }
  .state { font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; }
  .link { max-width: 45%; text-align: left; font-size: var(--t-sm); color: var(--aurora-m); }
  .link:hover { text-decoration: underline; }
  .was { max-width: 45%; font-size: var(--t-sm); color: var(--ink-3); user-select: text; }
  .log { max-height: 220px; overflow: auto; margin: 0; padding: 8px 10px; border-radius: var(--r-sm); background: var(--surface-1); font-family: var(--font-mono); font-size: 11.5px; line-height: 1.45; color: var(--ink-2); white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
</style>
