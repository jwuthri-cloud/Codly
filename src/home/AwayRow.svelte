<!--
  What happened while you were away, as one quiet row in the thread, before the first row that
  came after you left: "While you were away (47 min): Bo waits for your answer · Ada’s turn
  failed: … · Ada finished t1 and t2 · 14 more rows". Each fact takes you to its row (the oldest
  one of a group) and marks it for a moment. × puts the row away; the next time you are away a
  new one takes its place (see away.ts for the rule).
-->
<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { awayFor, type Away } from './away';

  interface Props {
    summary: Away;
    onjump: (key: string) => void;
    ondismiss: () => void;
  }
  let { summary, onjump, ondismiss }: Props = $props();
</script>

<div class="away" role="note" aria-label="While you were away">
  <p>
    <span class="lead">While you were away <span class="num">({awayFor(summary.minutes)})</span>:</span>
    {#each summary.facts as f, i (f.key + f.text)}{#if i}<span class="sep" aria-hidden="true">·</span>{/if}<button class="fact" onclick={() => onjump(f.key)}>{f.text}</button>{/each}
  </p>
  <button class="x" aria-label="Put away the summary" data-tip="Put it away" onclick={ondismiss}><Icon name="x" size={12} stroke={2} /></button>
</div>

<style>
  .away { display: flex; align-items: flex-start; gap: 8px; padding: 8px 8px 8px 14px; border-radius: var(--r-md); background: var(--surface-1); box-shadow: inset 0 0 0 1px var(--line); }
  p { flex: 1; min-width: 0; font-size: var(--t-sm); line-height: 22px; color: var(--ink-3); }
  .lead { color: var(--ink-2); font-weight: var(--w-med); }
  .fact { display: inline; padding: 0 2px; margin: 0 -2px; border-radius: 4px; font-size: inherit; line-height: inherit; color: var(--ink-2); text-decoration: underline; text-decoration-color: var(--line-3); text-underline-offset: 3px; }
  .fact:hover { color: var(--ink-1); text-decoration-color: currentColor; }
  .sep { margin: 0 6px; color: var(--ink-4); }
  .x { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 50%; flex-shrink: 0; color: var(--ink-3); }
  .x:hover { background: var(--surface-3); color: var(--ink-1); }
</style>
