<!-- Provenance badge: every fact says where it came from. -->
<script lang="ts">
  import { PROVENANCE_LABEL, PROVENANCE_TIP, type Provenance } from '../lib/util/labels';

  interface Props {
    kind: Provenance;
    compact?: boolean;
  }
  let { kind, compact = false }: Props = $props();
  const GLYPH: Record<Provenance, string> = {
    observed: '<circle cx="5" cy="5" r="2.4" fill="currentColor"/>',
    inferred: '<path d="M1.6 5.6c1.1-2 2.3-2 3.4 0s2.3 2 3.4 0" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
    provided: '<path d="M2 8V2h6" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/><circle cx="6.5" cy="6.5" r="1.6" fill="currentColor"/>',
    model: '<path d="M2 2.5h6v4H4.5L2 8.2z" fill="currentColor"/>',
  };
</script>

<span class="pv pv-{kind}" class:compact data-tip={PROVENANCE_TIP[kind]}>
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">{@html GLYPH[kind]}</svg>
  {#if !compact}<span class="label">{PROVENANCE_LABEL[kind]}</span>{:else}<span class="sr-only">{PROVENANCE_LABEL[kind]}</span>{/if}
</span>

<style>
  .pv { --c: var(--pv-observed); display: inline-flex; align-items: center; gap: 4px; height: 18px; padding: 0 7px 0 5px; border-radius: var(--r-full); font-size: var(--t-xs); line-height: 1; font-weight: var(--w-med); color: var(--ink-2); background: color-mix(in srgb, var(--c) 12%, transparent); white-space: nowrap; vertical-align: middle; flex-shrink: 0; }
  .pv.compact { padding: 0; width: 18px; justify-content: center; }
  .pv svg { color: var(--c); }
  .pv-observed { --c: var(--pv-observed); }
  .pv-inferred { --c: var(--pv-inferred); }
  .pv-provided { --c: var(--pv-provided); }
  .pv-model { --c: var(--pv-model); }
</style>
