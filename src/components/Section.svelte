<!-- Collapsible inspector section. Open state is remembered per section id (not per agent). -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { ui } from '../lib/state/ui.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    id: string;
    title: string;
    summary?: string;
    tone?: 'default' | 'attention';
    children: Snippet;
    aside?: Snippet;
    collapsible?: boolean;
  }
  let { id, title, summary, tone = 'default', children, aside, collapsible = true }: Props = $props();
  const open = $derived(!collapsible || (ui.sections[id] ?? true));
</script>

<section class="sec" class:attention={tone === 'attention'} data-section={id}>
  <header>
    {#if collapsible}
      <button class="toggle" aria-expanded={open} onclick={() => ui.toggleSection(id)}>
        <span class="chev" class:open><Icon name="chevron-right" size={12} stroke={2} /></span>
        <span class="eyebrow">{title}</span>
        {#if summary && !open && !aside}<span class="summary truncate">{summary}</span>{/if}
      </button>
    {:else}
      <span class="toggle static"><span class="eyebrow">{title}</span></span>
    {/if}
    {#if aside}<div class="aside">{@render aside()}</div>{/if}
  </header>
  {#if open}
    <div class="content">{@render children()}</div>
  {/if}
</section>

<style>
  .sec { padding: 14px 22px 16px; }
  .sec + :global(.sec) { box-shadow: inset 0 1px 0 var(--line); }
  .sec.attention { background: linear-gradient(180deg, var(--amber-soft), transparent 80%); }
  header { display: flex; align-items: center; gap: 8px; min-height: 22px; }
  .toggle { display: flex; align-items: center; gap: 6px; flex: 1; min-width: 0; height: 22px; text-align: left; }
  .toggle.static { cursor: default; }
  .toggle:hover :global(.eyebrow) { color: var(--ink-1); }
  .chev { display: grid; place-items: center; color: var(--ink-3); transition: transform var(--d-2) var(--ease-out); }
  .chev.open { transform: rotate(90deg); }
  .summary { color: var(--ink-3); font-size: var(--t-sm); margin-left: 4px; }
  .aside { display: flex; align-items: center; gap: 4px; }
  .content { padding-top: 10px; animation: fade-in var(--d-2) var(--ease-out); }
</style>
