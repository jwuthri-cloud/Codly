<!--
  One fixed-height row above the composer (so nothing jumps): while the thread's agent works it
  says what it is doing (and how many of your messages wait for it); otherwise it offers the next
  sensible actions for this request as small chips. New request (⌘N) always sits on the right.
-->
<script lang="ts">
  import { sendChat } from '../lib/state/actions';
  import { narrate } from '../lib/state/narration';
  import { store } from '../lib/state/store.svelte';
  import { isRequestThread, ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';
  import Say from '../components/Say.svelte';

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

  type Chip = { label: string; icon?: string; kbd?: string; primary?: boolean; run: () => void };
  const chips = $derived.by((): Chip[] => {
    if (!isRequest) return [];
    void store.rev;
    const plan = store.planOf(thread);
    const out: Chip[] = [];
    const waiting = store.pendingApprovals.filter((a) => store.approvalThread(a) === thread);
    if (plan?.status === 'awaiting_approval') {
      out.push({ label: 'Change the plan', run: () => ui.seedComposer(thread, 'Change the plan: ') });
      out.push({ label: 'Why these agents?', run: () => sendChat(thread, 'Why did you route the tasks to these agents?') });
    } else if (plan && (plan.status === 'running' || plan.status === 'integrating')) {
      if (waiting.length) out.push({ label: waiting.length === 1 ? 'Review the approval' : `Review ${waiting.length} approvals`, icon: 'flag', run: () => (ui.revealApproval = waiting[0]?.id ?? null) });
      out.push({ label: 'Show the plan', icon: 'plan', run: () => (ui.revealPlan = plan.id) });
      out.push({ label: 'What’s left?', run: () => sendChat(thread, 'What’s left, and is anything at risk?') });
    } else if (plan?.status === 'done' && !store.planMerges[plan.id] && !plan.merged_sha && !plan.kept && !ui.keptPlans.includes(plan.id)) {
      if (store.artifacts.some((a) => a.task_id && store.taskMap.get(a.task_id)?.plan_id === plan.id)) out.push({ label: 'See what they made', icon: 'image', run: () => ((ui.changesLens = 'artifacts'), ui.go('changes')) });
      out.push({ label: 'Anything risky in the diff?', run: () => sendChat(thread, 'Before I merge: is anything in the diff risky?') });
    } else if (plan?.status === 'done') {
      out.push({ label: 'Plan a follow-up', run: () => sendChat(thread, 'What would you do next?') });
      out.push({ label: 'View changes', icon: 'diff', run: () => ui.openDiff({ kind: 'plan', id: plan.id }) });
    }
    return out;
  });
</script>

<div class="row" aria-live="off">
  {#if working && agent && n}
    <p class="live">
      <span class="dot"></span><b>{agent.name}</b><span class="truncate"><Say parts={n.parts} /></span>
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
  {:else}
    <span class="grow"></span>
  {/if}
  {#if isRequest}
    <button class="nc new" onclick={() => ui.newRequest()} data-tip="Start another request in parallel — its own Lead, its own plan" data-tip-kbd="⌘N">
      <Icon name="plus" size={12} stroke={1.8} />New request
    </button>
  {/if}
</div>

<style>
  .row { height: 34px; display: flex; align-items: center; gap: 10px; padding: 0 4px; }
  .live { flex: 1; display: flex; align-items: center; gap: 8px; min-width: 0; font-size: var(--t-sm); color: var(--ink-2); animation: fade-in var(--d-3) var(--ease-out); }
  .live b { font-weight: var(--w-semi); color: var(--ink-1); white-space: nowrap; }
  .q { white-space: nowrap; color: var(--ink-3); }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--aurora); box-shadow: 0 0 0 3px var(--live-soft); flex-shrink: 0; }
  .chips { flex: 1; min-width: 0; display: flex; gap: 6px; overflow-x: auto; scrollbar-width: none; animation: fade-in var(--d-3) var(--ease-out); }
  .grow { flex: 1; }
  .nc { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 12px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); background: var(--surface-2); white-space: nowrap; transition: background var(--d-2), color var(--d-2); }
  .nc:hover { background: var(--surface-3); color: var(--ink-1); }
  .nc.primary { background: var(--aurora); color: var(--on-aurora); font-weight: var(--w-semi); }
  .nc.primary:hover { filter: brightness(1.06); }
  .nc.new { flex-shrink: 0; background: transparent; box-shadow: inset 0 0 0 1px var(--line-2); }
  .nc.new:hover { background: var(--surface-2); }
  .k { opacity: 0.65; font-weight: var(--w-reg); }
</style>
