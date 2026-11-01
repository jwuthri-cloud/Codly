<!--
  Diffs: the plan's integration diff and each task's diff. Left: scopes with their diffstat, then
  the files with +/−; right: unified view with line numbers. ] / [ jump between files.
  Plans already in your branch (merged from Codly or by hand) or kept on their own branch fold
  away under Merged, with their tasks.
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
  const target = $derived(
    ui.diffTarget ?? (plans[0] ? { kind: 'plan' as const, id: plans[0].id } : tasks[0] ? { kind: 'task' as const, id: tasks[0].id } : merged[0] ? { kind: 'plan' as const, id: merged[0].id } : null),
  );
  let showMerged = $state(false);
  // Open while you look at something in it.
  const mergedOpen = $derived(
    showMerged || (!!target && (target.kind === 'plan' ? settled.has(target.id) : settled.has(store.taskMap.get(target.id)?.plan_id ?? ''))),
  );

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
  <div class="diffs">
    <aside class="side">
      {#snippet planRow(p: (typeof allPlans)[number])}
        {@const s = diffstats.plan[p.id]}
        <button class="scope" class:on={target?.kind === 'plan' && target.id === p.id} onclick={() => (ui.diffTarget = { kind: 'plan', id: p.id })}>
          <span class="s-t truncate">{p.summary}</span>
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
      {#if plans.length}
        <p class="eyebrow">{plans.length > 1 ? 'Plans' : 'Plan'}</p>
        {#each plans as p (p.id)}
          {@render planRow(p)}
          <div class="sub">{#each tasks.filter((t) => t.plan_id === p.id) as t (t.id)}{@render taskRow(t)}{/each}</div>
        {/each}
      {/if}
      {#if loose.length}
        <p class="eyebrow">{plans.length ? 'Other tasks' : 'Tasks'}</p>
        {#each loose as t (t.id)}{@render taskRow(t)}{/each}
      {:else if !plans.length}
        <p class="all-in">Everything is in your branch.</p>
      {/if}
      {#if merged.length}
        <button class="eyebrow fold" aria-expanded={mergedOpen} onclick={() => (showMerged = !mergedOpen)}>
          <span class="chev" class:open={mergedOpen}>›</span>Merged · {merged.length}
        </button>
        {#if mergedOpen}
          {#each merged as p (p.id)}
            {@render planRow(p)}
            <div class="sub">{#each allTasks.filter((t) => t.plan_id === p.id) as t (t.id)}{@render taskRow(t)}{/each}</div>
          {/each}
        {/if}
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
  .fold { display: flex; align-items: center; gap: 4px; width: 100%; text-align: left; cursor: pointer; }
  .fold:hover { color: var(--ink-1); }
  .chev { display: inline-block; width: 10px; transition: transform 0.15s; }
  .chev.open { transform: rotate(90deg); }
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
