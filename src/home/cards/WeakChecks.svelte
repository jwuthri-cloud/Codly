<!--
  The tasks of a plan with no real check: none of their verify commands can fail on the work (the
  planner's mark, `weak_checks` in the core, kept on the plan itself: it is there after a restart
  and however long the history is). Shown with the plan before you run it, and after: a plan that
  runs at once (an Auto or Direct request) never waited with this on screen.
-->
<script lang="ts">
  import type { Plan } from '../../lib/gen/Plan';
  import { weakAdvice } from '../../lib/util/plan';
  import Icon from '../../components/Icon.svelte';

  interface Props {
    plan: Plan;
    /** In the Plan room, which has no card around it: the room's own side margins. */
    room?: boolean;
  }
  let { plan, room = false }: Props = $props();

  const advice = $derived(weakAdvice(plan));
</script>

{#if advice && plan.weak_checks.length}
  <ul class="weak" class:room aria-label="Tasks with no real check">
    {#each plan.weak_checks as w (w.task)}
      <li>
        <Icon name="alert" size={14} />
        <span>
          <strong>{w.task}</strong> has no check that can fail on the work: {#each w.checks as c, i (i)}{i ? ', ' : ''}<code>{c.command}</code> {c.why}{/each}.
          The plan’s checks pass this task whatever it changes. {advice}
        </span>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .weak { display: flex; flex-direction: column; gap: 6px; }
  .weak.room { margin: 0 28px 14px; flex-shrink: 0; }
  li { display: flex; gap: 8px; align-items: flex-start; padding: 8px 12px; border-radius: var(--r-md); background: var(--amber-soft); color: var(--ink-1); font-size: var(--t-md); line-height: var(--lh-md); }
  li :global(svg) { color: var(--amber); margin-top: 3px; flex-shrink: 0; }
  code { font-size: 11.5px; overflow-wrap: anywhere; }
</style>
