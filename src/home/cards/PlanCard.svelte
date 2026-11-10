<!--
  The plan card: the clearest thing in the conversation. What you asked (your words), the Lead's
  plan as a checklist (each task: title, who does it, what it waits on, its state), open
  questions highlighted, and the one action that matters: Run plan ⌘↵. While the plan runs the
  same checklist stays here and updates live; once it is over it folds to one line you can open.
-->
<script lang="ts">
  import { cancelPlan, runPlan } from '../../lib/state/actions';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { clock } from '../../lib/util/format';
  import { methodProvenance, PLAN_STATUS_LABEL } from '../../lib/util/labels';
  import Icon from '../../components/Icon.svelte';
  import Why, { type WhyLine } from '../../components/Why.svelte';
  import TaskRow from './TaskRow.svelte';

  interface Props {
    planId: string;
  }
  let { planId }: Props = $props();

  const plan = $derived(store.planMap.get(planId) ?? null);
  const tasks = $derived(store.tasks.filter((t) => t.plan_id === planId).sort((a, b) => a.key.localeCompare(b.key, undefined, { numeric: true })));
  const startable = $derived(tasks.filter((t) => !t.depends_on.length).length);
  const done = $derived(tasks.filter((t) => t.status === 'done').length);
  const deciding = $derived(plan?.status === 'awaiting_approval' || plan?.status === 'planning');
  const live = $derived(plan?.status === 'running' || plan?.status === 'integrating');
  const over = $derived(!deciding && !live);
  let open = $state(false);
  let busy = $state(false);
  let el: HTMLElement | undefined = $state();

  const eyebrow = $derived.by(() => {
    if (!plan) return '';
    const n = `${tasks.length} task${tasks.length === 1 ? '' : 's'}`;
    if (plan.status === 'planning') return `Plan · drafting · ${n}`;
    if (plan.status === 'awaiting_approval') return `Plan · waiting for your go · ${n}${startable ? ` · ${startable} start right away` : ''}`;
    if (live) return `Plan · ${plan.status === 'integrating' ? 'integrating' : 'running'} · ${done} of ${tasks.length} done`;
    return `Plan · ${PLAN_STATUS_LABEL[plan.status]} · ${done} of ${tasks.length} done`;
  });

  const why = $derived.by((): WhyLine[] => {
    void store.rev;
    return store.decisions
      .filter((d) => d.detail && typeof d.detail === 'object' && !Array.isArray(d.detail) && (d.detail as Record<string, unknown>).plan_id === planId && d.type !== 'route')
      .slice(0, 6)
      .map((d) => ({ prov: methodProvenance(d.method), text: d.summary }));
  });

  async function act(fn: () => Promise<unknown>) {
    busy = true;
    await fn();
    busy = false;
  }
  function edit() {
    ui.seedComposer(plan?.thread || 'lead', 'Change the plan: ');
  }

  // The pinned progress header asks to see this card.
  $effect(() => {
    if (ui.revealPlan === planId && el) {
      ui.revealPlan = null;
      open = true;
      el.scrollIntoView({ block: 'start', behavior: ui.reducedMotion ? 'auto' : 'smooth' });
      flash = true;
      setTimeout(() => (flash = false), 1500);
    }
  });
  let flash = $state(false);
</script>

{#if plan}
  <article class="card plan" class:ready={plan.status === 'awaiting_approval'} class:live class:over class:flash data-plan-card={planId} bind:this={el} aria-label="Plan: {plan.summary}">
    {#if over && !open}
      <button class="folded" onclick={() => (open = true)} aria-expanded="false">
        <Icon name="plan" size={14} />
        <span class="f-title truncate">{plan.summary}</span>
        <span class="f-state">{PLAN_STATUS_LABEL[plan.status]} · {done} of {tasks.length} done</span>
        <Icon name="chevron-down" size={13} />
      </button>
    {:else}
      <header class="head">
        <p class="eyebrow">{eyebrow}</p>
        {#if over}<button class="icon-btn sm" onclick={() => (open = false)} aria-label="Fold the plan"><Icon name="chevron-up" size={13} /></button>{/if}
      </header>
      <h3 class="serif title">{plan.summary}</h3>
      {#if plan.request}
        <blockquote class="req"><span class="rq">You asked</span><span class="rt">{plan.request}</span></blockquote>
      {/if}

      <ol class="tasks" aria-label="Tasks">
        {#each tasks as t (t.id)}<TaskRow task={t} {plan} goal={deciding} />{/each}
      </ol>

      {#if deciding && plan.open_questions.length}
        <ul class="qs">
          {#each plan.open_questions as q, i (i)}<li><Icon name="question" size={14} /><span>{q}</span></li>{/each}
        </ul>
      {/if}
      {#if deciding && plan.assumptions.length}
        <ul class="as">
          {#each plan.assumptions as a, i (i)}<li>Assumes {a.charAt(0).toLowerCase() + a.slice(1)}</li>{/each}
        </ul>
      {/if}

      {#if deciding}
        <footer>
          <button class="btn primary" disabled={busy || plan.status !== 'awaiting_approval'} onclick={() => act(() => runPlan(planId))}>
            <Icon name="play" size={13} />Run plan<span class="k">⌘↵</span>
          </button>
          <button class="btn ghost" disabled={busy} onclick={edit}>Change the plan</button>
          <button class="btn ghost" disabled={busy} onclick={() => act(() => cancelPlan(planId))}>Cancel</button>
        </footer>
        <p class="fine">Each task runs in its own worktree and lands on an integration branch; <code>{store.workspace?.current_branch ?? 'main'}</code> is untouched until you merge.</p>
        <Why lines={why} />
      {:else}
        <footer class="quiet">
          <span class="since num">{live ? `Started ${clock(plan.updated_at)}` : `Updated ${clock(plan.updated_at)}`}</span>
          <button class="btn ghost sm" onclick={() => ui.openPlan(planId, plan.thread)}>Plan room<Icon name="arrow-right" size={12} /></button>
        </footer>
      {/if}
    {/if}
  </article>
{/if}

<style>
  .card { position: relative; display: flex; flex-direction: column; gap: 12px; padding: 18px 18px 16px; border-radius: var(--r-lg); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); scroll-margin-top: 12px; }
  .plan.ready { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--aurora-m) 30%, transparent), 0 -8px 40px -26px var(--aurora-m) inset; }
  .plan.ready::before { content: ''; position: absolute; left: 18px; right: 18px; top: 0; height: 1px; background: var(--aurora); opacity: 0.9; }
  .plan.over { padding: 0; background: none; box-shadow: none; }
  .plan.over:has(.head) { padding: 18px 18px 16px; background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin: 0 4px -6px; }
  .title { font-size: var(--d-md); line-height: 1.12; color: var(--ink-1); margin: 0 4px; }
  .req { display: flex; flex-direction: column; gap: 2px; margin: -2px 4px 0; padding: 8px 12px; border-radius: var(--r-md); background: var(--surface-2); }
  .rq { font-size: var(--t-xs); font-weight: var(--w-semi); letter-spacing: 0.04em; text-transform: uppercase; color: var(--ink-3); }
  .rt { font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .tasks { display: flex; flex-direction: column; gap: 2px; margin: 0 -6px; }
  .qs { display: flex; flex-direction: column; gap: 6px; }
  .qs li { display: flex; gap: 8px; align-items: flex-start; padding: 8px 12px; border-radius: var(--r-md); background: var(--amber-soft); color: var(--ink-1); font-size: var(--t-md); line-height: var(--lh-md); }
  .qs li :global(svg) { color: var(--amber); margin-top: 3px; flex-shrink: 0; }
  .as { display: flex; flex-direction: column; gap: 3px; margin: 0 4px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
  footer { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; margin: 2px 4px 0; }
  footer.quiet { justify-content: space-between; margin-top: -2px; }
  .since { font-size: var(--t-sm); color: var(--ink-3); }
  .fine { font-size: var(--t-sm); color: var(--ink-3); margin: -4px 4px 0; }
  .fine code { font-size: 11px; color: var(--ink-2); }
  .folded { display: flex; align-items: center; gap: 10px; width: 100%; height: 42px; padding: 0 14px; border-radius: var(--r-md); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); color: var(--ink-3); text-align: left; }
  .folded:hover { background: var(--surface-2); color: var(--ink-1); }
  .f-title { color: var(--ink-1); font-weight: var(--w-med); }
  .f-state { flex: 1; font-size: var(--t-sm); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .plan.flash { animation: plan-flash 1.4s var(--ease-out); }
  @keyframes plan-flash { 0%, 30% { box-shadow: inset 0 0 0 1.5px var(--aurora-m); } 100% { box-shadow: inset 0 0 0 1px var(--line); } }
</style>
