// "From issue…" and "From pull request…", from the buttons' side. The dialog asks for a GitHub
// link or a number; the core reads it with your own gh and hands it to a request's Lead as
// quoted text. Work that starts this way gets a request of its own: the one you are on while
// nothing was asked in it yet, else a new one. When it cannot be read, the dialog says why and
// nothing is sent. The read cannot be stopped, so the dialog stays open until it has ended.
// When the Lead cannot start after the read, your message is in the request already: the
// request opens, and it and a toast say why.
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
  /** A read is on its way. It cannot be stopped, and the core records your message when it ends. */
  busy = $state(false);

  /** Cancel, Esc, × and a click beside the dialog. Not while a read runs: the dialog stays to show how it ended. */
  close() {
    if (!this.busy) this.dialog = null;
  }
}
export const intakeUi = new IntakeUi();

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/** The request the work goes to: `current` while nothing was asked in it yet, else a new one. */
export function intakeThread(current: string, used: (thread: string) => boolean, fresh: () => string): string {
  return used(current) ? fresh() : current;
}

const count = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;

/**
 * What the toast says once your message is in the request: what the Lead got, what came with it
 * and what was left out. Or, when the Lead could not start, why, and that it did not read it.
 */
export function intakeSummary(i: Intake): { title: string; body: string } {
  const issue = i.source.kind === 'issue';
  const what = `${issue ? 'Issue' : 'Pull request'} #${i.source.number}`;
  if (i.lead_problem !== null) {
    const why = oneLine(i.lead_problem, 300).replace(/\.$/, '');
    return {
      title: `${what} did not reach the Lead`,
      body: `The Lead could not start: ${why}. Your message is in the request. Start from the ${issue ? 'issue' : 'pull request'} again when this is fixed.`,
    };
  }
  const came = [
    i.comments ? count(i.comments, 'comment') : '',
    i.review_comments ? `${count(i.review_comments, 'comment')} on the code` : '',
    i.failing_checks ? count(i.failing_checks, 'failing check') : '',
  ].filter(Boolean);
  const title = oneLine(i.source.title, 120);
  return {
    title: `${what} is with the Lead`,
    body: [`“${title}”${came.length ? `, with ${came.join(', ')}` : ''}.`, ...i.notes].join(' '),
  };
}

/**
 * Read the issue or the pull request and start work on it. Returns why nothing was sent (null:
 * your message is in its request, which is open, and a toast says whether the Lead has it).
 */
export async function startFromGithub(kind: IntakeKind, reference: string, note: string): Promise<string | null> {
  const used = (t: string) => store.chats.some((c) => c.thread === t) || store.outbox.some((o) => o.thread === t) || !!store.leadOf(t);
  const thread = intakeThread(ui.request, used, () => `lead:${shortId()}`);
  let intake: Intake;
  intakeUi.busy = true;
  try {
    intake = await api.startFromGithub(thread, kind, reference, note);
  } catch (e) {
    return msg(e);
  } finally {
    intakeUi.busy = false;
  }
  // Its tab shows at once, like a new request's, also before the message comes back from the core.
  if (thread !== ui.request) ui.draftRequests = [...ui.draftRequests, { thread, ts: wallNow() }];
  ui.openRequest(thread);
  toast({ kind: intake.lead_problem !== null ? 'error' : intake.notes.length ? 'info' : 'success', ...intakeSummary(intake) });
  return null;
}
