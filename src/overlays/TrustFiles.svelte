<!--
  The repo's own CLI configuration, file by file: which CLI loads it and what Codly read in it
  (the commands its hooks run, what it allows without asking, the MCP servers it starts). Used
  by the open-repo question and by Settings → Safety. The text comes from the repo: it is shown
  as text, cleaned of control characters.
-->
<script lang="ts">
  import type { WorkspaceTrust } from '../lib/gen/WorkspaceTrust';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { clean } from '../lib/util/sanitize';

  interface Props {
    trust: WorkspaceTrust;
  }
  let { trust }: Props = $props();
</script>

<ul class="files" aria-label="The repo’s own CLI configuration">
  {#each trust.files as f (f.path)}
    <li>
      <span class="head"><code>{clean(f.path)}</code><span class="cli">{PROVIDER_LABEL[f.cli]}</span></span>
      {#each f.what as line, i (i)}<span class="what">{clean(line)}</span>{/each}
    </li>
  {/each}
</ul>
{#if trust.problem}
  <p class="problem" role="status">Codly cannot read all of these files ({clean(trust.problem)}), so this folder cannot be trusted as it is.</p>
{/if}
{#if trust.instructions.length}
  <p class="desc">
    Also in this repo, and not part of this choice: {#each trust.instructions as name (name)}<code class="chip">{name}</code>{/each}. These are
    instructions the agents read. What an agent then does still goes through your approvals.
  </p>
{/if}

<style>
  .files { display: flex; flex-direction: column; max-height: 260px; overflow: auto; border-radius: var(--r-md); background: var(--surface-1); }
  .files li { display: flex; flex-direction: column; gap: 2px; padding: 8px 12px; font-size: var(--t-sm); line-height: var(--lh-sm); }
  .files li + li { box-shadow: inset 0 1px 0 var(--line); }
  .head { display: flex; align-items: baseline; gap: 10px; }
  .head code { min-width: 0; overflow-wrap: anywhere; color: var(--ink-1); }
  .cli { margin-left: auto; flex-shrink: 0; color: var(--ink-3); }
  .what { color: var(--ink-2); overflow-wrap: anywhere; }
  .desc { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  .chip { margin-left: 6px; padding: 1px 6px; border-radius: var(--r-sm); background: var(--surface-1); }
  .problem { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--bad); }
</style>
