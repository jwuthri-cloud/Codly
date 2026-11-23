<!--
  Per-agent tabs under the agent sheet's own sections: Messages (the thread with this agent and
  a reply box), Traces (its tool calls with real durations), Git (its worktree; the one thing it
  changes is "Restore files to this point" on a checkpoint).
  Only the open tab mounts, so Git runs its commands only while you look at it. The open tab is
  `ui.sheetTab`: a focus on a place in a tab opens it ("/undo" lands on the Git tab's
  checkpoints: sheetFocus.ts).
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { ui } from '../lib/state/ui.svelte';
  import Section from '../components/Section.svelte';
  import AgentGitTab from './AgentGitTab.svelte';
  import AgentMessagesTab from './AgentMessagesTab.svelte';
  import AgentTracesTab from './AgentTracesTab.svelte';
  import type { SheetTab } from './sheetFocus';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const TABS = [
    { id: 'messages', label: 'Messages' },
    { id: 'traces', label: 'Traces' },
    { id: 'git', label: 'Git' },
  ] as const satisfies readonly { id: SheetTab; label: string }[];
</script>

<Section id="tabs" title="Messages, traces & git">
  <div class="lenses" role="tablist" aria-label="Agent views">
    {#each TABS as t (t.id)}
      <button role="tab" aria-selected={ui.sheetTab === t.id} onclick={() => (ui.sheetTab = t.id)}>{t.label}</button>
    {/each}
  </div>
  <div class="pane" role="tabpanel">
    {#if ui.sheetTab === 'messages'}
      <AgentMessagesTab {agent} />
    {:else if ui.sheetTab === 'traces'}
      <AgentTracesTab {agent} />
    {:else}
      <AgentGitTab {agent} />
    {/if}
  </div>
</Section>

<style>
  .pane { margin-top: 12px; }
</style>
