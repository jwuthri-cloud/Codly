<!--
  What the agent left at work in the background in its live run, as Claude Code lists it: the
  commands it started with `run_in_background` and its helpers. They go on after the turn that
  started them. "Stop" ends a command or a helper (orchestrator/live_session.rs); Claude Code's
  next list no longer names it, and the row shows how it ended, in Claude Code's own word. After
  the stop of a helper Claude Code starts a turn of the agent by itself (recorded:
  docs/research/cli-claude-stream-json-2.1.311-part3.md, section 1), and the button says so.
  A row whose start is not known (session.ts: its first list came from a snapshot) shows
  "running" with no timer. When a row you stopped leaves the running ones, the list closes up
  under the pointer: its "Stop" buttons stay off for a moment (`stopsSettling`), so a second
  press never stops the next row.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { backgroundKind, backgroundSummary, canStop, ofLiveRun, stopBackgroundTask, stopHint, stopLabel, stopOnItsWay, stopsSettling, type BackgroundWork } from '../lib/state/session';
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

  const work = $derived.by(() => {
    void store.revBy.get(agent.id);
    return ofLiveRun(store.live[agent.id]?.background ?? [], agent.run_id);
  });
  const running = $derived(work.filter((w) => w.status === 'running'));
  const ended = $derived(work.filter((w) => w.status !== 'running'));
  /** The work a stop is on its way for, by the CLI's task id: `sent` until Claude Code answers,
   *  then `taken` (with when it took the stop). Its answer does not say whether the work ended:
   *  its next list does, and moves the row. Until then the button reads "Stopping…"
   *  (`stopOnItsWay`); a request Claude Code did not take gives "Stop" back at once. */
  let sent = $state<string[]>([]);
  let taken = $state<Record<string, number>>({});
  const stopping = (w: BackgroundWork) => sent.includes(w.id) || stopOnItsWay(taken[w.id], ui.now);
  const settling = $derived(stopsSettling(ended, new Set([...sent, ...Object.keys(taken)]), ui.now));
  async function stop(w: BackgroundWork) {
    sent = [...sent, w.id];
    const took = await stopBackgroundTask(agent.id, w);
    sent = sent.filter((id) => id !== w.id);
    if (took) taken = { ...taken, [w.id]: ui.now };
  }
</script>

{#if work.length}
  <Section id="background" title="In the background" summary={backgroundSummary(work)}>
    <ul class="cmds">
      {#each running as w (w.id)}
        {@const kind = backgroundKind(w)}
        <li>
          <span class="dot on" aria-hidden="true"></span>
          <code class="what truncate selectable" data-tip={clean(w.description)}>{clean(w.description)}</code>
          <span class="meta num">{kind ? `${kind} · ` : ''}{w.startedAt == null ? 'running' : timer(ui.now - w.startedAt)}</span>
          {#if canStop(w)}
            <button class="btn sm ghost" disabled={stopping(w) || settling} aria-label={stopLabel(w)} data-tip={stopHint(w, agent.name)} onclick={() => stop(w)}><Icon name="stop" size={12} />{stopping(w) ? 'Stopping…' : 'Stop'}</button>
          {/if}
        </li>
      {/each}
      {#each ended as w (w.id)}
        {@const kind = backgroundKind(w)}
        <li class="fin">
          <Icon name={w.status === 'completed' ? 'check' : 'x-circle'} size={13} />
          <code class="what truncate selectable" data-tip={clean(w.description)}>{clean(w.description)}</code>
          <span class="meta num">{kind ? `${kind} · ` : ''}{w.status}{w.startedAt != null && w.endedAt != null ? ` · ${dur(w.endedAt - w.startedAt)}` : ''}</span>
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
