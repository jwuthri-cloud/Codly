<!--
  A 1:1 chat's own picker under the composer: how much the agent asks you. It works in its own
  worktree, so Autonomous lets it change anything there without asking. Unset follows Settings.
  A 1:1 Claude agent in the mode "plan first" says so first: until you accept its plan, only
  reads are approved for it, and the permissions apply after that.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import type { ApprovalMode } from '../lib/gen/ApprovalMode';
  import { setRequestPrefs } from '../lib/state/actions';
  import { PERMISSION_MODES, cyclesMode } from '../lib/state/control.svelte';
  import { store } from '../lib/state/store.svelte';
  import Icon from '../components/Icon.svelte';
  import Menu, { type MenuItem } from '../components/Menu.svelte';
  import { PERMISSIONS } from './LeadControls.svelte';
  import UsageChip from './UsageChip.svelte';

  interface Props {
    thread: string;
    /** A plan worker's chat: the picker is its request's, shared by every agent on that plan. */
    request?: boolean;
    /** The agent you talk to here: its context meter and cost show beside the picker. */
    agent: Agent | null;
  }
  let { thread, request = false, agent }: Props = $props();

  const own = $derived(store.requestPrefs[thread]?.approval_mode ?? null);
  const perm = $derived<ApprovalMode>(own ?? store.settings.approval_mode);
  const label = $derived(PERMISSIONS.find((p) => p.id === perm)?.label ?? perm);
  /** A 1:1 Claude agent in the mode "plan first": only reads are approved for it until you accept its plan, whatever the permissions. */
  const planFirst = $derived(!request && agent?.role === 'solo' && agent.provider === 'claude' && agent.permission_mode === 'plan');
  const planLabel = PERMISSION_MODES.find((m) => m.id === 'plan')!.label;
  /** A 1:1 Claude agent in the mode "accept edits": its edits run without asking; the permissions are for the rest. */
  const acceptEdits = $derived(!request && agent?.role === 'solo' && agent.provider === 'claude' && agent.permission_mode === 'accept_edits');
  const acceptLabel = PERMISSION_MODES.find((m) => m.id === 'accept_edits')!.label;
  /** ⇧⇥ in the composer switches the mode shown here. */
  const cycles = $derived(!request && !!agent && cyclesMode(agent));

  let open = $state(false);
  let btn: HTMLButtonElement | undefined = $state();
  const items = $derived<MenuItem[]>([
    { kind: 'header', label: planFirst ? 'Permissions, once you accept its plan' : 'Permissions' },
    ...PERMISSIONS.map((p) => ({ label: p.label, hint: p.hint, checked: p.id === perm, run: () => setRequestPrefs(thread, { approval_mode: p.id }) })),
  ]);
</script>

<div class="controls">
  <button class="chip" bind:this={btn} aria-haspopup="menu" aria-expanded={open} onclick={() => (open = !open)} data-tip={request ? 'How much every agent on this request asks you before it acts' : planFirst ? 'It reads and plans first: until you accept its plan, only reads are approved for it. After that, this is how much it asks you before it acts' : acceptEdits ? 'Its edits run without asking you. This is how much it asks you before anything else' : 'How much this agent asks you before it acts'} data-tip-kbd={cycles ? '⇧⇥' : undefined}>
    <Icon name="lock" size={13} />{#if planFirst}{planLabel}<span class="sub">· then {label}</span>{:else if acceptEdits}{acceptLabel}<span class="sub">· {label}</span>{:else}{label}{#if request}<span class="sub">· whole request</span>{:else if !own}<span class="sub">· Settings</span>{/if}{/if}<Icon name="chevron-down" size={12} />
  </button>
  <UsageChip {agent} />
</div>
{#if open && btn}<Menu anchor={btn} items={items} onclose={() => (open = false)} width={300} />{/if}

<style>
  /* In a narrow panel the context and cost chip goes to a line of its own; it is never cut. */
  .controls { display: flex; flex-wrap: wrap; gap: 4px; padding: 6px 10px 0; }
  .chip { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 8px 0 9px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .chip:hover, .chip[aria-expanded='true'] { background: var(--surface-2); color: var(--ink-1); }
  .sub { color: var(--ink-3); font-weight: var(--w-reg); }
</style>
