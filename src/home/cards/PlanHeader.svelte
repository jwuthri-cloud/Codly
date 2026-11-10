<!--
  The request's running plan, pinned at the top of its conversation: one segment per task in
  its agent's hue (live ones carry the aurora), how far along it is (in amber when an agent
  waits for you), and who is on it.
  Click → the plan's checklist in the conversation; the arrow opens the Plan room.
-->
<script lang="ts">
  import { requests } from '../../lib/state/requests.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { idVar, TASK_STATUS_LABEL } from '../../lib/util/labels';
  import Icon from '../../components/Icon.svelte';
  import Portrait from '../../components/Portrait.svelte';

  interface Props {
    planId: string;
  }
  let { planId }: Props = $props();
  const plan = $derived(store.planMap.get(planId) ?? null);
  const tasks = $derived(store.tasks.filter((t) => t.plan_id === planId).sort((a, b) => a.key.localeCompare(b.key, undefined, { numeric: true })));
  const done = $derived(tasks.filter((t) => t.status === 'done').length);
  const live = $derived(tasks.filter((t) => ['running', 'verifying', 'reviewing', 'merging'].includes(t.status)).length);
  const crew = $derived([...new Set(tasks.map((t) => t.assignee).filter((x): x is string => !!x))].map((id) => store.agentMap.get(id)).filter((a) => !!a));
  const finished = $derived(plan?.status === 'done');
  const target = $derived(store.workspace?.current_branch ?? 'main');
  /** Approvals and questions its agents wait on: a task held by one still counts as running. */
  const waiting = $derived(requests.get(plan?.thread || 'lead')?.waiting ?? 0);
  const status = $derived(
    waiting ? `${waiting === 1 ? 'Waiting for you' : `${waiting} waiting for you`} · ${done} of ${tasks.length} done`
    : finished ? (done === tasks.length ? `All ${tasks.length} merged · ready for ${target}` : `${done} of ${tasks.length} done · ready for ${target}`)
    : plan?.status === 'integrating' ? `${done} of ${tasks.length} done · integrating`
    : `${done} of ${tasks.length} done${live ? ` · ${live} working` : ''}`,
  );
</script>

{#if plan}
  <div class="ph" class:finished class:wait={waiting > 0}>
    <button class="main" onclick={() => (ui.revealPlan = planId)} aria-label="Show the plan: {plan.summary}, {status}">
      <span class="title serif truncate">{plan.summary}</span>
      <span class="bar" aria-hidden="true">
        {#each tasks as t (t.id)}
          {@const a = t.assignee ? store.agentMap.get(t.assignee) : null}
          <i class="s-{t.status}" style:--hue={a ? idVar(a.color) : 'var(--line-3)'} data-tip="{t.key} · {t.title} — {TASK_STATUS_LABEL[t.status]}"></i>
        {/each}
      </span>
      <span class="status num">{status}</span>
    </button>
    <span class="crew">
      {#each crew as a (a!.id)}<button class="face" onclick={() => ui.selectAgent(a!.id)} aria-label={a!.name} data-tip={a!.name}><Portrait agent={a} size={22} /></button>{/each}
    </span>
    <button class="icon-btn sm room" onclick={() => ui.openPlan(planId, plan.thread)} aria-label="Open in the Plan room" data-tip="Plan room" data-tip-kbd="G P"><Icon name="arrow-right" size={13} /></button>
  </div>
{/if}

<style>
  .ph { display: flex; align-items: center; gap: 10px; height: 52px; padding: 0 12px 0 20px; box-shadow: inset 0 -1px 0 var(--line); flex-shrink: 0; }
  .main { flex: 1; min-width: 0; display: flex; align-items: center; gap: 14px; height: 100%; text-align: left; }
  .title { font-size: 19px; color: var(--ink-1); flex: 0 1 auto; min-width: 60px; }
  .bar { display: flex; gap: 3px; width: 120px; flex-shrink: 0; }
  .bar i { flex: 1; height: 5px; border-radius: 3px; background: var(--line-2); transition: background var(--d-4) var(--ease-out); }
  .bar i.s-done { background: var(--hue); }
  .bar i.s-running, .bar i.s-verifying, .bar i.s-reviewing, .bar i.s-merging { background: linear-gradient(90deg, var(--hue), var(--aurora-a), var(--aurora-b), var(--hue)); background-size: 200% 100%; animation: aurora-drift 2.4s linear infinite; }
  .bar i.s-blocked, .bar i.s-failed { background: var(--bad); }
  .status { font-size: var(--t-sm); color: var(--ink-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .finished .status { color: var(--ink-2); }
  .wait .status { color: var(--amber-ink); }
  .crew { display: flex; }
  .room { margin-left: 2px; }
  .face { display: grid; border-radius: 50%; margin-left: -4px; box-shadow: 0 0 0 2px var(--glass-solid); }
  .main:hover .title { text-decoration: underline; text-decoration-thickness: 1px; text-underline-offset: 3px; }
</style>
