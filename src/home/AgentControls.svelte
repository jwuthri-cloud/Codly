<!--
  A 1:1 chat's own picker under the composer: how much the agent asks you. It works in its own
  worktree, so Autonomous lets it change anything there without asking. Unset follows Settings.
  For a 1:1 Claude agent the menu has a second group, "How it works": ask for edits, accept
  edits, plan first (the agent's sheet has the same switch). The chip says the way it is in:
  in "plan first", until you accept its plan, only reads are approved for it, and the
  permissions apply after that. ⇧⇥ in the composer switches to the next way (agentPicker.ts
  `nextWay`, the same call). A switch the core refuses changes nothing here, and a toast
  says why. What the chip says and what the menu lists: agentPicker.ts.
-->
<script lang="ts">
  import type { Agent } from '../lib/gen/Agent';
  import type { ApprovalMode } from '../lib/gen/ApprovalMode';
  import { setRequestPrefs } from '../lib/state/actions';
  import { setPermissionMode } from '../lib/state/control.svelte';
  import { store } from '../lib/state/store.svelte';
  import Icon from '../components/Icon.svelte';
  import Menu, { type MenuItem } from '../components/Menu.svelte';
  import { chipOf, groupsOf, wayOf } from './agentPicker';
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
  /** How a 1:1 Claude agent works with edits; null for an agent with no such switch here. */
  const way = $derived(wayOf(agent, request));
  const chip = $derived(chipOf(label, { own: own != null, request, way }));

  let open = $state(false);
  let btn: HTMLButtonElement | undefined = $state();
  const items = $derived<MenuItem[]>(
    groupsOf(PERMISSIONS, perm, way).flatMap((g): MenuItem[] => [
      { kind: 'header', label: g.header },
      ...(g.kind === 'permissions'
        ? g.entries.map((e) => ({ label: e.label, hint: e.hint, checked: e.checked, run: () => void setRequestPrefs(thread, { approval_mode: e.id }) }))
        : g.entries.map((e) => ({ label: e.label, hint: e.hint, checked: e.checked, run: () => void (agent && setPermissionMode(agent.id, e.id)) }))),
    ]),
  );
</script>

<div class="controls">
  <button class="chip" bind:this={btn} aria-haspopup="menu" aria-expanded={open} onclick={() => (open = !open)} data-tip={chip.tip} data-tip-kbd={way ? '⇧⇥' : undefined}>
    <Icon name="lock" size={13} />{chip.text}{#if chip.sub}<span class="sub">{chip.sub}</span>{/if}<Icon name="chevron-down" size={12} />
  </button>
  <UsageChip {agent} />
</div>
{#if open && btn}<Menu anchor={btn} items={items} onclose={() => (open = false)} width={300} wrap />{/if}

<style>
  /* In a narrow panel the context and cost chip goes to a line of its own; it is never cut. */
  .controls { display: flex; flex-wrap: wrap; gap: 4px; padding: 6px 10px 0; }
  .chip { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 8px 0 9px; border-radius: var(--r-full); font-size: var(--t-sm); font-weight: var(--w-med); color: var(--ink-2); }
  .chip:hover, .chip[aria-expanded='true'] { background: var(--surface-2); color: var(--ink-1); }
  .sub { color: var(--ink-3); font-weight: var(--w-reg); }
</style>
