<!--
  A task's acceptance criteria and its checks with their live results: running now, passed (with
  test counts), failed, or not run yet. First the task's own verify commands, then the repo's
  required checks that Codly ran for it. Codly's own run is Observed; an agent's run of the same
  command is Observed too; ticking a criterion is Inferred from them.
-->
<script lang="ts">
  import type { Task } from '../lib/gen/Task';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import { checksPassed, taskChecks } from '../lib/util/checks';
  import { timer } from '../lib/util/format';
  import Icon from '../components/Icon.svelte';
  import InlineText from '../components/InlineText.svelte';
  import Provenance from '../components/Provenance.svelte';

  interface Props {
    task: Task;
  }
  let { task }: Props = $props();

  const checks = $derived.by(() => {
    // The task's finished steps (replaced on change) and its assignee's own tool calls.
    if (task.assignee) void store.revBy.get(task.assignee);
    const marks = store.taskMarks[task.id] ?? [];
    const segs = task.assignee ? (store.live[task.assignee]?.segments ?? []).filter((s) => s.taskId === task.id && (s.activity === 'test' || s.activity === 'execute')) : [];
    return taskChecks(task.verify, marks, segs, (callId) => store.pendingApprovals.some((a) => a.call_id === callId));
  });
  const verified = $derived(task.status === 'done' || checksPassed(checks));
</script>

{#if task.acceptance.length}
  <ul class="crit">
    {#each task.acceptance as a, i (i)}
      <li class:ok={verified}><span class="box">{#if verified}<Icon name="check" size={10} stroke={2.4} />{/if}</span><span class="selectable"><InlineText text={a} /></span></li>
    {/each}
  </ul>
  {#if verified}<p class="src"><Provenance kind="inferred" /> ticked because Codly’s verify run passed</p>{/if}
{/if}
{#if checks.length}
  <ul class="checks">
    {#each checks as c (c.cmd)}
      <li class="c-{c.state}">
        <span class="ic">{#if c.state === 'running'}<span class="pulse"></span>{:else if c.state === 'pass'}<Icon name="check" size={12} stroke={2.2} />{:else if c.state === 'fail'}<Icon name="x" size={12} stroke={2.2} />{:else if c.state === 'wait'}<Icon name="flag" size={12} />{:else}<span class="ring"></span>{/if}</span>
        <code class="cmd truncate">{c.cmd}</code>
        <span class="d num">{c.state === 'running' && c.since ? `running · ${timer(ui.now - c.since)}` : c.detail}</span>
        {#if c.by}<Provenance kind="observed" compact />{/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  .crit { display: flex; flex-direction: column; gap: 6px; margin: 12px 0 4px; font-size: var(--t-md); line-height: 19px; color: var(--ink-2); }
  .crit li { display: flex; gap: 9px; align-items: flex-start; }
  .box { width: 15px; height: 15px; margin-top: 2px; border-radius: 4px; box-shadow: inset 0 0 0 1.5px var(--line-3); display: grid; place-items: center; flex-shrink: 0; color: var(--night); }
  .ok .box { background: var(--ok); box-shadow: none; }
  .src { display: flex; align-items: center; gap: 6px; font-size: var(--t-xs); color: var(--ink-3); margin: 6px 0 2px; }
  .checks { display: flex; flex-direction: column; gap: 4px; margin: 10px 0 8px; padding: 8px 10px; border-radius: var(--r-md); background: var(--surface-1); }
  .checks li { display: flex; align-items: center; gap: 8px; height: 26px; font-size: var(--t-sm); }
  .ic { width: 14px; display: grid; place-items: center; color: var(--ink-3); }
  .c-pass .ic { color: var(--ok); }
  .c-fail .ic { color: var(--bad); }
  .pulse { width: 7px; height: 7px; border-radius: 50%; background: var(--aurora); box-shadow: 0 0 0 3px var(--live-soft); }
  .ring { width: 8px; height: 8px; border-radius: 50%; box-shadow: inset 0 0 0 1.5px var(--line-3); }
  .cmd { font-size: 11.5px; color: var(--ink-1); flex: 0 1 auto; }
  .d { flex: 1; text-align: right; color: var(--ink-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .c-fail .d { color: var(--bad); }
  .c-wait .ic, .c-wait .d { color: var(--amber-ink); }
</style>
