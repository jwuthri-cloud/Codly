<!--
  Memory files: one markdown file per agent that you can read and edit. Codly adds it to that
  agent's brief at the start of every task (the first 2,000 characters). The core resolves the
  path from the agent id inside its own memory folder; nothing here names a path. Saving asks first.
-->
<script lang="ts">
  import { api } from '../../lib/api';
  import type { MemoryFile } from '../../lib/gen/MemoryFile';
  import { toast } from '../../lib/state/toasts.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { ago } from '../../lib/util/format';
  import { idVar } from '../../lib/util/labels';
  import EmptyState from '../../components/EmptyState.svelte';
  import Portrait from '../../components/Portrait.svelte';

  const agents = $derived(store.agents.filter((a) => !a.archived));
  let agentId = $state<string | null>(null);
  let file = $state<MemoryFile | null>(null);
  let text = $state('');
  let loading = $state(false);
  let saving = $state(false);
  let confirming = $state(false);
  let failed = $state<string | null>(null);
  let switchTo = $state<string | null>(null);

  const agent = $derived(agentId ? store.agentMap.get(agentId) : undefined);
  const dirty = $derived(!!file && text !== file.text);
  const tooBig = $derived(new Blob([text]).size > 64 * 1024);
  /** What the database already holds privately for this agent (read at task start, separately). */
  const privateMemories = $derived(store.memories.filter((m) => m.scope === 'agent' && m.scope_id === agentId && !m.superseded_by));

  async function load(id: string) {
    agentId = id;
    switchTo = null;
    confirming = false;
    loading = true;
    failed = null;
    try {
      const f = await api.memoryFile(id);
      if (agentId === id) {
        file = f;
        text = f.text;
      }
    } catch (e) {
      if (agentId === id) {
        file = null;
        failed = e instanceof Error ? e.message : String(e);
      }
    } finally {
      if (agentId === id) loading = false;
    }
  }
  function pick(id: string) {
    if (id === agentId) return;
    if (dirty) switchTo = id;
    else void load(id);
  }
  $effect(() => {
    if (!agentId && agents.length) void load(agents.find((a) => a.role !== 'lead')?.id ?? agents[0].id);
  });
  async function save() {
    if (!agentId) return;
    saving = true;
    try {
      const f = await api.saveMemoryFile(agentId, text);
      file = f;
      text = f.text;
      confirming = false;
      toast({ kind: 'success', title: `Saved ${agent?.name ?? 'the'} memory file`, body: 'It goes into the next task’s brief.' });
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t save the memory file', body: e instanceof Error ? e.message : String(e) });
    } finally {
      saving = false;
    }
  }
</script>

<div class="fi">
  <nav class="who" aria-label="Agents">
    {#each agents as a (a.id)}
      <button class:on={a.id === agentId} aria-current={a.id === agentId} onclick={() => pick(a.id)} style:--hue={idVar(a.color)}>
        <Portrait agent={a} size={22} />
        <span class="txt"><span class="nm truncate">{a.name}</span><span class="role truncate">{a.title ?? (a.role === 'lead' ? 'Lead' : '')}</span></span>
      </button>
    {:else}
      <p class="none">No agents yet.</p>
    {/each}
  </nav>

  <section class="ed">
    {#if !agent}
      <EmptyState compact title="Pick an agent." body="Each one can have a memory file you write: conventions, preferences, things it should never forget." />
    {:else if failed}
      <EmptyState compact title="Couldn’t open the file." body={failed} />
    {:else}
      <header>
        <h2 class="serif">{agent.name}</h2>
        <code class="path">{file?.path ?? ''}</code>
        <span class="state num">
          {#if loading}Loading…{:else if !file?.exists}Not saved yet — this is a starter{:else}Saved {ago(file.modified_at ?? ui.now, ui.now)}{/if}
        </span>
      </header>
      {#if switchTo}
        <div class="note" role="alert">
          <span>You have unsaved changes to {agent.name}’s file.</span>
          <button class="btn sm danger" onclick={() => load(switchTo!)}>Discard them</button>
          <button class="btn sm ghost" onclick={() => (switchTo = null)}>Keep editing</button>
        </div>
      {/if}
      <textarea class="text selectable" bind:value={text} spellcheck="false" disabled={loading} aria-label="{agent.name}’s memory file" placeholder="# {agent.name}: memory"></textarea>
      <footer>
        {#if confirming}
          <span class="ask">Overwrite <code>{file?.path}</code>? {agent.name} reads it at the start of every task.</span>
          <button class="btn primary sm" onclick={save} disabled={saving}>{saving ? 'Saving…' : 'Save file'}</button>
          <button class="btn ghost sm" onclick={() => (confirming = false)} disabled={saving}>Cancel</button>
        {:else}
          <span class="count num" class:bad={tooBig}>{text.length.toLocaleString()} characters{tooBig ? ' — over the 64 KB limit' : ''}{text.length > 2000 ? ' — only the first 2,000 reach a brief' : ''}</span>
          <span class="grow"></span>
          <button class="btn ghost sm" onclick={() => (text = file?.text ?? '')} disabled={!dirty}>Revert</button>
          <button class="btn primary sm" onclick={() => (confirming = true)} disabled={!dirty || tooBig || loading}>Save…</button>
        {/if}
      </footer>
      {#if privateMemories.length}
        <div class="priv">
          <h3 class="eyebrow">Also remembered privately by {agent.name} <span class="num">{privateMemories.length}</span></h3>
          <ul>{#each privateMemories as m (m.id)}<li class="selectable">{m.text}</li>{/each}</ul>
        </div>
      {/if}
    {/if}
  </section>
</div>

<style>
  .fi { flex: 1; min-height: 0; display: flex; gap: 16px; padding: 0 28px 24px; }
  .who { width: 220px; flex-shrink: 0; display: flex; flex-direction: column; gap: 2px; overflow-y: auto; }
  .who button { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: var(--r-md); text-align: left; box-shadow: inset 2px 0 0 transparent; }
  .who button:hover { background: var(--surface-1); }
  .who button.on { background: var(--surface-2); box-shadow: inset 2px 0 0 var(--hue); }
  .txt { display: flex; flex-direction: column; min-width: 0; }
  .nm { font-size: var(--t-md); font-weight: var(--w-med); color: var(--ink-1); }
  .role { font-size: var(--t-xs); color: var(--ink-3); }
  .none { padding: 8px 10px; font-size: var(--t-sm); color: var(--ink-3); }
  .ed { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; gap: 10px; overflow-y: auto; }
  header { display: flex; align-items: baseline; gap: 12px; flex-wrap: wrap; }
  header h2 { font-size: var(--d-sm); color: var(--ink-1); }
  .path { font-size: 12px; color: var(--ink-3); }
  .state { margin-left: auto; font-size: var(--t-xs); color: var(--ink-3); }
  .text { flex: 1; min-height: 160px; box-shadow: none; resize: none; padding: 14px 16px; border: 0; outline: none; border-radius: var(--r-lg); background: var(--surface-1); font-family: var(--font-mono); font-size: var(--mono); line-height: var(--lh-mono); color: var(--ink-1); }
  .text:focus-visible { box-shadow: 0 0 0 2px var(--focus); }
  footer { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .grow { flex: 1; }
  .count { font-size: var(--t-xs); color: var(--ink-3); }
  .count.bad { color: var(--bad); }
  .ask { flex: 1; font-size: var(--t-sm); color: var(--ink-1); }
  .ask code { font-size: 12px; }
  .note { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-radius: var(--r-md); background: var(--amber-soft); font-size: var(--t-sm); color: var(--ink-1); }
  .note span { flex: 1; }
  .priv { margin-top: 6px; }
  .priv ul { display: flex; flex-direction: column; gap: 4px; margin-top: 6px; }
  .priv li { padding: 7px 12px; border-radius: var(--r-md); background: var(--surface-1); font-size: var(--t-sm); line-height: 18px; color: var(--ink-2); }
</style>
