<!--
  List (the Plan room's default): the plan's tasks top to bottom, grouped by where they are —
  what needs you, what is being worked on, what comes next, what is done. Each row says who
  does it and what it waits on. Tasks outside any plan sit at the bottom.
-->
<script lang="ts">
  import type { Plan } from '../../lib/gen/Plan';
  import type { Task } from '../../lib/gen/Task';
  import { registerListNav } from '../../lib/state/keys';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import EmptyState from '../../components/EmptyState.svelte';
  import Icon from '../../components/Icon.svelte';
  import TaskRow from '../../home/cards/TaskRow.svelte';

  interface Props {
    plan: Plan | null;
    tasks: Task[];
  }
  let { plan, tasks }: Props = $props();

  type Group = { id: string; title: string; hint: string; tasks: Task[] };
  const waitingAgents = $derived(new Set(store.pendingApprovals.map((a) => a.agent_id)));
  const groupOf = (t: Task): string => {
    if (t.assignee && waitingAgents.has(t.assignee) && store.agentMap.get(t.assignee)?.task_id === t.id) return 'you';
    switch (t.status) {
      case 'blocked': case 'failed': case 'changes_requested': return 'you';
      case 'running': case 'verifying': case 'reviewing': case 'merging': case 'ready': return 'now';
      case 'done': return 'done';
      case 'cancelled': return 'cancelled';
      default: return 'next';
    }
  };
  const order = (a: Task, b: Task) => a.key.localeCompare(b.key, undefined, { numeric: true });
  const groups = $derived.by((): Group[] => {
    const by: Record<string, Task[]> = { you: [], now: [], next: [], done: [], cancelled: [] };
    for (const t of tasks) by[groupOf(t)].push(t);
    const out: Group[] = [
      { id: 'you', title: 'Needs you', hint: 'Waiting for your answer, or stuck', tasks: by.you.sort(order) },
      { id: 'now', title: 'In progress', hint: 'An agent is on it', tasks: by.now.sort(order) },
      { id: 'next', title: plan?.status === 'awaiting_approval' ? 'Proposed' : 'Up next', hint: plan?.status === 'awaiting_approval' ? 'Starts when you run the plan' : 'Starts when what it waits on is merged', tasks: by.next.sort(order) },
      { id: 'done', title: 'Done', hint: 'Merged into the integration branch', tasks: by.done.sort(order) },
      { id: 'cancelled', title: 'Cancelled', hint: '', tasks: by.cancelled.sort(order) },
    ];
    return out.filter((g) => g.tasks.length);
  });
  const adhoc = $derived(store.tasks.filter((t) => !t.plan_id).sort((a, b) => b.updated_at - a.updated_at));
  let showAdhoc = $state(false);

  const flat = $derived([...groups.flatMap((g) => g.tasks), ...(showAdhoc ? adhoc : [])]);
  $effect(() => registerListNav({ next: () => step(1), prev: () => step(-1) }));
  function step(d: number) {
    if (!flat.length) return;
    const i = flat.findIndex((t) => t.id === ui.selectedTaskId);
    const next = flat[(i + d + flat.length) % flat.length];
    ui.selectTask(next.id);
    document.querySelector(`[data-task="${next.id}"]`)?.scrollIntoView({ block: 'nearest' });
  }
</script>

<div class="list">
  <div class="col">
    {#if !tasks.length}
      <EmptyState compact title="No plan yet." body="Ask the Lead for something on Home. Its plan lands here as a checklist: who does what, in which order." />
    {/if}
    {#each groups as g (g.id)}
      <section class="group g-{g.id}">
        <header><h2>{g.title}</h2><span class="n num">{g.tasks.length}</span>{#if g.hint}<span class="hint">{g.hint}</span>{/if}</header>
        <ol>{#each g.tasks as t (t.id)}<TaskRow task={t} {plan} detail />{/each}</ol>
      </section>
    {/each}
    {#if adhoc.length}
      <section class="group adhoc">
        <button class="ah" onclick={() => (showAdhoc = !showAdhoc)} aria-expanded={showAdhoc}>
          <Icon name={showAdhoc ? 'chevron-down' : 'chevron-right'} size={13} /><h2>Not in a plan</h2><span class="n num">{adhoc.length}</span><span class="hint">Tasks you created by hand, and earlier work</span>
        </button>
        {#if showAdhoc}<ol>{#each adhoc as t (t.id)}<TaskRow task={t} plan={null} detail />{/each}</ol>{/if}
      </section>
    {/if}
  </div>
</div>

<style>
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 4px 24px 32px; }
  .col { max-width: 860px; margin: 0 auto; display: flex; flex-direction: column; gap: 22px; }
  .group { display: flex; flex-direction: column; gap: 4px; }
  header, .ah { display: flex; align-items: baseline; gap: 8px; padding: 0 14px 6px; }
  .ah { align-items: center; text-align: left; border-radius: var(--r-md); padding: 6px 14px; color: var(--ink-2); }
  .ah:hover { background: var(--surface-2); }
  h2 { font-size: var(--t-md); font-weight: var(--w-semi); color: var(--ink-1); }
  .n { font-size: var(--t-sm); color: var(--ink-3); }
  .hint { font-size: var(--t-sm); color: var(--ink-4); margin-left: 4px; }
  .g-you h2 { color: var(--amber-ink); }
  ol { display: flex; flex-direction: column; gap: 4px; }
  .group :global(.row) { background: var(--surface-1); }
  .group :global(.row:hover), .group :global(.row.lit) { background: var(--surface-2); }
  .g-you :global(.row) { background: color-mix(in srgb, var(--amber) 8%, var(--surface-1)); }
</style>
