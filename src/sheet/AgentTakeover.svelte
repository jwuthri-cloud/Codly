<!--
  "Open in my terminal": continue an agent's session yourself, in its CLI's own terminal UI.
  The core owns the hand-over (orchestrator/takeover.rs): it says whether the agent may go,
  stops its own process for the session first, builds the command, and starts nothing for the
  agent until "Give it back". This card only asks and shows. Elsewhere the agent reads "In your
  terminal" (narration), from the core's list of every agent that is there (Shell.svelte).
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import { copyText } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import { giveBack, loadTakeover, openInTerminal, showTerminal, takeovers } from '../lib/state/takeover.svelte';
  import { clock } from '../lib/util/format';
  import { PROVIDER_LABEL } from '../lib/util/labels';
  import { closeSession, takeoverKey } from '../rooms/terminal/sessions';
  import Icon from '../components/Icon.svelte';
  import Section from '../components/Section.svelte';

  interface Props {
    agent: Agent;
  }
  let { agent }: Props = $props();

  /** What the core said last about this agent (null: not asked yet). */
  const known = $derived(takeovers.of(agent.id));
  const there = $derived(known?.takeover ?? null);
  const cli = $derived(PROVIDER_LABEL[agent.provider]);
  /** The core's reason, as a sentence. */
  const why = $derived.by(() => {
    const b = known?.blocked;
    return b ? `${b[0].toUpperCase()}${b.slice(1)}${/[.!?]$/.test(b) ? '' : '.'}` : null;
  });

  // The core's answer follows what the agent does: ask again when it starts or ends a turn,
  // its process starts or ends, it gets a session, or its task moves.
  const resting = $derived(['idle', 'done', 'failed', 'offline'].includes(agent.status));
  const taskStatus = $derived(agent.task_id ? store.taskMap.get(agent.task_id)?.status : undefined);
  $effect(() => {
    void [resting, agent.run_id, agent.session_id, agent.task_id, taskStatus, agent.archived];
    void loadTakeover(agent.id);
  });

  let busy = $state(false);
  async function open() {
    busy = true;
    await openInTerminal(agent.id);
    busy = false;
  }
  async function back() {
    busy = true;
    // The core closed the terminal it opened: its tab here goes with it.
    if (await giveBack(agent.id)) await closeSession(takeoverKey(agent.id));
    busy = false;
  }
</script>

<Section id="takeover" title="Your terminal" summary={there ? 'in your terminal' : 'not in use'}>
  {#if there}
    <div class="in">
      <p class="state"><Icon name="terminal" size={13} /><span>{agent.name} is in your terminal since {clock(there.since)}. Codly sends it nothing until you give it back.</span></p>
      <code class="cmd selectable">{there.command}</code>
      <div class="acts">
        <button class="btn sm" onclick={() => showTerminal(agent.id)}><Icon name="terminal" size={12} />Show terminal</button>
        <button class="btn sm ghost" onclick={() => copyText(there.copy_command, 'Command copied')}><Icon name="copy" size={12} />Copy command</button>
        <button class="btn sm primary" disabled={busy} onclick={back}>Give it back</button>
      </div>
      <p class="sub">The command is typed in Codly’s terminal: press Enter there to start {cli}. For a terminal of your own, copy the command. Quit {cli} before you give {agent.name} back: two programs on one session damage it. Give it back closes Codly’s terminal; the next message then resumes the same session here.</p>
    </div>
  {:else}
    <div class="row">
      <span class="txt">
        <b>Open in my terminal</b>
        <span class="sub">{why ?? `Continue ${agent.name}’s session yourself in ${cli}. Codly stops its own process for it first, and sends it nothing until you give it back.`}</span>
      </span>
      <button class="btn sm" disabled={!known || !!known.blocked || busy} onclick={open}><Icon name="terminal" size={12} />Open</button>
    </div>
  {/if}
</Section>

<style>
  .row { display: flex; align-items: center; gap: 12px; }
  .txt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .txt b { font-weight: var(--w-med); font-size: var(--t-md); color: var(--ink-1); }
  .sub { font-size: var(--t-xs); line-height: 16px; color: var(--ink-3); }
  .in { display: flex; flex-direction: column; gap: 8px; }
  .state { display: flex; align-items: flex-start; gap: 6px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-1); }
  .state :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .cmd { font-size: 11.5px; padding: 5px 8px; border-radius: 6px; background: var(--surface-2); color: var(--ink-2); width: fit-content; max-width: 100%; overflow-x: auto; white-space: nowrap; }
  .acts { display: flex; flex-wrap: wrap; gap: 6px; }
</style>
