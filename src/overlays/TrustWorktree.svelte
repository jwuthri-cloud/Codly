<!--
  One worktree whose CLI configuration is not what you decided about for your project folder:
  an agent changed it there, or the worktree holds other files. Agents that start there get
  none of it until you trust these files. Used by the open-repo question and by Settings →
  Safety.
-->
<script lang="ts">
  import type { WorktreeTrust } from '../lib/gen/WorktreeTrust';
  import { workspaceTrust } from '../lib/state/workspaceTrust.svelte';
  import { clean } from '../lib/util/sanitize';
  import TrustFiles from './TrustFiles.svelte';

  interface Props {
    worktree: WorktreeTrust;
  }
  let { worktree }: Props = $props();
  let saving = $state(false);

  async function trust() {
    saving = true;
    await workspaceTrust.choose(worktree.fingerprint);
    saving = false;
  }
</script>

<div class="worktree">
  <p>
    A worktree has CLI configuration that is not what you chose for your project folder: an agent changed it there, or the worktree
    holds other files. Agents that start in this worktree get none of it until you trust these files, and a CLI that Codly cannot
    start without them is not started there.
  </p>
  <code class="path">{clean(worktree.path)}</code>
  <TrustFiles files={worktree.files} problem={worktree.problem} />
  {#if !worktree.problem}
    <div class="actions"><button class="btn sm" disabled={saving} onclick={trust}>Trust these files</button></div>
  {/if}
</div>

<style>
  .worktree { display: flex; flex-direction: column; gap: 8px; font-size: var(--t-sm); line-height: var(--lh-sm); }
  .path { overflow-wrap: anywhere; color: var(--ink-2); }
  .actions { display: flex; justify-content: flex-end; }
</style>
