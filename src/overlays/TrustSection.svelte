<!--
  Settings → Safety: the open repo's own CLI configuration (project settings with hooks or allow
  rules, project MCP servers) and your choice about it. The same two choices as the open-repo
  question; a choice is for the files as they are now and for agents started after it. Below
  them: each worktree whose files are not the ones of your folder, to trust by itself.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { store } from '../lib/state/store.svelte';
  import { workspaceTrust } from '../lib/state/workspaceTrust.svelte';
  import { startsOwnCodexServers, trustChoices, trustStatus, withoutTrust } from '../lib/util/workspaceTrust';
  import LoadError from '../components/LoadError.svelte';
  import MoreAbout from './MoreAbout.svelte';
  import TrustFiles from './TrustFiles.svelte';
  import TrustWorktree from './TrustWorktree.svelte';

  const t = $derived(workspaceTrust.value);
  const codexOwn = $derived(startsOwnCodexServers(store.settings, store.clis));
  let loading = $state(false);

  async function load() {
    loading = true;
    await workspaceTrust.check(false);
    loading = false;
  }
  // The files may have changed since the repo was opened.
  onMount(() => void load());
</script>

{#if store.workspace}
  <section>
    <h3 class="eyebrow">This folder’s own CLI configuration</h3>
    <p class="desc">
      A repo can carry configuration that an agent CLI loads by itself when it starts there, before any approval: project settings with
      hooks or allow rules, and project MCP servers.
    </p>
    <MoreAbout id="trust">
      <p class="desc">
        Codly lets the CLIs load the files that git tracks in {store.workspace.name} only once you trusted them. The choice is about the repo’s
        files: your own files that Codly copies into a new worktree stay yours there.
      </p>
    </MoreAbout>
    {#if t}
      {@const can = trustChoices(t)}
      <p class="status" class:warn={t.state === 'ask'} role="status">{trustStatus(t)}</p>
      {#if t.state !== 'nothing'}
        <TrustFiles files={t.files} problem={t.problem} instructions={t.instructions} />
        {#if t.state !== 'trusted'}
          <ul class="desc">{#each withoutTrust(t, codexOwn) as line (line)}<li>{line}</li>{/each}</ul>
        {/if}
        <div class="choices">
          {#if can.trust}<button class="btn sm" onclick={() => workspaceTrust.choose(can.trust)}>Trust this folder</button>{/if}
          {#if can.without}<button class="btn sm" onclick={() => workspaceTrust.choose(null)}>Open without the repo’s own CLI configuration</button>{/if}
        </div>
        <p class="desc">A choice applies to agents started after it. If the files change, Codly asks again.</p>
      {/if}
      {#each t.worktrees as w (w.path)}<TrustWorktree worktree={w} />{/each}
    {:else if workspaceTrust.error}
      <LoadError what="this repo’s own CLI configuration" error={workspaceTrust.error} {loading} onretry={load} />
    {:else}
      <p class="desc">Loading…</p>
    {/if}
  </section>
{/if}

<style>
  section { padding: 20px 0; box-shadow: inset 0 -1px 0 var(--line); display: flex; flex-direction: column; gap: 12px; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  ul.desc { display: flex; flex-direction: column; gap: 4px; }
  .status { font-weight: var(--w-med); }
  .status.warn { color: var(--bad); }
  .choices { display: flex; flex-wrap: wrap; gap: 8px; }
</style>
