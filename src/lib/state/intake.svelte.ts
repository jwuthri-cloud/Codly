// "From issue…" and "From pull request…", from the buttons' side. The dialog asks for a GitHub
// link or a number; the core reads it with your own gh and hands it to a request's Lead as
// quoted text. Work that starts this way gets a request of its own: the one you are on while
// nothing was asked in it yet, else a new one. A failure is told in the dialog, and nothing is
// sent.
import { api } from '../api';
import type { Intake } from '../gen/Intake';
import type { IntakeKind } from '../gen/IntakeKind';
import { oneLine } from '../util/sanitize';
import { store } from './store.svelte';
import { toast } from './toasts.svelte';
import { shortId, ui, wallNow } from './ui.svelte';

class IntakeUi {
  /** The dialog that asks for the link or the number: what it starts from (null: closed). */
  dialog = $state<IntakeKind | null>(null);
}
export const intakeUi = new IntakeUi();

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/** The request the work goes to: `current` while nothing was asked in it yet, else a new one. */
export function intakeThread(current: string, used: (thread: string) => boolean, fresh: () => string): string {
  return used(current) ? fresh() : current;
}

const count = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;

/** What the toast says once the Lead has it: what it is, what came with it, and what was left out. */
export function intakeSummary(i: Intake): { title: string; body: string } {
  const came = [
    i.comments ? count(i.comments, 'comment') : '',
    i.review_comments ? `${count(i.review_comments, 'comment')} on the code` : '',
    i.failing_checks ? count(i.failing_checks, 'failing check') : '',
  ].filter(Boolean);
  const title = oneLine(i.source.title, 120);
  return {
    title: `${i.source.kind === 'issue' ? 'Issue' : 'Pull request'} #${i.source.number} is with the Lead`,
    body: [`“${title}”${came.length ? `, with ${came.join(', ')}` : ''}.`, ...i.notes].join(' '),
  };
}

/**
 * Read the issue or the pull request and start work on it. Returns what went wrong (null: the
 * Lead has it, and its request is open).
 */
export async function startFromGithub(kind: IntakeKind, reference: string, note: string): Promise<string | null> {
  const used = (t: string) => store.chats.some((c) => c.thread === t) || store.outbox.some((o) => o.thread === t) || !!store.leadOf(t);
  const thread = intakeThread(ui.request, used, () => `lead:${shortId()}`);
  let intake: Intake;
  try {
    intake = await api.startFromGithub(thread, kind, reference, note);
  } catch (e) {
    return msg(e);
  }
  // Its tab shows at once, like a new request's, also before the message comes back from the core.
  if (thread !== ui.request) ui.draftRequests = [...ui.draftRequests, { thread, ts: wallNow() }];
  ui.openRequest(thread);
  toast({ kind: intake.notes.length ? 'info' : 'success', ...intakeSummary(intake) });
  return null;
}
