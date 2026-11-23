<!--
  Diffs, one plan at a time like the Plan room: a chip per plan (plus Other tasks and Merged)
  picks it. Left: the whole plan (its integration branch) and each task's worktree with their
  diffstat, then the files with +/−; right: unified view with line numbers. ] / [ jump between
  files. Plans already in your branch (merged from Codly or by hand) or kept on their own branch
  sit under Merged, with their tasks.
  It is also where you review: comment on a line or a file, tick files as viewed, open one in
  the editor, and Send back gives all your comments to the agent that did the work (a new
  request, for work that is merged or finished: the comments stay here until you send its
  message; Discard takes them out of that request too). A comment on a file that left the diff since stays in the bar above Send back,
  where you can read and remove it.
  A task's diff has a second scope, "Last turn": only what the last turn in the task's worktree
  changed, committed or not (from the checkpoint Codly kept before that turn). It is for
  reading: comments go on the whole task's diff, and the ones you wrote wait there. Each read
  of it takes a snapshot of the worktree, so it is read one at a time, and when a turn starts or
  ends, not on each file the agent writes.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { ide } from '../../ide/ide.svelte';
  import { api } from '../../lib/api';
  import type { Agent } from '../../lib/gen/Agent';
  import type { LastTurn } from '../../lib/gen/LastTurn';
  import type { Task } from '../../lib/gen/Task';
  import { discardReview, mergePlan, reviewDraft, sendBack, waitingRequest } from '../../lib/state/actions';
  import { diffstats } from '../../lib/state/diffstats.svelte';
  import { review, reviewKey } from '../../lib/state/review.svelte';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { diffStats, type DiffFile } from '../../lib/util/diff';
  import { ago, plural } from '../../lib/util/format';
  import { idVar, TASK_STATUS_LABEL } from '../../lib/util/labels';
  import { oneAtATime } from '../../lib/util/oneAtATime';
  import { discardWords, editorPlace, heavy, HEAVY_LABEL, inTurn, offDiff, parseDiffKeeping, placeId, reviewBarWords, turnReadKey, type DiffParts, type ReviewComment, type ReviewDraft } from '../../lib/util/review';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';
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
  const into = $derived(store.workspace?.current_branch ?? store.workspace?.default_branch ?? 'main');
  // A done plan merges straight in (a failed merge goes to its Lead, see `mergePlan`).
  const mergeable = $derived(!!groupPlan && groupPlan.status === 'done' && !store.mergeOf(groupPlan.id));
  let merging = $state(false);
  async function merge(planId: string) {
    merging = true;
    await mergePlan(planId);
    merging = false;
  }
  // A task with no plan to merge through: a new request asks a Lead to land it.
  const looseTask = $derived(group === 'loose' && target?.kind === 'task' ? store.taskMap.get(target.id) : null);
  function landWithLead(t: NonNullable<typeof looseTask>) {
    ui.newRequest();
    ui.seedComposer(
      ui.request,
      `Bring ${t.key} “${t.title}” into ${into}. Its work is on the branch ${t.branch}${t.status === 'done' ? '' : ` (the task is ${TASK_STATUS_LABEL[t.status].toLowerCase()}, so check it is complete)`}. Merge it and resolve any conflicts.`,
    );
  }
  function choose(id: string) {
    if (id === 'loose') ui.diffTarget = { kind: 'task', id: loose[0].id };
    else ui.diffTarget = { kind: 'plan', id: id === 'merged' ? merged[0].id : id };
  }

  // Raw: the parsed diff is only ever replaced, and a deep proxy over every line of a large
  // diff is slow to read.
  let files = $state.raw<DiffFile[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let active = $state<string | null>(null);
  /** The diff on screen: its name (`key`), its text, and each file's part of it, parsed. */
  let drawn: { key: string; text: string; parts: DiffParts } | null = null;
  /** The diff whose files are on screen (its `key`); null while another one loads. */
  let loaded = $state<string | null>(null);
  /** You picked "Last turn": a task's diff shows only what its last turn changed. */
  let turnOnly = $state(false);
  const lastTurn = $derived(turnOnly && target?.kind === 'task');
  /** The turn on screen in the "Last turn" scope; null when Codly kept no turn of the task. */
  let turn = $state.raw<LastTurn | null>(null);
  /** Your comments are on the task or the plan, whichever scope is on screen. */
  const notesKey = $derived(target ? reviewKey(target) : '');
  /** The diff on screen: the last turn of a task has its own folds and ticks. */
  const key = $derived(lastTurn ? `${notesKey}:turn` : notesKey);
  let main = $state<HTMLElement | null>(null);

  const version = $derived.by(() => {
    if (!target) return '';
    void store.rev;
    if (target.kind === 'plan') return store.tasks.filter((t) => t.plan_id === target.id).map((t) => `${t.id}:${t.status}`).join(',');
    const t = store.taskMap.get(target.id);
    // The last turn is not read again on each file the agent writes: see `turnReadKey`.
    if (lastTurn) return turnReadKey(t?.status, t?.assignee ? store.agentMap.get(t.assignee)?.status : undefined);
    const l = t?.assignee ? store.live[t.assignee] : null;
    return `${t?.status}:${l ? Object.values(l.files).filter((f) => f.modified).length : 0}`;
  });
  let seq = 0;
  /** Try again: bumps to load the same diff once more. */
  let retry = $state(0);
  /** One read (the one numbered `my`) of the diff named `name`: its answer is drawn, unless a newer read started since. */
  async function draw(my: number, name: string, read: Promise<{ text: string; turn: LastTurn | null }>) {
    try {
      const got = await read;
      if (my !== seq) return;
      const text = got.text;
      error = null;
      loaded = name;
      turn = got.turn;
      // The same text again (a task changed status, the code did not): nothing to draw.
      if (drawn?.text === text) return;
      // Files whose part of the text did not change keep their objects, so only the others are drawn again.
      const parsed = parseDiffKeeping(text, drawn?.parts ?? new Map());
      drawn = { key: name, text, parts: parsed.parts };
      files = parsed.files;
      if (!files.find((f) => f.path === active)) active = files[0]?.path ?? null;
    } catch (e) {
      if (my === seq) error = e instanceof Error ? e.message : String(e);
    } finally {
      if (my === seq) loading = false;
    }
  }
  // A read of the last turn starts git in the worktree and takes a snapshot of it: never two
  // at once. It reads what is asked for when it starts (the newest `seq`), not when it was asked.
  const readTurn = oneAtATime(async () => {
    const t = target;
    if (!t || !lastTurn) return;
    await draw(seq, key, api.lastTurnDiff({ kind: 'task', id: t.id }).then((kept) => ({ text: kept?.diff ?? '', turn: kept })));
  });
  $effect(() => {
    const t = target;
    void version;
    void retry;
    if (!t) return;
    const my = ++seq;
    const name = key;
    // Another diff: the one on screen goes at once, so no comment lands on a diff you left.
    if (drawn?.key !== name) {
      drawn = null;
      files = [];
      loaded = null;
    }
    loading = true;
    if (lastTurn) untrack(() => void readTurn());
    else void draw(my, name, (t.kind === 'plan' ? api.planDiff(t.id) : api.taskDiff(t.id)).then((text) => ({ text, turn: null })));
  });
  const totals = $derived(diffStats(files));

  const targetTask = $derived(target?.kind === 'task' ? (store.taskMap.get(target.id) ?? null) : null);
  /** In the "Last turn" scope while a turn of the task's agent is on: the diff on screen is not read on each step of it. */
  const turnNote = $derived.by(() => {
    const worker = lastTurn && targetTask?.assignee ? store.agentMap.get(targetTask.assignee) : null;
    return worker && inTurn(worker.status) ? ` ${worker.name} is working: this is read again when its turn ends.` : '';
  });
  /** Which task (and agent) wrote each file: from Codly's commits, then from what its agent
   *  changes now. One pass for the whole diff; the first task of the scope that has a file owns it. */
  const owners = $derived.by(() => {
    void store.rev;
    const out = new Map<string, { task: Task; agent: Agent | null }>();
    const scope = target?.kind === 'plan' ? store.tasks.filter((t) => t.plan_id === target.id) : targetTask ? [targetTask] : [];
    for (const t of scope) {
      const owner = { task: t, agent: (t.assignee && store.agentMap.get(t.assignee)) || null };
      const own = (path: string) => out.has(path) || out.set(path, owner);
      for (const m of store.taskMarks[t.id] ?? []) m.files?.forEach(own);
      const live = t.assignee ? store.live[t.assignee]?.files : null;
      if (live) for (const [path, f] of Object.entries(live)) if (f.modified) own(path);
    }
    return out;
  });
  function jump(path: string) {
    active = path;
    review.fold(key, path, false);
    document.getElementById(`diff-${path}`)?.scrollIntoView({ block: 'start', behavior: ui.reducedMotion ? 'auto' : 'smooth' });
  }

  // ───────────── review: comments, viewed, the editor ─────────────

  const NONE: ReviewComment[] = [];
  const comments = $derived(review.comments(notesKey));
  const byFile = $derived.by(() => {
    const out = new Map<string, ReviewComment[]>();
    for (const c of comments) out.set(c.path, [...(out.get(c.path) ?? []), c]);
    return out;
  });
  /** Comments on files that left the diff since you wrote them: no file on screen shows them.
   *  (The last turn shows a part of the task's diff: a file that is not in it did not leave.) */
  const gone = $derived(!lastTurn && loaded === key && !error ? offDiff(comments, files.map((f) => f.path)) : NONE);
  const viewedCount = $derived(files.filter((f) => review.viewed(key, f)).length);
  /** Where a file opens in the editor: the worktree of the task that wrote it, or your
   *  repository once the plan is merged (see `editorPlace`). */
  function placeOf(path: string) {
    const task = targetTask ?? owners.get(path)?.task ?? null;
    const planId = target?.kind === 'plan' ? target.id : task?.plan_id;
    const plan = (planId && store.planMap.get(planId)) || null;
    const worker = (task?.assignee && store.agentMap.get(task.assignee)) || null;
    return editorPlace(task, plan, worker, store.leadOf(plan?.thread ?? 'lead') ?? store.leads[0] ?? null);
  }
  // A task that is not done takes the comments itself; merged or finished work gets a new request.
  const toWorker = $derived(!!targetTask && targetTask.status !== 'done');
  const workerName = $derived((targetTask?.assignee && store.agentMap.get(targetTask.assignee)?.name) || 'its worker');
  /** Finished work whose comments are in a new request already, not sent yet. */
  const waiting = $derived(!toWorker && !!waitingRequest(notesKey));
  let sending = $state(false);
  async function send() {
    if (!target || sending) return;
    sending = true;
    await sendBack(target, files.map((f) => f.path));
    sending = false;
  }
  /** Discard asks once more, and says where else the comments are (`draft`, read at the first press). */
  let discarding = $state<{ draft: ReviewDraft } | null>(null);
  $effect(() => {
    void comments;
    discarding = null;
  });
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
        {#if mergeable}<button class="btn sm primary land" disabled={merging} onclick={() => merge(groupPlan.id)}><Icon name="merge" size={13} />Merge into {into}</button>{/if}
        {#if mine.length}
          <p class="eyebrow">{mine.length === 1 ? 'Worktree' : `Worktrees · ${mine.length}`}</p>
          {#each mine as t (t.id)}{@render taskRow(t)}{/each}
        {/if}
      {:else if group === 'loose'}
        <p class="eyebrow">{plans.length ? 'Other tasks' : 'Tasks'}</p>
        {#each loose as t (t.id)}{@render taskRow(t)}{/each}
        {#if looseTask?.branch && files.length}
          <button class="btn sm land" onclick={() => landWithLead(looseTask)} data-tip="Opens a new request for a Lead, with the prompt ready to send"><Icon name="merge" size={13} />Merge {looseTask.key} with the Lead</button>
        {/if}
      {:else if group === 'merged'}
        {#if !plans.length && !loose.length}<p class="all-in">Everything is in your branch.</p>{/if}
        <p class="eyebrow">Merged · {merged.length}</p>
        {#each merged as p (p.id)}
          {@render planRow(p)}
          <div class="sub">{#each allTasks.filter((t) => t.plan_id === p.id) as t (t.id)}{@render taskRow(t)}{/each}</div>
        {/each}
      {/if}
      {#if target.kind === 'task'}
        <div class="lenses turn-pick" role="radiogroup" aria-label="Which changes of the task">
          <button role="radio" aria-checked={!turnOnly} onclick={() => (turnOnly = false)} data-tip="Everything the task changed and Codly committed">Whole task</button>
          <button role="radio" aria-checked={turnOnly} onclick={() => (turnOnly = true)} data-tip="Only what the last turn in the task’s worktree changed, committed or not">Last turn</button>
        </div>
      {/if}
      {#if files.length}
        <p class="eyebrow files-h">{plural(files.length, 'file')} ·<span class="num"><span class="add">+{totals.added}</span> <span class="del">−{totals.removed}</span></span>{#if viewedCount} · {viewedCount} viewed{/if}</p>
        <ul class="files">
          {#each files as f (f.path)}
            {@const o = owners.get(f.path)}
            {@const n = byFile.get(f.path)?.length ?? 0}
            <li>
              <button class="fl" class:on={active === f.path} class:seen={review.viewed(key, f)} style:--hue={o?.agent ? idVar(o.agent.color) : 'var(--line-3)'} onclick={() => jump(f.path)} onpointerenter={() => ui.setHover({ path: f.path })} onpointerleave={() => ui.setHover(null)} data-tip={f.path}>
                <span class="dot"></span><span class="f-n truncate">{f.path.split('/').pop()}</span>
                {#if n}<span class="f-c num" aria-label="{n} {n === 1 ? 'comment' : 'comments'} not sent">{n}</span>{/if}
                {#if review.viewed(key, f)}<span class="f-v" aria-label="Viewed"><Icon name="check" size={11} stroke={2} /></span>{/if}
                <span class="f-s num"><span class="add">+{f.added}</span> <span class="del">−{f.removed}</span></span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </aside>
    <div class="main" bind:this={main}>
      {#if error}<p class="err">Couldn’t load the diff: {error}<button class="btn sm" disabled={loading} onclick={() => retry++}>Try again</button></p>
      {:else if loading && !files.length}<EmptyState compact title="Loading the diff…" />
      {:else if lastTurn && !turn && !loading}<EmptyState compact title="No turn to show for this task." body={`Codly keeps the files before each turn in the task’s worktree. That worktree is gone or holds other work now, or no turn ran there yet.${turnNote}`} />
      {:else if lastTurn && turn && !files.length && !loading}<EmptyState compact title="The last turn changed no file." body={`It started with “${turn.label}” (${ago(turn.ts, ui.now)}).${turnNote}`} />
      {:else if !files.length && !loading}<EmptyState compact title="No changes in this scope yet." />
      {:else}
        {#if lastTurn && turn}<p class="turn-h">Since the turn that started with “{turn.label}” ({ago(turn.ts, ui.now)}). To comment, switch to Whole task.{turnNote}</p>{/if}
        {#each files as f (f.path)}
          {@const o = owners.get(f.path)}
          {@const why = heavy(f)}
          {@const place = placeOf(f.path)}
          {@const editor = place.agentId}
          <DiffView
            file={f}
            hue={o?.agent ? idVar(o.agent.color) : 'var(--line-3)'}
            who={target.kind === 'plan' && o ? `${o.task.key} · ${o.agent?.name ?? ''}` : undefined}
            kind={why ? HEAVY_LABEL[why] : undefined}
            root={main}
            collapsed={review.folded(key, f)}
            ontoggle={() => review.fold(key, f.path, !review.folded(key, f))}
            comments={lastTurn ? NONE : (byFile.get(f.path) ?? NONE)}
            oncomment={lastTurn ? undefined : (at, text) => review.comment(notesKey, at, text)}
            viewed={review.viewed(key, f)}
            onviewed={(on) => review.setViewed(key, f, on)}
            openTip={place.tip}
            onopen={editor ? () => void ide.openPath(editor, f.path) : undefined}
          />
        {/each}
      {/if}
      <!-- Also when the diff did not load or is empty by now: what you wrote is never out of reach. -->
      {#if comments.length}
        <div class="review" role="region" aria-label="Your review">
          {#if gone.length}
            <ul class="gone" aria-label="Comments on files that left the diff">
              {#each gone as c (c.path + ' ' + placeId(c) + c.quote)}
                <li>
                  <p class="g-w"><span class="mono">{c.path}{c.line == null ? '' : `:${c.line}`}</span> is not in this diff any more. Send back still sends your comment{c.quote == null ? '' : ', with the line as it was'}.</p>
                  {#if c.quote != null}<p class="g-q">{c.quote}</p>{/if}
                  <p class="g-t">{c.text}</p>
                  <button class="btn sm ghost" onclick={() => review.comment(notesKey, c, '')}>Remove</button>
                </li>
              {/each}
            </ul>
          {/if}
          <div class="r-row">
            {#if lastTurn}
              <p class="r-t"><b>{comments.length} {comments.length === 1 ? 'comment' : 'comments'} not sent.</b> {comments.length === 1 ? 'It is' : 'They are'} on the whole task’s diff.</p>
              <button class="btn sm" onclick={() => (turnOnly = false)}>Whole task</button>
            {:else}
            <p class="r-t"><b>{comments.length} {comments.length === 1 ? 'comment' : 'comments'} not sent.</b> {discarding ? discardWords(comments.length, discarding.draft) : reviewBarWords(comments.length, toWorker ? workerName : null, waiting ? review.inHand(notesKey) : null)}</p>
            {#if discarding}
              <button class="btn sm danger" onclick={() => discardReview(notesKey)}>Discard {comments.length === 1 ? 'it' : `all ${comments.length}`}</button>
              <button class="btn sm ghost" onclick={() => (discarding = null)}>Keep</button>
            {:else}
              <button class="btn sm ghost" onclick={() => (discarding = { draft: reviewDraft(notesKey) })}>Discard</button>
              <button class="btn sm primary" disabled={sending} onclick={send}><Icon name="send" size={13} />Send back</button>
            {/if}
            {/if}
          </div>
        </div>
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
  .land { margin: 6px 8px 2px; align-self: flex-start; }
  .turn-pick { margin: 12px 8px 0; align-self: flex-start; flex-shrink: 0; }
  .turn-h { flex-shrink: 0; padding: 0 8px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-3); }
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
  .fl.seen .f-n { color: var(--ink-3); }
  .f-c { min-width: 16px; height: 16px; padding: 0 4px; border-radius: var(--r-full); background: var(--amber-soft); color: var(--amber-ink); font-size: var(--t-xs); line-height: 16px; text-align: center; }
  .f-v { display: inline-flex; color: var(--ok); }
  .add { color: var(--ok); }
  .del { color: var(--bad); }
  .main { flex: 1; min-width: 0; overflow-y: auto; padding: 4px 24px 24px 6px; display: flex; flex-direction: column; gap: 12px; }
  .err { display: flex; align-items: center; gap: 10px; color: var(--bad); font-size: var(--t-sm); padding: 20px; }
  /* Your comments wait here, in view under the files, until you send them back. */
  .review { position: sticky; bottom: 0; z-index: 2; flex-shrink: 0; margin-top: auto; display: flex; flex-direction: column; gap: 8px; padding: 10px 12px 10px 14px; border-radius: var(--r-md); background: var(--glass-solid); box-shadow: inset 0 0 0 1px var(--line-2), inset 3px 0 0 var(--amber); }
  .r-row { display: flex; align-items: center; gap: 8px; }
  /* Comments on files that left the diff: no file shows them, so they are listed here. */
  .gone { display: flex; flex-direction: column; gap: 6px; max-height: 180px; overflow-y: auto; }
  .gone li { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: start; gap: 2px 8px; padding: 8px 10px; border-radius: var(--r-sm); background: var(--amber-soft); font-size: var(--t-sm); line-height: var(--lh-sm); }
  .gone li > p { grid-column: 1; min-width: 0; overflow-wrap: anywhere; }
  .gone li > button { grid-column: 2; grid-row: 1; }
  .g-w { color: var(--ink-2); }
  .g-q { font-family: var(--font-mono); font-size: 11.5px; white-space: pre; overflow-x: auto; color: var(--ink-2); }
  .g-t { white-space: pre-wrap; color: var(--ink-1); }
  .r-t { flex: 1; min-width: 0; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .r-t b { color: var(--ink-1); font-weight: var(--w-semi); }
</style>
