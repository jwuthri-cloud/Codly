<!--
  The warning Full access shows before it can be turned on (Settings): what it lets
  through, what still asks, and an explicit confirm.
-->
<script lang="ts" module>
  /** What Full access lets through, and what still asks: shown before it can be turned on. */
  export const FULL_ACCESS = {
    allows: [
      'Any command or edit a worker asks about, including risky ones: rm -rf, git push, network installs, files outside its worktree.',
      'Approved once per request, automatically: nothing waits for you.',
    ],
    stillAsks: [
      'Questions an agent asks you',
      'Changes to an agent’s permissions',
      'Anything touching Codly’s private data',
      'Read-only agents: the Lead and reviewers',
    ],
  };
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../components/Icon.svelte';

  interface Props {
    onconfirm: () => void;
    oncancel: () => void;
  }
  let { onconfirm, oncancel }: Props = $props();
  let cancelBtn: HTMLButtonElement | undefined = $state();
  onMount(() => cancelBtn?.focus());
</script>

<div class="warn" role="alertdialog" aria-labelledby="fa-title" aria-describedby="fa-body">
  <p class="w-title" id="fa-title"><Icon name="alert" size={15} />Turn on Full access?</p>
  <div class="w-body" id="fa-body">
    <p class="eyebrow">Allowed without asking</p>
    <ul>{#each FULL_ACCESS.allows as a (a)}<li>{a}</li>{/each}</ul>
    <p class="eyebrow">Still asks you</p>
    <ul>{#each FULL_ACCESS.stillAsks as a (a)}<li>{a}</li>{/each}</ul>
    <p class="fine">Only use it on repositories you can restore. Nothing merges into your branch without you.</p>
  </div>
  <div class="w-actions">
    <button class="btn ghost sm" bind:this={cancelBtn} onclick={oncancel}>Keep my current mode</button>
    <button class="btn amber sm" onclick={onconfirm}>Yes, turn on Full access</button>
  </div>
</div>

<style>
  .warn { display: flex; flex-direction: column; gap: 10px; padding: 14px 16px; border-radius: var(--r-md); background: var(--amber-soft); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--amber) 40%, transparent); animation: rise-in var(--d-3) var(--spring) both; }
  .w-title { display: inline-flex; align-items: center; gap: 8px; font-weight: var(--w-semi); color: var(--amber-ink); }
  .w-body { display: flex; flex-direction: column; gap: 6px; font-size: var(--t-sm); line-height: var(--lh-sm); color: var(--ink-2); }
  .w-body ul { display: flex; flex-direction: column; gap: 3px; margin-bottom: 4px; }
  .w-body li { padding-left: 14px; position: relative; }
  .w-body li::before { content: '·'; position: absolute; left: 4px; color: var(--ink-3); }
  .fine { color: var(--ink-3); }
  .w-actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
