<!--
  Why a plan did not start by itself in a request that runs its plans at once: it may answer a
  message from outside Codly, or its request started from GitHub (the core's `submit_plan_inner`).
  Shown where you run the plan, while it waits: on its card above Run plan, and in the Plan room.
-->
<script lang="ts">
  import type { Plan } from '../../lib/gen/Plan';
  import { store } from '../../lib/state/store.svelte';
  import { waitReason } from '../../lib/util/plan';
  import Icon from '../../components/Icon.svelte';

  interface Props {
    plan: Plan;
    /** In the Plan room, which has no card around it: the room's own side margins. */
    room?: boolean;
  }
  let { plan, room = false }: Props = $props();

  const reason = $derived.by(() => {
    void store.rev;
    return plan.status === 'awaiting_approval' ? waitReason(store.decisions, plan.id) : null;
  });
</script>

{#if reason}
  <p class="waits" class:room role="note"><Icon name="alert" size={14} /><span>{reason}</span></p>
{/if}

<style>
  .waits { display: flex; gap: 8px; align-items: flex-start; padding: 8px 12px; border-radius: var(--r-md); background: var(--amber-soft); color: var(--ink-1); font-size: var(--t-md); line-height: var(--lh-md); }
  .waits.room { margin: 0 28px 14px; flex-shrink: 0; }
  .waits :global(svg) { color: var(--amber); margin-top: 3px; flex-shrink: 0; }
</style>
