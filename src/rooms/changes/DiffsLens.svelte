<!--
  Diffs, one plan at a time like the Plan room: a chip per plan (plus Other tasks and Merged)
  picks it. Left: the whole plan (its integration branch) and each task's worktree with their
  diffstat, then the files with +/−; right: unified view with line numbers. ] / [ jump between
  files. Plans already in your branch (merged from Codly or by hand) or kept on their own branch
  sit under Merged, with their tasks.
-->
<script lang="ts">
  import { api } from '../../lib/api';
  import { diffstats } from '../../lib/state/diffstats.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { diffStats, parseDiff, type DiffFile } from '../../lib/util/diff';
  import { idVar, TASK_STATUS_LABEL } from '../../lib/util/labels';
  import EmptyState from '../../components/EmptyState.svelte';
  import DiffView from './DiffView.svelte';

  const allPlans = $derived([...store.plans].filter((p) => p.status !== 'cancelled' && store.tasks.some((t) => t.plan_id === p.id && (t.status === 'done' || t.branch))).sort((a, b) => b.created_at - a.created_at));
  const allTasks = $derived([...store.tasks].filter((t) => t.branch || t.status === 'done').sort((a, b) => (a.plan_id ?? '~').localeCompare(b.plan_id ?? '~') || a.key.localeCompare(b.key, undefined, { numeric: true })));
  const settled = $derived(new Set(store.plans.filter((p) => p.merged_sha || p.kept).map((p) => p.id)));
  const plans = $derived(allPlans.filter((p) => !settled.has(p.id)));
  const tasks = $derived(allTasks.filter((t) => !t.plan_id || !settled.has(t.plan_id)));
  // Tasks with no plan shown above them: no plan, or a cancelled one.
  const loose = $derived(tasks.filter((t) => !plans.some((p) => p.id === t.plan_id)));
  const merged = $derived(allPlans.filter((p) => settled.has(p.id)));
  // The request you are on picks its plan first, as in the Plan room.
  const first = $derived(plans.find((p) => p.id === store.planOf(ui.request)?.id) ?? plans[0]);
  const target = $derived(
    ui.diffTarget ?? (first ? { kind: 'plan' as const, id: first.id } : tasks[0] ? { kind: 'task' as const, id: tasks[0].id } : merged[0] ? { kind: 'plan' as const, id: merged[0].id } : null),
  );
  // The chip you are in: a plan's id, 'loose' (tasks with no plan shown) or 'merged'.
  const group = $derived.by(() => {
    if (!target) return null;
    const pid = target.kind === 'plan' ? target.id : store.taskMap.get(target.id)?.plan_id;
    if (pid && settled.has(pid)) return 'merged';
    return pid && plans.some((p) => p.id === pid) ? pid : 'loose';
  });
  const groupPlan = $derived(plans.find((p) => p.id === group) ?? null);
  const chips = $derived([
    ...plans.map((p) => ({ id: p.id, label: p.summary, stat: diffstats.plan[p.id] ?? null, n: null as number | null })),
    ...(loose.length ? [{ id: 'loose', label: plans.length ? 'Other tasks' : 'Tasks', stat: null, n: loose.length }] : []),
    ...(merged.length ? [{ id: 'merged', label: 'Merged', stat: null, n: merged.length }] : []),
  ]);
  function choose(id: string) {
    if (id === 'loose') ui.diffTarget = { kind: 'task', id: loose[0].id };
    else ui.diffTarget = { kind: 'plan', id: id === 'merged' ? merged[0].id : id };
  }

  let files = $state<DiffFile[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let collapsed = $state<Record<string, boolean>>({});
  let active = $state<string | null>(null);

  const version = $derived.by(() => {
    if (!target) return '';
    void store.rev;
    if (target.kind === 'plan') return store.tasks.filter((t) => t.plan_id === target.id).map((t) => `${t.id}:${t.status}`).join(',');
    const t = store.taskMap.get(target.id);
    const l = t?.assignee ? store.live[t.assignee] : null;
    return `${t?.status}:${l ? Object.values(l.files).filter((f) => f.modified).length : 0}`;
  });
  let seq = 0;
  $effect(() => {
    const t = target;
    void version;
    if (!t) return;
    const my = ++seq;
    loading = true;
    (t.kind === 'plan' ? api.planDiff(t.id) : api.taskDiff(t.id))
      .then((text) => {
        if (my !== seq) return;
        files = parseDiff(text);
        error = null;
        if (!files.find((f) => f.path === active)) active = files[0]?.path ?? null;
      })
      .catch((e) => my === seq && (error = e instanceof Error ? e.message : String(e)))
      .finally(() => my === seq && (loading = false));
  });
  const totals = $derived(diffStats(files));

  /** Which task (and agent) wrote a file: from Codly's commits. */
  function ownerOf(path: string) {
    void store.rev;
    const scope = target?.kind === 'plan' ? store.tasks.filter((t) => t.plan_id === target.id) : target ? [store.taskMap.get(target.id)].filter((t) => !!t) : [];
    for (const t of scope) {
      const hit = (store.taskMarks[t!.id] ?? []).some((m) => m.files?.includes(path)) || (t!.assignee && store.live[t!.assignee]?.files[path]?.modified);
      if (hit) return { task: t!, agent: t!.assignee ? store.agentMap.get(t!.assignee) : null };
    }
    return null;
  }
  function jump(path: string) {
    active = path;
    collapsed = { ...collapsed, [path]: false };
    document.getElementById(`diff-${path}`)?.scrollIntoView({ block: 'start', behavior: ui.reducedMotion ? 'auto' : 'smooth' });
  }
  function onKey(e: KeyboardEvent) {
    const el = e.target as HTMLElement;
    if (el.closest('input, textarea') || e.metaKey || e.ctrlKey || !files.length) return;
    if (e.key === ']' || e.key === '[') {
      const i = files.findIndex((f) => f.path === active);
      jump(files[(i + (e.key === ']' ? 1 : -1) + files.length) % files.length].path);
    }
  }
</script>

<svelte:window onkeydown={onKey} />

{#if !target}
  <EmptyState title="Nothing has changed yet." body="When an agent finishes a task, its diff lands here, next to everything the agents made." />
{:else}
  {#if chips.length > 1}
    <div class="picks" role="tablist" aria-label="Plan">
      {#each chips as c (c.id)}
        <button role="tab" aria-selected={group === c.id} class="pk" class:muted={c.id === 'merged'} onclick={() => choose(c.id)} data-tip={c.label}>
          <span class="truncate">{c.label}</span>
          {#if c.stat}<span class="num st"><span class="add">+{c.stat.added}</span> <span class="del">−{c.stat.removed}</span></span>{:else if c.n != null}<span class="num st">{c.n}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
  <div class="diffs">
    <aside class="side">
      {#snippet planRow(p: (typeof allPlans)[number], label?: string)}
        {@const s = diffstats.plan[p.id]}
        <button class="scope" class:on={target?.kind === 'plan' && target.id === p.id} onclick={() => (ui.diffTarget = { kind: 'plan', id: p.id })} data-tip={p.integration_branch ?? undefined}>
          <span class="s-t truncate">{label ?? p.summary}</span>
          {#if s}<span class="s-n num"><span class="add">+{s.added}</span> <span class="del">−{s.removed}</span></span>{/if}
        </button>
      {/snippet}
      {#snippet taskRow(t: (typeof allTasks)[number])}
        {@const a = t.assignee ? store.agentMap.get(t.assignee) : null}
        {@const s = diffstats.task[t.id]}
        <button class="scope" class:on={target?.kind === 'task' && target.id === t.id} style:--hue={a ? idVar(a.color) : 'var(--line-3)'} onclick={() => (ui.diffTarget = { kind: 'task', id: t.id })} onpointerenter={() => ui.setHover({ taskId: t.id })} onpointerleave={() => ui.setHover(null)}>
          <span class="dot"></span><span class="k">{t.key}</span><span class="s-t truncate">{t.title}</span>
          {#if s}<span class="s-n num"><span class="add">+{s.added}</span> <span class="del">−{s.removed}</span></span>{:else}<span class="s-n">{TASK_STATUS_LABEL[t.status]}</span>{/if}
        </button>
      {/snippet}
      {#if groupPlan}
        {@const mine = tasks.filter((t) => t.plan_id === groupPlan.id)}
        <p class="eyebrow">Integration branch</p>
        {@render planRow(groupPlan, 'Whole plan')}
        {#if mine.length}
          <p class="eyebrow">{mine.length === 1 ? 'Worktree' : `Worktrees · ${mine.length}`}</p>
          {#each mine as t (t.id)}{@render taskRow(t)}{/each}
        {/if}
      {:else if group === 'loose'}
        <p class="eyebrow">{plans.length ? 'Other tasks' : 'Tasks'}</p>
        {#each loose as t (t.id)}{@render taskRow(t)}{/each}
      {:else if group === 'merged'}
        {#if !plans.length && !loose.length}<p class="all-in">Everything is in your branch.</p>{/if}
        <p class="eyebrow">Merged · {merged.length}</p>
        {#each merged as p (p.id)}
          {@render planRow(p)}
          <div class="sub">{#each allTasks.filter((t) => t.plan_id === p.id) as t (t.id)}{@render taskRow(t)}{/each}</div>
        {/each}
      {/if}
      {#if files.length}
        <p class="eyebrow files-h">{files.length} files · <span class="num"><span class="add">+{totals.added}</span> <span class="del">−{totals.removed}</span></span></p>
        <ul class="files">
          {#each files as f (f.path)}
            {@const o = ownerOf(f.path)}
            <li>
              <button class="fl" class:on={active === f.path} style:--hue={o?.agent ? idVar(o.agent.color) : 'var(--line-3)'} onclick={() => jump(f.path)} onpointerenter={() => ui.setHover({ path: f.path })} onpointerleave={() => ui.setHover(null)} data-tip={f.path}>
                <span class="dot"></span><span class="f-n truncate">{f.path.split('/').pop()}</span>
                <span class="f-s num"><span class="add">+{f.added}</span> <span class="del">−{f.removed}</span></span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </aside>
    <div class="main">
      {#if error}<p class="err">Couldn’t load the diff: {error}</p>
      {:else if !files.length && !loading}<EmptyState compact title="No changes in this scope yet." />
      {:else}
        {#each files as f (f.path)}
          {@const o = ownerOf(f.path)}
          <DiffView file={f} hue={o?.agent ? idVar(o.agent.color) : 'var(--line-3)'} who={target.kind === 'plan' && o ? `${o.task.key} · ${o.agent?.name ?? ''}` : undefined} collapsed={!!collapsed[f.path]} ontoggle={() => (collapsed = { ...collapsed, [f.path]: !collapsed[f.path] })} />
        {/each}
      {/if}
    </div>
  </div>
{/if}

<style>
  .diffs { flex: 1; min-height: 0; display: flex; }
  .side { width: 290px; flex-shrink: 0; overflow-y: auto; padding: 4px 14px 24px 22px; display: flex; flex-direction: column; gap: 2px; }
  .side .eyebrow { padding: 14px 8px 6px; }
  .side .eyebrow:first-child { padding-top: 2px; }
  .picks { display: flex; gap: 6px; padding: 0 28px 14px; overflow-x: auto; scrollbar-width: none; flex-shrink: 0; }
  .pk { display: inline-flex; align-items: center; gap: 8px; height: 32px; padding: 0 12px; border-radius: var(--r-full); max-width: 360px; font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); background: var(--surface-1); flex-shrink: 0; }
  .pk:hover { background: var(--surface-2); color: var(--ink-1); }
  .pk[aria-selected='true'] { background: var(--surface-3); color: var(--ink-1); box-shadow: inset 0 0 0 1px var(--line-2); }
  .pk.muted:not([aria-selected='true']) { color: var(--ink-3); }
  .st { color: var(--ink-3); font-weight: var(--w-reg); white-space: nowrap; font-size: var(--t-xs); font-family: var(--font-mono); }
  .sub { padding-left: 12px; }
  .all-in { padding: 6px 10px; color: var(--ink-3); font-size: var(--t-sm); }
  .scope, .fl { display: flex; align-items: center; gap: 8px; width: 100%; height: 34px; padding: 0 10px; border-radius: var(--r-md); text-align: left; font-size: var(--t-md); color: var(--ink-2); }
  .scope:hover, .fl:hover { background: var(--surface-1); color: var(--ink-1); }
  .scope.on, .fl.on { background: var(--surface-3); color: var(--ink-1); }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--hue); flex-shrink: 0; }
  .k { font-family: var(--font-mono); font-size: 11px; color: var(--ink-3); }
  .s-t { flex: 1; }
  .s-n, .f-s { font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; font-family: var(--font-mono); }
  .files { display: flex; flex-direction: column; gap: 1px; }
  .fl { height: 30px; font-size: var(--t-sm); }
  .f-n { flex: 1; font-family: var(--font-mono); font-size: 11.5px; }
  .add { color: var(--ok); }
  .del { color: var(--bad); }
  .main { flex: 1; min-width: 0; overflow-y: auto; padding: 4px 24px 24px 6px; display: flex; flex-direction: column; gap: 12px; }
  .err { color: var(--bad); font-size: var(--t-sm); padding: 20px; }
</style>
