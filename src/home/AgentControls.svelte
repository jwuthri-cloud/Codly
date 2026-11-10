<!--
  A 1:1 chat's own picker under the composer: how much the agent asks you. It works in its own
  worktree, so Autonomous lets it change anything there without asking. Unset follows Settings.
-->
<script lang="ts">
  import type { ApprovalMode } from '../lib/gen/ApprovalMode';
  import { setRequestPrefs } from '../lib/state/actions';
  import { store } from '../lib/state/store.svelte';
  import Icon from '../components/Icon.svelte';
  import Menu, { type MenuItem } from '../components/Menu.svelte';
  import { PERMISSIONS } from './LeadControls.svelte';

  interface Props {
    thread: string;
    /** A plan worker's chat: the picker is its request's, shared by every agent on that plan. */
    request?: boolean;
  }
  let { thread, request = false }: Props = $props();

  const own = $derived(store.requestPrefs[thread]?.approval_mode ?? null);
  const perm = $derived<ApprovalMode>(own ?? store.settings.approval_mode);
  const label = $derived(PERMISSIONS.find((p) => p.id === perm)?.label ?? perm);

  let open = $state(false);
  let btn: HTMLButtonElement | undefined = $state();
  const items = $derived<MenuItem[]>([
    { kind: 'header', label: 'Permissions' },
    ...PERMISSIONS.map((p) => ({ label: p.label, hint: p.hint, checked: p.id === perm, run: () => setRequestPrefs(thread, { approval_mode: p.id }) })),
  ]);
</script>

<div class="controls">
  <button class="chip" bind:this={btn} aria-haspopup="menu" aria-expanded={open} onclick={() => (open = !open)} data-tip={request ? 'How much every agent on this request asks you before it acts' : 'How much this agent asks you before it acts'}>
    <Icon name="lock" size={13} />{label}{#if request}<span class="sub">· whole request</span>{:else if !own}<span class="sub">· Settings</span>{/if}<Icon name="chevron-down" size={12} />
  </button>
</div>
{#if open && btn}<Menu anchor={btn} items={items} onclose={() => (open = false)} width={300} />{/if}

<style>
  .controls { display: flex; gap: 4px; padding: 6px 10px 0; }
  .chip { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 8px 0 9px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .chip:hover, .chip[aria-expanded='true'] { background: var(--surface-2); color: var(--ink-1); }
  .sub { color: var(--ink-3); font-weight: var(--w-reg); }
</style>
