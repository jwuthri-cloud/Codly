// The picker under the composer of a 1:1 chat: what its chip says and what its menu offers.
// Pure functions, so they can be tested; AgentControls.svelte draws them.
import type { Agent } from '../lib/gen/Agent';
import type { ApprovalMode } from '../lib/gen/ApprovalMode';
import type { PermissionMode } from '../lib/gen/PermissionMode';
import { PERMISSION_MODES } from '../lib/state/control.svelte';

/** One of the permissions (`PERMISSIONS` of LeadControls). */
type Permission = { id: ApprovalMode; label: string; hint: string };

/**
 * How the agent of this chat works with edits, when the picker can switch it: a 1:1 Claude
 * agent in its own chat (one that never chose asks for edits). Null: nothing to switch here.
 * That is another CLI, a plan's worker, and a 1:1 agent on a task: its chat shows the
 * permissions of the task's request (`request`), and its mode can change when the task is done.
 */
export function wayOf(agent: Pick<Agent, 'role' | 'provider' | 'permission_mode'> | null, request: boolean): PermissionMode | null {
  return !request && agent?.role === 'solo' && agent.provider === 'claude' ? (agent.permission_mode ?? 'ask') : null;
}

/** A way of working as the agent's sheet names and explains it (`PERMISSION_MODES` lists every one). */
const wayWords = (way: PermissionMode) => PERMISSION_MODES.find((m) => m.id === way)!;

/** The chip: its words, the lighter words after them, and its tip. */
export type Chip = { text: string; sub: string | null; tip: string };

/**
 * What the chip says. It follows the agent: "Plan first · then Balanced" (until you accept its
 * plan only reads are approved for it, whatever the permissions), "Accept edits · Balanced"
 * (edits do not ask; the permissions are for the rest), else the permissions alone, as for
 * every other agent. `permission`: the label of the permissions in force; `own`: this chat
 * chose them (else they are the ones of Settings).
 */
export function chipOf(permission: string, at: { own: boolean; request: boolean; way: PermissionMode | null }): Chip {
  if (at.way === 'plan') {
    return {
      text: wayWords('plan').label,
      sub: `· then ${permission}`,
      tip: 'It reads and plans first: until you accept its plan, only reads are approved for it. After that, this is how much it asks you before it acts',
    };
  }
  if (at.way === 'accept_edits') return { text: wayWords('accept_edits').label, sub: `· ${permission}`, tip: wayWords('accept_edits').hint };
  if (at.request) return { text: permission, sub: '· whole request', tip: 'How much every agent on this request asks you before it acts' };
  return { text: permission, sub: at.own ? null : '· Settings', tip: 'How much this agent asks you before it acts' };
}

/** What a way of working means, in the few words a row of the menu has room for. The agent's sheet says the rest. */
const WAY_HINT: Record<PermissionMode, string> = {
  ask: 'Per your permissions',
  accept_edits: 'Edits do not ask you',
  plan: 'You accept its plan',
};

type Entry<Id> = { id: Id; label: string; hint: string; checked: boolean };
/** One group of the chip's menu: its header and its entries, the one in force checked. */
export type PickerGroup = { kind: 'permissions'; header: string; entries: Entry<ApprovalMode>[] } | { kind: 'way'; header: string; entries: Entry<PermissionMode>[] };

/**
 * The chip's menu: the permissions, and for an agent with a way of working to switch (`wayOf`)
 * a second group, "How it works", with the agent's own way checked. Other agents see the
 * permissions alone.
 */
export function groupsOf(permissions: Permission[], permission: ApprovalMode, way: PermissionMode | null): PickerGroup[] {
  const first: PickerGroup = {
    kind: 'permissions',
    header: way === 'plan' ? 'Permissions, once you accept its plan' : 'Permissions',
    entries: permissions.map((p) => ({ ...p, checked: p.id === permission })),
  };
  if (!way) return [first];
  return [first, { kind: 'way', header: 'How it works', entries: PERMISSION_MODES.map((m) => ({ id: m.id, label: m.label, hint: WAY_HINT[m.id], checked: m.id === way })) }];
}
