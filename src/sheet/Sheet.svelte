<!--
  The sheet: slides in from the right only when something is selected (agent, task or file),
  and slides away on Esc or ×. The stage makes room for it; nothing is covered.
-->
<script lang="ts">
  import { cubicIn } from 'svelte/easing';
  import { fly } from 'svelte/transition';
  import { store } from '../lib/state/store.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import Icon from '../components/Icon.svelte';
  import AgentSheet from './AgentSheet.svelte';
  import FileSheet from './FileSheet.svelte';
  import TaskSheet from './TaskSheet.svelte';

  const sel = $derived(ui.selection);
  const agent = $derived(sel?.kind === 'agent' ? (store.agentMap.get(sel.id) ?? null) : null);
  const task = $derived(sel?.kind === 'task' ? (store.taskMap.get(sel.id) ?? null) : null);
  const label = $derived(agent ? `${agent.name} details` : task ? `Task ${task.key}` : sel?.kind === 'file' ? 'File' : '');
</script>

<!-- The exit runs through the Web Animations API, which the CSS reduced-motion override does not reach. -->
{#if sel}
  <aside class="sheet glass" aria-label={label} out:fly={{ x: 16, duration: ui.reducedMotion ? 0 : 180, easing: cubicIn }}>
    <button class="icon-btn close" aria-label="Close" data-tip="Close" data-tip-kbd="Esc" onclick={() => ui.select(null)}><Icon name="x" /></button>
    {#key sel.kind + sel.id}
      <div class="body">
        {#if agent}<AgentSheet {agent} />
        {:else if task}<TaskSheet {task} />
        {:else if sel.kind === 'file'}<FileSheet path={sel.id} />
        {:else}<p class="gone">This is no longer here.</p>{/if}
      </div>
    {/key}
  </aside>
{/if}

<style>
  .sheet { position: fixed; z-index: var(--z-sheet); top: var(--gutter); right: var(--gutter); bottom: var(--gutter); width: var(--sheet-w); border-radius: var(--r-xl); background: var(--glass-strong); display: flex; flex-direction: column; overflow: hidden; animation: sheet-in var(--d-4) var(--spring); }
  @keyframes sheet-in { from { transform: translateX(calc(100% + 24px)); } to { transform: none; } }
  .close { position: absolute; top: 14px; right: 14px; z-index: 2; }
  .body { flex: 1; min-height: 0; display: flex; flex-direction: column; animation: fade-in var(--d-3) var(--ease-out); }
  .gone { padding: 60px 24px; color: var(--ink-3); text-align: center; }
</style>
