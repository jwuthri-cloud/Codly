// The buttons under a request that waits for you, in the order they show. Only the choices the
// agent's CLI offered become buttons.
// A plan (a Claude agent in plan mode) has its own set: accept it, change it and accept, send
// it back with a note, or stop the turn. It has no plain "no": without a note Claude Code shows
// the same plan again, so the one way to say no opens the note field.
// A file edit of a 1:1 Claude agent that asks for edits can also be approved together with a
// switch to "Accept edits" (`acceptsEdits`), so that its next edits stop asking.
import type { Agent } from '../gen/Agent';
import type { ApprovalChoice } from '../gen/ApprovalChoice';
import type { ApprovalMode } from '../gen/ApprovalMode';
import type { ApprovalSubject } from '../gen/ApprovalSubject';
import type { Provider } from '../gen/Provider';
import { pageOf, SAFETY_PAGES, type SafetySection } from '../../overlays/settingsPages';
import { PROVIDER_LABEL } from './labels';
import { PERMISSION_SCOPE } from './permissionsAsked';

export type FooterButton = {
  /** `choice`: answer with `choice`. `edit`: open the plan to change it. `note`: open the note
   *  field. `accept_edits`: approve once, then switch the agent to "Accept edits". */
  act: 'choice' | 'edit' | 'note' | 'accept_edits';
  /** The answer a `choice` button sends. */
  choice?: ApprovalChoice;
  label: string;
  /** The key that does the same, shown on the card the keyboard acts on. */
  kbd?: string;
  /** The button's look: 'primary', 'ghost', 'danger', or '' for a plain one. */
  cls: string;
  /** What it does beyond its label, for its tooltip. */
  hint?: string;
};

const CHOICE: Record<ApprovalChoice, { label: string; kbd?: string; cls: string }> = {
  allow_once: { label: 'Approve once', kbd: '⌘↵', cls: 'primary' },
  allow_session: { label: 'Allow for this session', kbd: '⌥⌘↵', cls: '' },
  allow_always: { label: 'Always allow', cls: '' },
  deny: { label: 'Deny', kbd: '⌘⌫', cls: 'ghost' },
  cancel: { label: 'Stop the turn', cls: 'danger' },
};
const ORDER: ApprovalChoice[] = ['allow_once', 'allow_session', 'allow_always', 'deny', 'cancel'];

const answer = (choice: ApprovalChoice, label = CHOICE[choice].label): FooterButton => ({ act: 'choice', choice, label, kbd: CHOICE[choice].kbd, cls: CHOICE[choice].cls });

/** A Codex permissions request is allowed for its turn or for the session: the scope Codly answers with. */
const SCOPED: Partial<Record<ApprovalChoice, string>> = { allow_once: PERMISSION_SCOPE.allow_once, allow_session: PERMISSION_SCOPE.allow_session };

/** Who asks: the CLI of the agent and its name. */
export type Asker = { provider: Provider; name: string };

/** `plan`: the card shows a Claude agent's plan. `offered`: the choices its CLI offered.
 *  `permissions`: the card is a permissions request. `asker`: who asks (null: an agent Codly
 *  does not know). `acceptEdits`: the hint of "Accept edits" when the card offers to switch to
 *  it (`acceptsEdits`), else null. */
export function footerButtons(plan: boolean, offered: readonly ApprovalChoice[], permissions = false, asker: Asker | null = null, acceptEdits: string | null = null): FooterButton[] {
  const has = (c: ApprovalChoice) => offered.includes(c);
  if (plan) {
    return [
      ...(has('allow_once') ? [answer('allow_once', 'Accept the plan'), { act: 'edit', label: 'Change the plan…', cls: '' } satisfies FooterButton] : []),
      ...(has('deny') ? [{ act: 'note', label: 'Send it back with a note…', kbd: '⌘⌫', cls: 'ghost' } satisfies FooterButton] : []),
      ...(has('cancel') ? [answer('cancel')] : []),
    ];
  }
  const edits: FooterButton[] = acceptEdits !== null ? [{ act: 'accept_edits', label: ACCEPT_EDITS_LABEL, cls: '', hint: `Approve this edit, and accept edits from now on. ${acceptEdits}` }] : [];
  const choices = ORDER.filter(has).flatMap((c) => [answer(c, (permissions && SCOPED[c]) || undefined), ...(c === 'allow_once' ? edits : [])]);
  return [...choices, ...(has('deny') ? [noteButton(asker)] : [])];
}

/** The labels are short, so that the five buttons of a file edit fit one row: the whole sentence is in the tooltip. */
const ACCEPT_EDITS_LABEL = 'Approve and accept edits';

/** Claude Code takes a note with its "no". Every other CLI takes none: Codly sends your note to
 *  the agent as your next message, and the button's tooltip and the field say so. */
const otherCli = (asker: Asker | null): asker is Asker => !!asker && asker.provider !== 'claude';
const noteButton = (asker: Asker | null): FooterButton =>
  otherCli(asker)
    ? { act: 'note', label: 'Deny and say why…', kbd: '⇧⌘⌫', cls: 'ghost', hint: `Deny, and tell ${asker.name} why: ${PROVIDER_LABEL[asker.provider]} takes no note with a deny, so Codly sends yours as your next message.` }
    : { act: 'note', label: 'Deny with a note…', kbd: '⇧⌘⌫', cls: 'ghost' };

/** The line under the note field of an agent whose CLI takes no note with a "no"; null for Claude Code and for an agent Codly does not know. */
export function noteGoes(asker: Asker | null): string | null {
  return otherCli(asker) ? `${PROVIDER_LABEL[asker.provider]} takes no note with a deny. Codly sends yours to ${asker.name} as your next message.` : null;
}

/**
 * Whether a request's card offers "Approve and accept edits" (from now on): a file change of a
 * 1:1 Claude agent in "Ask for edits" that is on no task, and that can be approved once. Only
 * such an agent has a mode to switch (its mode waits while it is on a task). Never for a plan's
 * worker, a Lead, another CLI, or an agent already in another mode. `permission`: the
 * permissions of its conversation; under "Ask first" every change asks you and the core refuses
 * "Accept edits", so the card does not offer it.
 */
export function acceptsEdits(subject: ApprovalSubject, offered: readonly ApprovalChoice[], agent: Pick<Agent, 'provider' | 'role' | 'task_id' | 'permission_mode'> | null, permission: ApprovalMode): boolean {
  return (
    subject.kind === 'file_change' &&
    offered.includes('allow_once') &&
    permission !== 'strict' &&
    !!agent &&
    agent.provider === 'claude' &&
    agent.role === 'solo' &&
    !agent.task_id &&
    (agent.permission_mode ?? 'ask') === 'ask'
  );
}

/** Where Settings shows a section of its Safety tab, by the name of its page: "Settings → Safety → Approvals".
 *  `safetyLink` of the same section opens it there. */
export function safetyPlace(section: SafetySection): string {
  const page = pageOf(section);
  return `Settings → Safety → ${SAFETY_PAGES.find((p) => p.id === page)!.label}`;
}

/** The note that goes back with a plan must say something: without one the same plan comes back. */
export const PLAN_NOTE_HINT = 'Say what to change: without a note the same plan comes back.';
