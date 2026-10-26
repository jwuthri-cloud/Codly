<!--
  A task on the plan canvas: its agent's hue on the edge, the agent and its sentence, progress
  from the agent's own list, and what it may write.
-->
<script lang="ts">
  import type { Task } from '../../lib/gen/Task';
  import { narrate } from '../../lib/state/narration';
  import { store } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { idVar, PROVIDER_LABEL, TASK_STATUS_LABEL, taskState } from '../../lib/util/labels';
  import { basename } from '../../lib/util/sanitize';
  import Portrait from '../../components/Portrait.svelte';
  import Say from '../../components/Say.svelte';
  import StateMark from '../../components/StateMark.svelte';

  interface Props {
    task: Task;
    compact?: boolean;
  }
  let { task, compact = false }: Props = $props();
  const agent = $derived(task.assignee ? (store.agentMap.get(task.assignee) ?? null) : null);
  const n = $derived(agent && agent.task_id === task.id ? narrate(agent.id) : null);
  const progress = $derived.by(() => {
    if (task.status === 'done') return 1;
    if (!agent || agent.task_id !== task.id) return null;
    void store.rev;
    const todo = store.live[agent.id]?.todo ?? [];
    return todo.length ? todo.filter((s) => s.status === 'completed').length / todo.length : null;
  });
  const tone = $derived.by(() => {
    const st = taskState(task.status);
    if (n?.tone === 'wait') return 'wait' as const;
    return st === 'working' ? ('live' as const) : st === 'waiting' ? ('wait' as const) : st;
  });
  const scope = $derived(task.write_scope.length ? [task.write_scope[0].replace(/\/\*\*$/, '/'), ...task.write_scope.slice(1, 2).map((g) => basename(g.replace(/\/\*\*$/, '')))].join(' · ') + (task.write_scope.length > 2 ? ` +${task.write_scope.length - 2}` : '') : 'any file');
  const lit = $derived(ui.hover.taskId === task.id || (!!agent && ui.hover.agentId === agent.id));
</script>

<button
  class="node t-{tone}"
  class:lit
  class:compact
  class:sel={ui.selectedTaskId === task.id}
  style:--hue={agent ? idVar(agent.color) : 'var(--line-3)'}
  data-task={task.id}
  data-drag-ok={compact ? '' : undefined}
  onclick={() => ui.selectTask(task.id)}
  onpointerenter={() => ui.setHover({ taskId: task.id })}
  onpointerleave={() => ui.setHover(null)}
>
  <span class="top"><span class="key">{task.key}</span><span class="st"><StateMark {tone} size={8} />{TASK_STATUS_LABEL[task.status]}</span></span>
  <span class="title">{task.title}</span>
  {#if agent}
    <span class="who"><Portrait {agent} size={20} /><span class="name">{agent.name}</span><span class="say truncate">{#if n}<Say parts={n.parts} />{:else}{PROVIDER_LABEL[agent.provider]}{/if}</span></span>
  {:else}
    <span class="who hint">{task.agent_hint ? `${PROVIDER_LABEL[task.agent_hint]}, suggested` : 'Any free agent'}{task.status_reason ? ` · ${task.status_reason}` : ''}</span>
  {/if}
  {#if !compact}
    <span class="bar" aria-hidden="true"><i style:width="{(progress ?? 0) * 100}%"></i></span>
    <code class="scope truncate">{scope}</code>
  {/if}
</button>

<style>
  .node { position: relative; display: flex; flex-direction: column; gap: 6px; width: 100%; padding: 14px 16px 13px 18px; border-radius: var(--r-lg); text-align: left; background: var(--surface-2); box-shadow: inset 3px 0 0 var(--hue); transition: background-color var(--d-2) var(--ease-out), box-shadow var(--d-2) var(--ease-out), transform var(--d-3) var(--spring); }
  .node:hover, .node.lit { background: var(--surface-3); }
  .node.lit { box-shadow: inset 3px 0 0 var(--hue), inset 0 0 0 1px color-mix(in srgb, var(--hue) 50%, transparent); }
  .node.sel { box-shadow: inset 3px 0 0 var(--hue), inset 0 0 0 1.5px color-mix(in srgb, var(--hue) 80%, transparent); }
  .t-wait { background: color-mix(in srgb, var(--amber) 12%, var(--surface-2)); }
  .top { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .key { font-family: var(--font-mono); font-size: 11px; color: var(--ink-3); }
  .st { display: inline-flex; align-items: center; gap: 5px; font-size: var(--t-xs); color: var(--ink-3); }
  .t-live .st { color: var(--live); }
  .t-done .st { color: var(--ok); }
  .t-wait .st { color: var(--amber-ink); }
  .t-blocked .st { color: var(--bad); }
  .title { font-size: 15px; line-height: 20px; font-weight: var(--w-semi); color: var(--ink-1); }
  .t-done .title { color: var(--ink-2); }
  .who { display: flex; align-items: center; gap: 7px; min-width: 0; font-size: var(--t-sm); color: var(--ink-2); }
  .who.hint { color: var(--ink-3); }
  .name { font-weight: var(--w-semi); color: var(--ink-1); white-space: nowrap; }
  .say { color: var(--ink-2); }
  .bar { height: 3px; border-radius: 2px; background: var(--line); overflow: hidden; margin-top: 2px; }
  .bar i { display: block; height: 100%; background: var(--hue); border-radius: 2px; transition: width var(--d-5) var(--spring); }
  .t-live .bar i { background: var(--aurora); }
  .scope { font-size: 10.5px; color: var(--ink-3); }
  .compact { padding: 11px 13px 11px 15px; gap: 5px; }
  .compact .title { font-size: var(--t-md); line-height: 18px; }
</style>
