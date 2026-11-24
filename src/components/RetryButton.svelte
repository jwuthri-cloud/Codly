<!--
  The Retry beside the words "Retry to resume" (a stopped task's reason, in an agent's line):
  the same move as the task sheet's Retry. Nothing shows for a task those words are not on.
-->
<script lang="ts">
  import type { Task } from '../lib/gen/Task';
  import { moveTask } from '../lib/state/actions';
  import { retryTarget } from '../lib/util/retry';

  interface Props {
    task: Task | null | undefined;
  }
  let { task }: Props = $props();

  const target = $derived(retryTarget(task));
  let busy = $state(false);
  async function retry(e: MouseEvent) {
    // A Retry inside a row that opens something: it only retries.
    e.stopPropagation();
    if (!target || busy) return;
    busy = true;
    await moveTask(target, 'ready');
    busy = false;
  }
</script>

{#if target}
  <button class="btn sm retry" disabled={busy} onclick={retry} data-tip="Back to Ready: its agent resumes it">Retry</button>
{/if}

<style>
  .retry { flex-shrink: 0; }
</style>
