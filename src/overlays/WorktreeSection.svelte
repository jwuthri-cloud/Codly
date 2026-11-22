<!--
  Settings → Safety: what a new worktree of the open repo gets. Codly copies git-ignored files
  (the built-in list plus the repo's `.worktreeinclude`) and can run the repo's setup script,
  `.codly/setup.sh`. The script is code from the repo: off until you turn it on here, where its
  text and what the approval rules see in it are shown. Turning it on is for that exact text.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../lib/api';
  import type { WorktreeSetup } from '../lib/gen/WorktreeSetup';
  import { Loader } from '../lib/state/loader.svelte';
  import { store } from '../lib/state/store.svelte';
  import { toast } from '../lib/state/toasts.svelte';
  import LoadError from '../components/LoadError.svelte';

  const setup = new Loader<WorktreeSetup>(() => api.worktreeSetup());
  const s = $derived(setup.value);
  onMount(() => {
    if (store.workspace) void setup.load();
  });

  /** The switch shows what the core saved; a refusal (the file changed meanwhile) reloads it. */
  async function turn(on: boolean) {
    if (!s) return;
    try {
      setup.set(await api.worktreeSetup({ allow: on ? s.sha : null }));
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t change the setup script', body: e instanceof Error ? e.message : String(e) });
      void setup.load();
    }
  }
</script>

{#if store.workspace}
  <section>
    <h3 class="eyebrow">New worktrees</h3>
    <p class="desc">
      Each agent works in its own copy of {store.workspace.name} (a git worktree). git puts only tracked files there, so Codly also copies
      what git ignores and an agent needs: <code>node_modules</code>, <code>.venv</code>, <code>venv</code> and the <code>.env</code> files at the top of the repo.
      To copy more, list it in a <code>.worktreeinclude</code> file at the top of the repo (same syntax as <code>.gitignore</code>; only ignored files are copied).
    </p>
    {#if s}
      {#if s.include.length}
        <p class="desc">.worktreeinclude adds: {#each s.include as line, i (i)}<code class="chip">{line}</code>{/each}</p>
      {/if}
      <label class="row toggle">
        <div class="r-text">
          <span class="r-title">Run the repo’s setup script</span>
          <span class="desc">
            {#if s.script === null}
              This repo has no <code>.codly/setup.sh</code>. Add one to install packages or write local settings in each new worktree.
            {:else}
              Codly runs <code>.codly/setup.sh</code> from your checkout once in each new worktree, before the agent starts (10 minute limit). It can read
              <code>$CODLY_PORT_BASE</code> (10 ports of its own) and <code>$CODLY_REPO_PATH</code> (your checkout). If it fails, the task waits for you.
            {/if}
          </span>
        </div>
        <input type="checkbox" class="switch" checked={s.on} disabled={s.script === null} aria-label="Run the repo’s setup script" onchange={(e) => turn(e.currentTarget.checked)} />
      </label>
      {#if s.script !== null}
        {#if s.changed}<p class="changed" role="status">The script changed since you turned it on. Codly does not run it, and new tasks wait, until you read it and turn it on again.</p>{/if}
        <pre class="script" aria-label="The setup script">{s.script}</pre>
        <p class="desc">
          This is code from the repo, and once it is on it runs without asking again.
          {#if s.risks.length}The approval rules see in it: {s.risks.join(', ')}.{:else}The approval rules see no risk in it.{/if}
          Turning it on is for this text only: if the file changes, Codly asks you again.
        </p>
      {/if}
    {:else if setup.error}
      <LoadError what="the worktree setup" error={setup.error} loading={setup.loading} onretry={() => setup.load()} />
    {:else}
      <p class="desc">Loading…</p>
    {/if}
  </section>
{/if}

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .row { display: flex; align-items: center; gap: 16px; min-height: 42px; }
  .r-text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .r-title { font-weight: var(--w-med); }
  .chip { margin-left: 6px; padding: 1px 6px; border-radius: var(--r-sm); background: var(--surface-1); }
  .changed { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--bad); }
  .script { max-height: 220px; overflow: auto; padding: 10px 12px; border-radius: var(--r-md); background: var(--surface-1); font-size: var(--t-sm); line-height: var(--lh-sm); white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
