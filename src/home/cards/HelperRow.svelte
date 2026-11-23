<!--
  A helper (a subagent the agent started with one tool call), as one step of a work row: its
  kind and what it was asked, how far it is, its own calls, and what it reported.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Seg, SubAgent } from '../../lib/state/store.svelte';
  import { ui } from '../../lib/state/ui.svelte';
  import { dur } from '../../lib/util/format';
  import { uniqueKeys } from '../../lib/util/keys';
  import { clean } from '../../lib/util/sanitize';
  import Icon from '../../components/Icon.svelte';
  import { helperProgress, helperState, helperTitle } from '../helpers';

  interface Props {
    helper: SubAgent;
    /** The agent's call that started it. */
    call: Seg;
    /** The helper's own calls. */
    steps: Seg[];
    /** How the work row shows one call. */
    step: Snippet<[Seg]>;
  }
  let { helper, call, steps, step }: Props = $props();
  const phase = $derived(helperState(helper, call));
  const progress = $derived(helperProgress(helper));
  const now = $derived(phase === 'running' ? clean(helper.step).trim() : '');
  const report = $derived(clean(helper.report).trim());
  /** One key per call, its call id. A CLI may use an id again: the later call adds its place. */
  const stepKeys = $derived(uniqueKeys(steps, (s) => s.callId));
</script>

<div class="helper s-{phase}">
  <button class="head" onclick={() => ui.selectAgent(call.agentId, call.eventId)}>
    <span class="ic"><Icon name="crew" size={13} /></span>
    <span class="title">{helperTitle(helper, phase)}</span>
    {#if helper.name}<span class="kind">{clean(helper.name)}</span>{/if}
    <span class="what truncate">{clean(helper.description)}</span>
    {#if progress}<span class="meta num">{progress}</span>{/if}
    <span class="dur num">{phase === 'running' ? '…' : dur(helper.ts - helper.startedAt)}</span>
  </button>
  {#if now || steps.length || report}
    <div class="inside">
      {#if now}<p class="now truncate">{now}</p>{/if}
      {#if steps.length}
        <ol class="own">
          {#each steps as s, si (stepKeys[si])}
            <li>{@render step(s)}</li>
          {/each}
        </ol>
      {/if}
      {#if report}<p class="report selectable">{report}</p>{/if}
    </div>
  {/if}
</div>

<style>
  .head { display: flex; align-items: center; gap: 9px; width: 100%; height: 26px; padding: 0 8px; border-radius: var(--r-sm); font-size: var(--t-sm); text-align: left; }
  .head:hover { background: var(--surface-1); }
  .ic { display: grid; place-items: center; color: var(--ink-3); }
  .s-running .ic { color: var(--live); }
  .s-failed .ic { color: var(--bad); }
  .title { color: var(--ink-2); white-space: nowrap; }
  .kind { padding: 1px 7px; border-radius: var(--r-full); box-shadow: inset 0 0 0 1px var(--line-2); font-size: var(--t-xs); color: var(--ink-3); white-space: nowrap; }
  .what { flex: 1; color: var(--ink-1); }
  .meta, .dur { color: var(--ink-3); white-space: nowrap; }
  .inside { margin: 1px 0 4px 14px; padding-left: 12px; box-shadow: inset 1px 0 0 var(--line-2); display: flex; flex-direction: column; gap: 1px; }
  .now { padding: 2px 8px; font-size: var(--t-sm); color: var(--ink-2); }
  .own { display: flex; flex-direction: column; gap: 1px; }
  .report { padding: 4px 8px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); white-space: pre-wrap; overflow-wrap: anywhere; max-width: 60ch; display: -webkit-box; -webkit-line-clamp: 6; line-clamp: 6; -webkit-box-orient: vertical; overflow: hidden; }
</style>
