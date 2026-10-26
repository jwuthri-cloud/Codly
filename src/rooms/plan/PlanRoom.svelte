<!--
  Plan room. One plan at a time (pick the request it answers on the left of the header).
  List (default): the tasks as a checklist grouped by status · Flow: the dependency graph ·
  Timeline: what each agent did, when.
-->
<script lang="ts">
  import { cancelPlan, runPlan } from '../../lib/state/actions';
  import { requests } from '../../lib/state/requests.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { ui, type PlanLens } from '../../lib/state/ui.svelte';
  import { clock } from '../../lib/util/format';
  import { GLOSSARY, PLAN_STATUS_LABEL } from '../../lib/util/labels';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';
  import RoomHead from '../RoomHead.svelte';
  import FlowLens from './FlowLens.svelte';
  import ListLens from './ListLens.svelte';
  import TimelineLens from './TimelineLens.svelte';

  const plans = $derived([...store.plans].sort((a, b) => b.created_at - a.created_at));
  // The plan you picked if it answers the request you are on; else that request's plan.
  const chosen = $derived(ui.planId ? (store.planMap.get(ui.planId) ?? null) : null);
  const plan = $derived((chosen && (chosen.thread || 'lead') === ui.request ? chosen : null) ?? store.planOf(ui.request) ?? chosen ?? store.activePlan ?? plans[0] ?? null);
  const tasks = $derived(plan ? store.tasks.filter((t) => t.plan_id === plan.id).sort((a, b) => a.key.localeCompare(b.key, undefined, { numeric: true })) : []);
  const done = $derived(tasks.filter((t) => t.status === 'done').length);
  /** One chip per request that has a plan (its current plan). */
  const picks = $derived(requests.list.filter((r) => r.plan));
  const LENSES: { id: PlanLens; label: string; icon: string; hint: string }[] = [
    { id: 'list', label: 'List', icon: 'list', hint: 'The tasks, grouped by status' },
    { id: 'flow', label: 'Flow', icon: 'plan', hint: 'Which task waits on which' },
    { id: 'timeline', label: 'Timeline', icon: 'timeline', hint: 'What each agent did, when' },
  ];
  function pick(planId: string, thread: string) {
    ui.planId = planId;
    ui.request = thread;
  }
</script>

<RoomHead eyebrow={plan ? `Plan · ${PLAN_STATUS_LABEL[plan.status]}` : 'Plan'} title={plan?.summary ?? 'No plan yet'}>
  {#snippet sub()}
    {#if plan}
      <span class="num">{done} of {tasks.length} done</span>
      {#if plan.integration_branch}<span>·</span><code data-tip={GLOSSARY.integration}>{plan.integration_branch}</code>{/if}
      <span>·</span><span>proposed {clock(plan.created_at)}</span>
    {:else}
      <span>Ask the Lead on Home; its plan appears here as a checklist.</span>
    {/if}
  {/snippet}
  {#snippet right()}
    {#if plan?.status === 'awaiting_approval'}
      <button class="btn primary" onclick={() => runPlan(plan.id)}><Icon name="play" size={13} />Run plan<span class="k">⌘↵</span></button>
      <button class="btn ghost" onclick={() => cancelPlan(plan.id)}>Cancel</button>
    {:else if plan?.status === 'running'}
      <button class="btn ghost sm" onclick={() => cancelPlan(plan.id)}>Cancel plan</button>
    {/if}
    <div class="lenses" role="radiogroup" aria-label="Lens">
      {#each LENSES as l (l.id)}
        <button role="radio" aria-checked={ui.planLens === l.id} onclick={() => (ui.planLens = l.id)} data-tip={l.hint}><Icon name={l.icon} size={13} />{l.label}</button>
      {/each}
    </div>
  {/snippet}
</RoomHead>

{#if picks.length > 1}
  <div class="picks" role="tablist" aria-label="Request">
    {#each picks as r (r.thread)}
      <button role="tab" aria-selected={plan?.id === r.plan!.id} class="pk p-{r.phase}" class:amber={r.needsYou} onclick={() => pick(r.plan!.id, r.thread)} data-tip={r.title}>
        <span class="dot"></span><span class="truncate">{r.plan!.summary}</span><span class="st">{r.status}</span>
      </button>
    {/each}
  </div>
{/if}

{#if ui.planLens === 'list'}
  <ListLens {plan} {tasks} />
{:else if ui.planLens === 'flow'}
  {#if tasks.length}<FlowLens {tasks} />{:else}<EmptyState title="No plan yet." body="Ask the Lead for something on Home. Its plan lands here as tasks flowing left to right." />{/if}
{:else}
  <TimelineLens />
{/if}

<style>
  code { font-size: 11.5px; color: var(--ink-2); }
  .picks { display: flex; gap: 6px; padding: 0 28px 14px; overflow-x: auto; scrollbar-width: none; flex-shrink: 0; }
  .pk { display: inline-flex; align-items: center; gap: 8px; height: 32px; padding: 0 12px 0 10px; border-radius: var(--r-full); max-width: 360px; font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); background: var(--surface-1); flex-shrink: 0; }
  .pk:hover { background: var(--surface-2); color: var(--ink-1); }
  .pk[aria-selected='true'] { background: var(--surface-3); color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line-2); }
  .st { color: var(--ink-3); font-weight: var(--w-reg); white-space: nowrap; }
  .pk.amber .st { color: var(--amber-ink); }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; box-shadow: inset 0 0 0 1.5px var(--ink-4); }
  .p-planning .dot, .p-running .dot { background: var(--live); box-shadow: 0 0 0 3px var(--live-soft); }
  .p-done .dot { background: var(--ok); box-shadow: none; }
  .p-merged .dot { box-shadow: inset 0 0 0 1.5px var(--ok); }
  .pk.amber .dot { background: var(--amber); box-shadow: none; }
</style>
