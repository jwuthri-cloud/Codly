<!--
  The open-repo question: this repo has configuration that the agent CLIs load by themselves
  (project settings with hooks or allow rules, project MCP servers). Shown once for a repo, and
  again when those files change. Two answers: trust the folder, or open it without the repo's
  own CLI configuration. A worktree whose files are not the ones of your folder is asked about
  here too, with its own files. Closing the question answers nothing: agents start without the
  files, and Settings → Safety → Trust has the same choices. The list of what each CLI does without
  trust also says what does not depend on the choice: a Codex agent's own MCP servers.
-->
<script lang="ts">
  import { store } from '../lib/state/store.svelte';
  import { workspaceTrust } from '../lib/state/workspaceTrust.svelte';
  import { startsOwnCodexServers, trustChoices, withoutTrust } from '../lib/util/workspaceTrust';
  import Dialog from '../components/Dialog.svelte';
  import TrustFiles from './TrustFiles.svelte';
  import TrustWorktree from './TrustWorktree.svelte';

  const t = $derived(workspaceTrust.value);
  const name = $derived(store.workspace?.name ?? 'This repo');
  const codexOwn = $derived(startsOwnCodexServers(store.settings, store.clis));
  let saving = $state(false);

  async function choose(trust: string | null) {
    saving = true;
    await workspaceTrust.choose(trust);
    saving = false;
  }
</script>

{#if t}
  {@const can = trustChoices(t)}
  {@const folder = t.state === 'ask'}
  {@const label = !folder ? 'A worktree has other CLI configuration' : t.changed ? 'This repo’s CLI configuration changed' : 'Trust this folder?'}
  <Dialog {label} onclose={() => workspaceTrust.notNow()} width={640}>
    <div class="body">
      {#if folder}
        <p>
          {name} has its own configuration for the agent CLIs. A CLI loads these files by itself when it starts in this folder, before
          any approval: they can run commands, allow tools without asking and start MCP servers.
          {#if t.changed}They changed since your last choice.{/if}
        </p>
        <TrustFiles files={t.files} problem={t.problem} instructions={t.instructions} />
        <div class="without">
          <p class="eyebrow">Until you trust this folder</p>
          <ul>{#each withoutTrust(t, codexOwn) as line (line)}<li>{line}</li>{/each}</ul>
        </div>
        <p class="desc">Your choice is for these files as they are now, and for agents started after it. If the files change, Codly asks again. You can change it in Settings → Safety → Trust.</p>
        <div class="actions">
          <button class="btn" disabled={saving || !can.without} onclick={() => choose(null)}>Open without the repo’s own CLI configuration</button>
          <button class="btn primary" disabled={saving || !can.trust} onclick={() => choose(can.trust)}>Trust this folder</button>
        </div>
      {/if}
      {#each t.worktrees as w (w.path)}<TrustWorktree worktree={w} />{/each}
    </div>
  </Dialog>
{/if}

<style>
  .body { display: flex; flex-direction: column; gap: 14px; padding: 18px 26px 22px; line-height: var(--lh-md); }
  .without { display: flex; flex-direction: column; gap: 6px; }
  .without ul { display: flex; flex-direction: column; gap: 4px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px; }
</style>
