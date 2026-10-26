<!--
  One task as a checklist line: state mark, key, title, what it waits on or what it is doing,
  and who does it (the assigned agent, or the provider the Lead suggested). Used by the plan
  card in the conversation and by the Plan room's list. Click → the task sheet.
-->
<script lang="ts">
  import type { Plan } from '../../lib/gen/Plan';
  import type { Task } from '../../lib/gen/Task';
  import type { Tone } from '../../lib/narrate';
  import { moveTask } from '../../lib/state/actions';
  import { narrate } from '../../lib/state/narration';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { idVar, PROVIDER_LABEL, TASK_STATUS_LABEL } from '../../lib/util/labels';
  import Icon from '../../components/Icon.svelte';
  import InlineText from '../../components/InlineText.svelte';
  import Portrait from '../../components/Portrait.svelte';
  import Say from '../../components/Say.svelte';
  import StateMark from '../../components/StateMark.svelte';

  interface Props {
    task: Task;
    plan: Plan | null;
    /** Plan room: goal, acceptance criteria, scope and the actions you own. */
    detail?: boolean;
    /** Show the goal under the title (the plan card does while you decide). */
    goal?: boolean;
  }
  let { task, plan, detail = false, goal = false }: Props = $props();

  const agent = $derived(task.assignee ? (store.agentMap.get(task.assignee) ?? null) : null);
  const n = $derived(agent && agent.task_id === task.id ? narrate(agent.id) : null);
  const deps = $derived(task.depends_on.map((id) => store.taskMap.get(id)).filter((t): t is Task => !!t));
  const unmet = $derived(deps.filter((d) => d.status !== 'done'));
  const proposing = $derived(!plan || plan.status === 'planning' || plan.status === 'awaiting_approval');
  const keys = (ts: Task[]) => ts.map((t) => t.key).join(', ');

  const tone = $derived.by((): Tone => {
    if (n?.tone === 'wait') return 'wait';
    switch (task.status) {
      case 'running': case 'verifying': case 'reviewing': case 'merging': case 'ready': return 'live';
      case 'done': return 'done';
      case 'blocked': case 'failed': case 'changes_requested': return 'blocked';
      case 'cancelled': return 'off';
      default: return 'idle';
    }
  });

  /** What it waits on, or what is happening to it, in one line. */
  const line = $derived.by((): string => {
    switch (task.status) {
      case 'pending':
        if (proposing || !plan) return deps.length ? `After ${keys(deps)}` : plan ? 'Starts right away' : 'In the backlog';
        return unmet.length ? `Waiting on ${keys(unmet)}` : 'Up next';
      case 'ready': return unmet.length ? `Waiting on ${keys(unmet)}` : 'Starting…';
      case 'running': return 'In progress';
      case 'verifying': return task.verify[0] ? `Checking · ${task.verify[0]}` : 'Checking';
      case 'reviewing': return 'In review';
      case 'merging': return 'Merging';
      case 'done': return task.summary ?? 'Merged';
      case 'cancelled': return 'Cancelled';
      default: return task.status_reason ?? TASK_STATUS_LABEL[task.status];
    }
  });
  const lit = $derived(ui.hover.taskId === task.id || (!!agent && ui.hover.agentId === agent.id));
  const retry = $derived(detail && ['blocked', 'failed', 'cancelled'].includes(task.status));
  const queue = $derived(detail && !task.plan_id && task.status === 'pending');
</script>

<li
  class="row t-{tone}"
  class:lit
  class:sel={ui.selectedTaskId === task.id}
  class:detail
  style:--hue={agent ? idVar(agent.color) : 'var(--line-3)'}
  data-task={task.id}
  onpointerenter={() => ui.setHover({ taskId: task.id })}
  onpointerleave={() => ui.setHover(null)}
>
  <button class="hit" onclick={() => ui.selectTask(task.id)} aria-label="{task.key} · {task.title} — {TASK_STATUS_LABEL[task.status]}"></button>
  <span class="mark"><StateMark {tone} size={11} /></span>
  <span class="key">{task.key}</span>
  <span class="main">
    <span class="title">{task.title}</span>
    {#if goal || detail}<span class="goal" class:clamp={!detail}><InlineText text={task.goal} /></span>{/if}
    <span class="line">
      {#if task.status === 'pending' && deps.length}<Icon name="branch" size={11} />{/if}
      {#if n && (task.status === 'running' || n.tone === 'wait')}<Say parts={n.parts} />{:else}<span class="truncate">{line}</span>{/if}
    </span>
    {#if detail && (task.acceptance.length || task.write_scope.length)}
      <span class="more">
        {#if task.acceptance.length}
          <span class="crit">{#each task.acceptance.slice(0, 4) as c, i (i)}<span><Icon name="check" size={11} /><InlineText text={c} /></span>{/each}</span>
        {/if}
        {#if task.write_scope.length}<code class="scope truncate">writes {task.write_scope.join(' · ')}</code>{/if}
      </span>
    {/if}
  </span>
  <span class="who">
    {#if agent}
      <Portrait {agent} size={20} /><span class="wn">{agent.name}</span><span class="wp">{PROVIDER_LABEL[agent.provider]}</span>
    {:else if task.agent_hint}
      <span class="wp hint">{PROVIDER_LABEL[task.agent_hint]}</span>
    {:else}
      <span class="wp hint">Any agent</span>
    {/if}
    {#if retry}<button class="btn sm act" onclick={() => moveTask(task.id, 'ready')}>Retry</button>{/if}
    {#if queue}<button class="btn sm act" onclick={() => moveTask(task.id, 'ready')}>Queue it</button>{/if}
  </span>
</li>

<style>
  .row { position: relative; display: grid; grid-template-columns: 14px 22px minmax(0, 1fr) auto; align-items: start; column-gap: 10px; padding: 11px 14px 11px 12px; border-radius: var(--r-md); transition: background-color var(--d-2) var(--ease-out); }
  .row:hover, .row.lit { background: var(--surface-2); }
  .row.sel { background: var(--surface-3); }
  .hit { position: absolute; inset: 0; border-radius: inherit; z-index: 0; }
  .mark, .key, .main, .who { position: relative; z-index: 1; pointer-events: none; }
  .mark { padding-top: 4px; }
  .key { font-family: var(--font-mono); font-size: 11.5px; line-height: 20px; color: var(--ink-3); }
  .main { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .title { font-size: 15px; line-height: 20px; font-weight: var(--w-semi); color: var(--ink-1); }
  .t-done .title { color: var(--ink-2); }
  .t-off .title { color: var(--ink-3); text-decoration: line-through; text-decoration-thickness: 1px; }
  .goal { font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .goal.clamp { display: -webkit-box; -webkit-line-clamp: 1; line-clamp: 1; -webkit-box-orient: vertical; overflow: hidden; }
  .line { display: flex; align-items: center; gap: 5px; min-width: 0; font-size: var(--t-sm); line-height: 18px; color: var(--ink-3); }
  .line :global(svg) { flex-shrink: 0; transform: rotate(90deg); color: var(--ink-4); }
  .t-live .line { color: var(--live); }
  .t-wait .line { color: var(--amber-ink); }
  .t-blocked .line { color: var(--bad); }
  .t-done .line { color: var(--ink-3); }
  .who { display: flex; align-items: center; gap: 6px; padding-top: 1px; white-space: nowrap; }
  .act { pointer-events: auto; }
  .wn { font-size: var(--t-sm); font-weight: var(--w-semi); color: var(--ink-1); }
  .wp { font-size: var(--t-sm); color: var(--ink-3); }
  .wp.hint { padding: 2px 9px; border-radius: var(--r-full); box-shadow: inset 0 0 0 1px var(--line-2); color: var(--ink-2); }
  .act { margin-left: 6px; }
  .more { display: flex; flex-direction: column; gap: 6px; margin-top: 6px; }
  .crit { display: flex; flex-direction: column; gap: 3px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .crit span { display: flex; gap: 6px; align-items: flex-start; }
  .crit :global(svg) { flex-shrink: 0; margin-top: 3px; color: var(--ink-4); }
  .t-done .crit :global(svg) { color: var(--ok); }
  .scope { font-size: 11px; color: var(--ink-3); }
  .detail { padding: 14px 16px 14px 14px; }
</style>
