<!--
  The toast stack: top right, under the top bar, so it never sits on the composer's Send or a
  sheet's footer. While the agent or task sheet is open the stack moves left of it, off the
  sheet's own controls. Dialogs are drawn above it.
  The corner is not empty: while a toast shows, it lies on the top right of the stage (the
  last buttons of a conversation's header; a room's header buttons, which start about 70px
  under the top bar, when the stack is taller than that).
-->
<script lang="ts">
  import { fade } from 'svelte/transition';
  import { toasts } from '../lib/state/toasts.svelte';
  import { ui } from '../lib/state/ui.svelte';
  import Icon from './Icon.svelte';
  import Kbd from './Kbd.svelte';

  const ICON: Record<string, string> = { success: 'check-circle', error: 'x-circle', info: 'info', progress: 'refresh', attention: 'flag' };
</script>

<div class="stack" class:beside-sheet={ui.sheetOpen} aria-live="polite" aria-relevant="additions text">
  {#each toasts.list as t (t.id)}
    <div
      class="toast k-{t.kind}"
      role={t.kind === 'error' ? 'alert' : 'status'}
      onpointerenter={() => toasts.pause(t.id)}
      onpointerleave={() => toasts.resume(t.id)}
      out:fade={{ duration: ui.reducedMotion ? 0 : 120 }}
    >
      <span class="ic" class:spin={t.kind === 'progress'}><Icon name={ICON[t.kind]} size={16} /></span>
      <div class="txt">
        <p class="title">{t.title}</p>
        {#if t.body}<p class="body selectable">{t.body}</p>{/if}
      </div>
      {#if t.action}
        <button class="btn sm" onclick={() => { t.action!.run(); toasts.dismiss(t.id); }}>
          {t.action.label}
          {#if t.action.kbd}<Kbd keys={t.action.kbd} subtle />{/if}
        </button>
      {/if}
      <button class="icon-btn sm close" aria-label="Dismiss" onclick={() => toasts.dismiss(t.id)}><Icon name="x" size={14} /></button>
    </div>
  {/each}
</div>

<style>
  /* column-reverse: the newest toast is the one next to the top bar */
  .stack { position: fixed; right: var(--gutter); top: calc(var(--top-h) + 4px); z-index: var(--z-toast); display: flex; flex-direction: column-reverse; gap: 8px; width: min(360px, calc(100vw - 2 * var(--gutter))); pointer-events: none; transition: right var(--d-4) var(--spring); }
  .stack.beside-sheet { right: calc(var(--sheet-w) + var(--gutter) * 2); }
  .toast { pointer-events: auto; display: flex; align-items: flex-start; gap: 10px; padding: 12px 10px 12px 14px; border-radius: var(--r-lg); background: var(--pop); box-shadow: var(--pop-shadow); animation: pop-in var(--d-3) var(--spring); }
  .ic { margin-top: 2px; color: var(--ink-2); }
  .k-success .ic { color: var(--ok); }
  .k-error .ic { color: var(--bad); }
  .k-attention .ic { color: var(--amber); }
  .k-progress .ic { color: var(--live); }
  .spin :global(svg) { animation: spin 1.1s linear infinite; }
  .txt { flex: 1; min-width: 0; }
  .title { font-weight: var(--w-med); color: var(--ink-1); }
  .body { margin-top: 2px; color: var(--ink-2); font-size: var(--t-sm); line-height: var(--lh-sm); overflow-wrap: anywhere; }
  .btn { margin-top: -2px; }
  .close { margin: -3px -2px 0 -2px; color: var(--ink-3); }
</style>
