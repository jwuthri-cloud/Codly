<!--
  The task sheet: its goal and acceptance with live checks, who has it and why, what it may
  write, what it waits on and unblocks, Codly's gate (commit → verify → review → merge), the
  diff and what it made.
-->
<script lang="ts">
  import type { Task } from '../lib/gen/Task';
  import type { TaskStatus } from '../lib/gen/TaskStatus';
  import { moveTask } from '../lib/state/actions';
  import { diffstats, statLine } from '../lib/state/diffstats.svelte';
  import { narrate } from '../lib/state/narration';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { clock, shortSha } from '../lib/util/format';
  import { GLOSSARY, idVar, methodProvenance, PROVIDER_LABEL, TASK_STATUS_LABEL } from '../lib/util/labels';
  import { tildify } from '../lib/util/sanitize';
  import ArtifactThumb from '../components/ArtifactThumb.svelte';
  import Icon from '../components/Icon.svelte';
  import InlineText from '../components/InlineText.svelte';
  import Portrait from '../components/Portrait.svelte';
  import Provenance from '../components/Provenance.svelte';
  import Say from '../components/Say.svelte';
  import Section from '../components/Section.svelte';
  import Why, { type WhyLine } from '../components/Why.svelte';
  import TaskChecks from './TaskChecks.svelte';

  interface Props {
    task: Task;
  }
  let { task }: Props = $props();
  const agent = $derived(task.assignee ? (store.agentMap.get(task.assignee) ?? null) : null);
  const n = $derived(agent && agent.task_id === task.id ? narrate(agent.id) : null);
  const plan = $derived(task.plan_id ? (store.planMap.get(task.plan_id) ?? null) : null);
  const deps = $derived(task.depends_on.map((id) => store.taskMap.get(id)).filter((t) => !!t));
  const unblocks = $derived(store.tasks.filter((t) => t.depends_on.includes(task.id)));
  const marks = $derived.by(() => {
    void store.rev;
    return store.taskMarks[task.id] ?? [];
  });
  const stat = $derived(diffstats.task[task.id] ?? null);
  const arts = $derived(store.artifacts.filter((a) => a.task_id === task.id));
  const why = $derived.by((): WhyLine[] => {
    void store.rev;
    return store.decisions.filter((d) => d.taskId === task.id).map((d) => ({ prov: methodProvenance(d.method), text: d.summary, meta: clock(d.ts) }));
  });
  const LEGAL: Partial<Record<TaskStatus, { to: TaskStatus; label: string }[]>> = {
    pending: [{ to: 'ready', label: 'Move to Ready' }, { to: 'cancelled', label: 'Cancel' }],
    ready: [{ to: 'pending', label: 'Back to Backlog' }, { to: 'cancelled', label: 'Cancel' }],
    blocked: [{ to: 'ready', label: 'Retry' }],
    failed: [{ to: 'ready', label: 'Retry' }],
    cancelled: [{ to: 'ready', label: 'Reopen' }],
  };
  const MARK_ICON: Record<string, string> = { committed: 'commit', verified: 'check-circle', reviewed: 'eye', merged: 'merge' };
</script>

<div class="ts" style:--hue={idVar(agent?.color ?? 0)}>
  <header>
    <p class="eyebrow">{plan ? 'Plan task' : 'Board task'} · {task.key}</p>
    <h2 class="serif">{task.title}</h2>
    <p class="status st-{task.status}">{TASK_STATUS_LABEL[task.status]}{task.status_reason ? ` — ${task.status_reason}` : ''}</p>
    <div class="acts">
      {#if marks.length || task.status === 'done'}<button class="btn sm" onclick={() => ui.openDiff({ kind: 'task', id: task.id })}><Icon name="diff" size={13} />View changes</button>{/if}
      {#each LEGAL[task.status] ?? [] as l (l.to)}<button class="btn sm ghost" onclick={() => moveTask(task.id, l.to)}>{l.label}</button>{/each}
      {#if plan}<button class="btn sm ghost" onclick={() => ui.openPlan(plan.id, plan.thread)}><Icon name="plan" size={13} />In the plan</button>{/if}
    </div>
  </header>

  <div class="sections">
    <Section id="t-agent" title="Who">
      {#if agent}
        <button class="who" onclick={() => ui.selectAgent(agent.id)}>
          <Portrait {agent} size={30} />
          <span class="w-txt"><b>{agent.name}</b><span class="w-prov">{PROVIDER_LABEL[agent.provider]}</span>{#if n}<span class="w-now truncate"><Say parts={n.parts} /></span>{/if}</span>
          <Icon name="chevron-right" size={13} />
        </button>
      {:else}
        <p class="small">{task.agent_hint ? `Not started · the planner suggests ${PROVIDER_LABEL[task.agent_hint]}` : 'Not assigned · Codly picks a free agent when it is ready'}</p>
      {/if}
      <Why lines={why} label="Why this way?" />
    </Section>

    <Section id="t-goal" title="Goal & checks">
      <p class="goal selectable"><InlineText text={task.goal} /></p>
      <TaskChecks {task} />
    </Section>

    <Section id="t-scope" title="Scope" summary={task.write_scope.length ? `${task.write_scope.length} paths` : 'unrestricted'}>
      {#if task.write_scope.length}<ul class="scope">{#each task.write_scope as g (g)}<li><code>{g}</code></li>{/each}</ul>{:else}<p class="small">May change any file.</p>{/if}
      {#if deps.length || unblocks.length}
        <p class="deps">
          {#if deps.length}<span>After {#each deps as d, i (d!.id)}{i ? ', ' : ''}<button class="link" onclick={() => ui.selectTask(d!.id)}>{d!.key}</button>{/each}</span>{/if}
          {#if unblocks.length}<span>Unblocks {#each unblocks as d, i (d.id)}{i ? ', ' : ''}<button class="link" onclick={() => ui.selectTask(d.id)}>{d.key}</button>{/each}</span>{/if}
        </p>
      {/if}
      {#if task.branch}<p class="branch" data-tip={GLOSSARY.worktree}><Icon name="branch" size={12} /><code>{task.branch}</code>{#if task.worktree}<span class="wt">{tildify(task.worktree)}</span>{/if}</p>{/if}
    </Section>

    {#if marks.length || task.summary}
      <Section id="t-gate" title="Landing" summary={stat ? statLine(stat) : ''}>
        {#if task.summary}<p class="summary selectable"><Provenance kind="model" compact /><span><InlineText text={task.summary} /></span></p>{/if}
        {#if stat}<p class="stat num"><span class="add">+{stat.added}</span> <span class="del">−{stat.removed}</span> in {stat.files} {stat.files === 1 ? 'file' : 'files'}</p>{/if}
        <ol class="marks">
          {#each marks as m (m.eventId)}
            <li class:bad={!m.ok}><Icon name={MARK_ICON[m.kind]} size={13} /><span class="m-k">{m.kind}</span><span class="m-t truncate">{m.kind === 'committed' ? `${shortSha(m.sha)} · ${m.files?.length ?? 0} files` : m.text}</span><time class="num">{clock(m.ts)}</time></li>
          {/each}
        </ol>
      </Section>
    {/if}

    {#if arts.length}
      <Section id="t-arts" title="Made" summary={String(arts.length)}>
        <div class="arts">
          {#each arts as a (a.id)}
            <button class="art" onclick={() => ui.openArtifact(a.id)}><span class="tb"><ArtifactThumb artifact={a} /></span><span class="cap truncate">{a.title}</span></button>
          {/each}
        </div>
      </Section>
    {/if}
  </div>
</div>

<style>
  .ts { display: flex; flex-direction: column; height: 100%; min-height: 0; overflow-y: auto; }
  header { padding: 26px 22px 14px; display: flex; flex-direction: column; gap: 6px; box-shadow: inset 3px 0 0 var(--hue); }
  h2 { font-size: var(--d-md); line-height: 1.1; padding-right: 30px; }
  .status { font-size: var(--t-sm); color: var(--ink-3); }
  .st-running, .st-verifying, .st-merging { color: var(--live); }
  .st-blocked, .st-failed { color: var(--bad); }
  .st-done { color: var(--ok); }
  .acts { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px; }
  .who { display: flex; align-items: center; gap: 10px; width: 100%; padding: 8px 10px; border-radius: var(--r-md); background: var(--surface-1); text-align: left; margin-bottom: 8px; }
  .who:hover { background: var(--surface-2); }
  .w-txt { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .w-txt b { font-weight: var(--w-semi); }
  .w-prov { font-size: var(--t-xs); color: var(--ink-3); }
  .w-now { font-size: var(--t-sm); color: var(--ink-2); }
  .small { font-size: var(--t-sm); color: var(--ink-3); margin-bottom: 6px; }
  .goal { font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-1); }
  .scope { display: flex; flex-wrap: wrap; gap: 6px; }
  .scope code { display: inline-block; font-size: 11px; padding: 3px 8px; border-radius: var(--r-full); background: var(--surface-2); color: var(--ink-2); }
  .deps { display: flex; gap: 14px; margin-top: 10px; font-size: var(--t-sm); color: var(--ink-3); }
  .link { font-family: var(--font-mono); font-size: 11.5px; color: var(--ink-1); text-decoration: underline; text-decoration-color: var(--line-3); text-underline-offset: 2px; }
  .branch { display: flex; align-items: center; gap: 6px; margin-top: 10px; font-size: var(--t-sm); color: var(--ink-3); min-width: 0; }
  .branch code { font-size: 11px; color: var(--ink-2); }
  .wt { font-family: var(--font-mono); font-size: 10.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .summary { display: flex; gap: 8px; align-items: flex-start; font-size: var(--t-md); line-height: var(--lh-md); color: var(--ink-2); margin-bottom: 8px; }
  .stat { font-size: var(--t-sm); color: var(--ink-2); margin-bottom: 8px; }
  .add { color: var(--ok); }
  .del { color: var(--bad); }
  .marks { display: flex; flex-direction: column; gap: 4px; }
  .marks li { display: flex; align-items: center; gap: 8px; height: 24px; font-size: var(--t-sm); color: var(--ink-2); }
  .marks li :global(svg) { color: var(--ok); }
  .marks li.bad :global(svg) { color: var(--bad); }
  .m-k { text-transform: capitalize; width: 74px; color: var(--ink-1); }
  .m-t { flex: 1; font-family: var(--font-mono); font-size: 11px; color: var(--ink-3); }
  .marks time { font-size: var(--t-xs); color: var(--ink-3); }
  .arts { display: grid; grid-template-columns: repeat(2, 1fr); gap: 10px; }
  .art { display: flex; flex-direction: column; gap: 6px; text-align: left; }
  .tb { display: block; height: 96px; border-radius: var(--r-md); overflow: hidden; box-shadow: inset 0 0 0 1px var(--line); }
  .cap { font-size: var(--t-xs); color: var(--ink-3); }
</style>
