// The buttons under a request that waits for you, in the order they show. Only the choices the
// agent's CLI offered become buttons.
// A plan (a Claude agent in plan mode) has its own set: accept it, change it and accept, send
// it back with a note, or stop the turn. It has no plain "no": without a note Claude Code shows
// the same plan again, so the one way to say no opens the note field.
import type { ApprovalChoice } from '../gen/ApprovalChoice';

export type FooterButton = {
  /** `choice`: answer with `choice`. `edit`: open the plan to change it. `note`: open the note field. */
  act: 'choice' | 'edit' | 'note';
  /** The answer a `choice` button sends. */
  choice?: ApprovalChoice;
  label: string;
  /** The key that does the same, shown on the card the keyboard acts on. */
  kbd?: string;
  /** The button's look: 'primary', 'ghost', 'danger', or '' for a plain one. */
  cls: string;
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

/** `plan`: the card shows a Claude agent's plan. `offered`: the choices its CLI offered. */
export function footerButtons(plan: boolean, offered: readonly ApprovalChoice[]): FooterButton[] {
  const has = (c: ApprovalChoice) => offered.includes(c);
  if (plan) {
    return [
      ...(has('allow_once') ? [answer('allow_once', 'Accept the plan'), { act: 'edit', label: 'Change the plan…', cls: '' } satisfies FooterButton] : []),
      ...(has('deny') ? [{ act: 'note', label: 'Send it back with a note…', kbd: '⌘⌫', cls: 'ghost' } satisfies FooterButton] : []),
      ...(has('cancel') ? [answer('cancel')] : []),
    ];
  }
  return [...ORDER.filter(has).map((c) => answer(c)), ...(has('deny') ? [{ act: 'note', label: 'Deny with a note…', kbd: '⇧⌘⌫', cls: 'ghost' } satisfies FooterButton] : [])];
}

/** The note that goes back with a plan must say something: without one the same plan comes back. */
export const PLAN_NOTE_HINT = 'Say what to change: without a note the same plan comes back.';
