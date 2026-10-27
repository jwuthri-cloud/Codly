<!--
  Per-agent tabs under the agent sheet's own sections: Messages (the thread with this agent and
  a reply box), Traces (its tool calls with real durations), Git (its worktree, read-only).
  Only the open tab mounts, so Git runs its commands only while you look at it.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import Section from '../components/Section.svelte';
  import AgentGitTab from './AgentGitTab.svelte';
  import AgentMessagesTab from './AgentMessagesTab.svelte';
  import AgentTracesTab from './AgentTracesTab.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const TABS = [
    { id: 'messages', label: 'Messages' },
    { id: 'traces', label: 'Traces' },
    { id: 'git', label: 'Git' },
  ] as const;
  let tab = $state<(typeof TABS)[number]['id']>('messages');
</script>

<Section id="tabs" title="Messages, traces & git">
  <div class="lenses" role="tablist" aria-label="Agent views">
    {#each TABS as t (t.id)}
      <button role="tab" aria-selected={tab === t.id} onclick={() => (tab = t.id)}>{t.label}</button>
    {/each}
  </div>
  <div class="pane" role="tabpanel">
    {#if tab === 'messages'}
      <AgentMessagesTab {agent} />
    {:else if tab === 'traces'}
      <AgentTracesTab {agent} />
    {:else}
      <AgentGitTab {agent} />
    {/if}
  </div>
</Section>

<style>
  .pane { margin-top: 12px; }
</style>
