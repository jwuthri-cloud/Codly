<!--
  One fixed-height row above the composer (so nothing jumps): while the thread's agent works it
  says what it is doing (and how many of your messages wait for it); otherwise it offers the next
  sensible actions for this request as small chips (a finished plan's merge among them, while
  nothing stands in its way: its card may be far above the end). When the agent keeps its own todo list, a
  small count ("3/7") follows its name; a press opens the whole list over the row. While it waits
  for you in the middle of its list, the row says the step it waits in, greyed. While its
  turn runs, Stop interrupts it; so does Esc pressed twice (after the first press the row asks
  for the second).
  New request (⌘N) sits on the right once something was said here (in a new request it would
  open another empty one). When you come back and the summary of your time away is above the
  view, a chip at its start takes you to it.
-->
<script lang="ts">
  import { mergePlan, sendChat, stopThread } from '../lib/state/actions';
  import { groupSame } from '../lib/state/approvalGroups';
  import { mergeBlock, mergeKey, mergeUi } from '../lib/state/mergeState.svelte';
  import { narrate } from '../lib/state/narration';
  import { busy } from '../lib/state/queue';
  import { stopping } from '../lib/state/stops';
  import { store } from '../lib/state/store.svelte';
  import { isRequestThread, ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';
  import Menu, { type MenuItem } from '../components/Menu.svelte';
  import Say from '../components/Say.svelte';
  import { awayFor } from './away';
  import { awayShown } from './awayShown.svelte';
  import { reviewLabel } from './threadWords';
  import { todoLine, todoWords } from './todoLine';

  interface Props {
    thread: string;
    /** Your messages still waiting for this agent. */
    queued?: number;
  }
  let { thread, queued = 0 }: Props = $props();
  const agent = $derived(store.threadAgent(thread));
  const n = $derived(agent ? narrate(agent.id) : null);
  const working = $derived(n?.tone === 'live');
  const isRequest = $derived(isRequestThread(thread));
  /** A turn is running (also while it waits for your approval): Stop interrupts it. */
  const running = $derived(busy(thread));
  const halting = $derived(!!agent && stopping(agent.id));
  /** The first Esc on this thread's running turn: the row asks for the second one. */
  const escAsked = $derived(ui.escAsk?.thread === thread && running && !halting);

  // ── its own todo list: "3/7", and the whole list on a press ──
  const todo = $derived.by(() => {
    if (!agent) return [];
    void store.revBy.get(agent.id);
    return store.live[agent.id]?.todo ?? [];
  });
  const line = $derived(todoLine(todo));
  let listOpen = $state(false);
  let countBtn: HTMLButtonElement | undefined = $state();
  $effect(() => {
    if (!working || !line) listOpen = false;
  });
  const steps = $derived.by((): MenuItem[] => {
    if (!agent || !line) return [];
    const id = agent.id;
    return [
      { kind: 'header', label: `${agent.name}’s list · ${line.done} of ${line.total} done` },
      ...todo.map((s, i): MenuItem => ({
        label: s.text,
        icon: s.status === 'completed' ? 'check' : i === line.at ? 'arrow-right' : undefined,
        hint: s.status === 'completed' ? 'Done' : i === line.at ? 'Now' : undefined,
        run: () => ui.selectAgent(id),
      })),
    ];
  });

  /** Nothing was said in this request yet. */
  const empty = $derived(!store.chats.some((c) => c.thread === thread) && !store.outbox.some((o) => o.thread === thread));

  type Chip = { label: string; icon?: string; kbd?: string; primary?: boolean; run: () => void };
  const chips = $derived.by((): Chip[] => {
    if (!isRequest) return [];
    void store.rev;
    const plan = store.planOf(thread);
    const out: Chip[] = [];
    // Counted as the conversation shows them: the same request from several agents is one card.
    const waiting = groupSame(store.pendingApprovals.filter((a) => store.approvalThread(a) === thread)).map((g) => g.approval);
    if (plan?.status === 'awaiting_approval') {
      out.push({ label: 'Change the plan', run: () => ui.seedComposer(thread, 'Change the plan: ') });
      out.push({ label: 'Why these agents?', run: () => sendChat(thread, 'Why did you route the tasks to these agents?') });
    } else if (plan && (plan.status === 'running' || plan.status === 'integrating')) {
      if (waiting.length) out.push({ label: reviewLabel(waiting), icon: 'flag', run: () => (ui.revealApproval = waiting[0]?.id ?? null) });
      out.push({ label: 'Show the plan', icon: 'plan', run: () => (ui.revealPlan = plan.id) });
      out.push({ label: 'What’s left?', run: () => sendChat(thread, 'What’s left, and is anything at risk?') });
    } else if (plan?.status === 'done' && !store.planMerges[plan.id] && !plan.merged_sha && !plan.kept && !ui.keptPlans.includes(plan.id)) {
      // The plan's card can be far above the end (a review, fix or resolve task came after it):
      // its merge is here too, while nothing stands in its way.
      if (!mergeBlock(mergeUi.states[mergeKey({ kind: 'plan', id: plan.id })])) {
        const target = store.workspace?.current_branch ?? store.workspace?.default_branch ?? 'main';
        const id = plan.id;
        out.push({ label: `Merge into ${target}`, icon: 'merge', primary: true, run: () => void mergePlan(id) });
      }
      if (store.artifacts.some((a) => a.task_id && store.taskMap.get(a.task_id)?.plan_id === plan.id)) out.push({ label: 'See what they made', icon: 'image', run: () => ((ui.changesLens = 'artifacts'), ui.go('changes')) });
      out.push({ label: 'Anything risky in the diff?', run: () => sendChat(thread, 'Before I merge: is anything in the diff risky?') });
    } else if (plan?.status === 'done') {
      out.push({ label: 'Plan a follow-up', run: () => sendChat(thread, 'What would you do next?') });
      out.push({ label: 'View changes', icon: 'diff', run: () => ui.openDiff({ kind: 'plan', id: plan.id }) });
    }
    return out;
  });
  /** The summary of your time away is out of view (you came back at the thread's end): how long you were away. */
  const awayMinutes = $derived(awayShown.hidden);
</script>

<div class="row" aria-live="off">
  {#if awayMinutes != null}
    <button class="nc away" onclick={() => awayShown.reveal(ui.reducedMotion)} data-tip="Show what came while you were away">
      <Icon name="chevron-up" size={12} />While you were away ({awayFor(awayMinutes)})
    </button>
  {/if}
  {#if escAsked && agent}
    <p class="live esc" role="status">Press Esc again to stop {agent.name}</p>
  {:else if working && agent && n}
    <p class="live">
      <span class="dot"></span><b>{agent.name}</b>
      {#if line}
        <button class="cnt num" bind:this={countBtn} aria-haspopup="menu" aria-expanded={listOpen} aria-label="{agent.name}’s list: {line.done} of {line.total} done. Show the steps" data-tip="Its own list: {line.done} of {line.total} done" onclick={() => (listOpen = !listOpen)}>{line.done}/{line.total}</button>
      {/if}
      <span class="truncate"><Say parts={n.parts} /></span>
      {#if queued}<span class="q num">· {queued} queued</span>{/if}
    </p>
  {:else if chips.length}
    <div class="chips">
      {#each chips as c (c.label)}
        <button class="nc" class:primary={c.primary} onclick={c.run}>
          {#if c.icon}<Icon name={c.icon} size={12} />{/if}{c.label}{#if c.kbd}<span class="k">{c.kbd}</span>{/if}
        </button>
      {/each}
    </div>
  {:else if n?.tone === 'wait' && agent && line}
    <!-- It waits for you in the middle of its list: the line stays, greyed, on the step it waits in. -->
    <p class="live waits"><b>{agent.name}</b><span class="truncate num">{todoWords(line)}</span></p>
  {:else}
    <span class="grow"></span>
  {/if}
  {#if running && agent}
    <button class="nc stop" disabled={halting} onclick={() => stopThread(thread)} data-tip={isRequest ? `Stop ${agent.name}’s turn. Agents on tasks keep working.` : `Stop ${agent.name}’s turn`} data-tip-kbd="Esc Esc">
      <Icon name="stop" size={12} />{halting ? 'Stopping…' : 'Stop'}
    </button>
  {/if}
  {#if isRequest && !empty}
    <button class="nc new" onclick={() => ui.newRequest()} data-tip="Start another request in parallel — its own Lead, its own plan" data-tip-kbd="⌘N">
      <Icon name="plus" size={12} stroke={1.8} />New request
    </button>
  {/if}
</div>
{#if listOpen && countBtn}<Menu anchor={countBtn} items={steps} onclose={() => (listOpen = false)} width={340} />{/if}

<style>
  .row { height: 34px; display: flex; align-items: center; gap: 10px; padding: 0 4px; }
  .live { flex: 1; display: flex; align-items: center; gap: 8px; min-width: 0; font-size: var(--t-sm); color: var(--ink-2); animation: fade-in var(--d-3) var(--ease-out); }
  .live b { font-weight: var(--w-semi); color: var(--ink-1); white-space: nowrap; }
  .live.esc { font-weight: var(--w-med); color: var(--ink-1); white-space: nowrap; }
  .live.waits, .live.waits b { color: var(--ink-3); }
  .cnt { flex-shrink: 0; height: 22px; padding: 0 8px; border-radius: var(--r-full); font-size: var(--t-xs); font-weight: var(--w-semi); color: var(--ink-2); background: var(--surface-2); box-shadow: inset 0 0 0 1px var(--line-2); }
  .cnt:hover, .cnt[aria-expanded='true'] { background: var(--surface-3); color: var(--ink-1); }
  .q { white-space: nowrap; color: var(--ink-3); }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--aurora); box-shadow: 0 0 0 3px var(--live-soft); flex-shrink: 0; }
  .chips { flex: 1; min-width: 0; display: flex; gap: 6px; overflow-x: auto; scrollbar-width: none; animation: fade-in var(--d-3) var(--ease-out); }
  .grow { flex: 1; }
  .nc { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 12px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); background: var(--surface-2); white-space: nowrap; transition: background var(--d-2), color var(--d-2); }
  .nc:hover { background: var(--surface-3); color: var(--ink-1); }
  .nc.primary { background: var(--aurora); color: var(--on-aurora); font-weight: var(--w-semi); }
  .nc.primary:hover { filter: brightness(1.06); }
  .nc.stop { flex-shrink: 0; color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line-3); }
  .nc.stop:disabled { opacity: 1; color: var(--ink-3); }
  .nc.away { flex-shrink: 0; color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line-2); }
  .nc.new { flex-shrink: 0; background: transparent; box-shadow: inset 0 0 0 1px var(--line-2); }
  .nc.new:hover { background: var(--surface-2); }
  .k { opacity: 0.65; font-weight: var(--w-reg); }
</style>
