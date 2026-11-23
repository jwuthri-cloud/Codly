<!--
  The commands the agent left running in the background in its live run (Claude Code's
  `run_in_background`): they go on after the turn that started them. "Stop" ends one
  (orchestrator/live_session.rs); the command then shows as stopped, in Claude Code's own word.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { backgroundSummary, ofLiveRun, stopBackgroundCommand } from '../lib/state/session';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { dur, timer } from '../lib/util/format';
  import { clean } from '../lib/util/sanitize';
  import Icon from '../components/Icon.svelte';
  import Provenance from '../components/Provenance.svelte';
  import Section from '../components/Section.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  const commands = $derived.by(() => {
    void store.revBy.get(agent.id);
    return ofLiveRun(store.live[agent.id]?.background ?? [], agent.run_id);
  });
  const running = $derived(commands.filter((c) => c.status === 'running'));
  const ended = $derived(commands.filter((c) => c.status !== 'running'));
  /** The commands a stop is on its way for (by the CLI's task id), until Claude Code answered. Its
   *  answer does not say whether the command ended: the command's own event does, and moves the row. */
  let stopping = $state<string[]>([]);
  async function stop(taskId: string) {
    stopping = [...stopping, taskId];
    await stopBackgroundCommand(agent.id, taskId);
    stopping = stopping.filter((id) => id !== taskId);
  }
</script>

{#if commands.length}
  <Section id="background" title="Background commands" summary={backgroundSummary(commands)}>
    <ul class="cmds">
      {#each running as c (c.taskId)}
        <li>
          <span class="dot on" aria-hidden="true"></span>
          <code class="what truncate selectable" data-tip={clean(c.description)}>{clean(c.description)}</code>
          <span class="meta num">{timer(ui.now - c.startedAt)}</span>
          <button class="btn sm ghost" disabled={stopping.includes(c.taskId)} onclick={() => stop(c.taskId)}><Icon name="stop" size={12} />{stopping.includes(c.taskId) ? 'Stopping…' : 'Stop'}</button>
        </li>
      {/each}
      {#each ended as c (c.taskId)}
        <li class="fin">
          <Icon name={c.status === 'completed' ? 'check' : 'x-circle'} size={13} />
          <code class="what truncate selectable" data-tip={clean(c.description)}>{clean(c.description)}</code>
          <span class="meta num">{c.status} · {dur(c.ts - c.startedAt)}</span>
        </li>
      {/each}
    </ul>
    <p class="src"><Provenance kind="observed" /> as reported by Claude Code</p>
  </Section>
{/if}

<style>
  .cmds { display: flex; flex-direction: column; gap: 8px; }
  .cmds li { display: flex; align-items: center; gap: 10px; min-width: 0; min-height: 24px; color: var(--ink-2); }
  .cmds li > :global(svg) { flex-shrink: 0; color: var(--ink-3); }
  .dot { flex: none; width: 8px; height: 8px; margin: 0 2px; border-radius: 50%; background: var(--ink-4); }
  .dot.on { background: var(--live); box-shadow: 0 0 0 3px var(--live-soft); }
  .what { flex: 1; min-width: 0; font-size: 11.5px; color: var(--ink-1); }
  .fin .what { color: var(--ink-3); }
  .meta { font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; }
  .src { margin-top: 12px; display: flex; align-items: center; gap: 6px; font-size: var(--t-xs); color: var(--ink-3); }
</style>
