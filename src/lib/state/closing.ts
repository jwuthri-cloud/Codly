// Closing a request can be taken back for a few seconds. Its tab leaves at once and a toast
// offers Undo; only when that time is up does the core close it (its Lead stops, its unfinished
// plans are cancelled, its messages move to the closed requests). Until then nothing has
// changed, so Undo loses nothing: the Lead never stopped and the plan was never cancelled.
// If Codly quits before the time is up, the request is simply still open the next time.
// Opening another repository does not wait: what was closed is closed first (`flushClosing`).
// A close that leaves work behind asks before any of this (`closeQuestion`).
import type { Plan } from '../gen/Plan';
import { closeRequest } from './actions';
import { loadPullRequest, prKey, prUi } from './pullRequest.svelte';
import { busy } from './queue';
import { requests, type Request } from './requests.svelte';
import { store } from './store.svelte';
import { toast, toasts } from './toasts.svelte';
import { ui } from './ui.svelte';

/** How long Undo is on offer. */
export const UNDO_MS = 6000;

type Pending = { timer: ReturnType<typeof setTimeout>; toast: number; title: string };
/** By office and thread: thread names repeat across offices. */
const pending = new Map<string, Pending>();
/** The closes the core is working on right now. */
const running = new Set<Promise<void>>();
const office = (): string | null => store.workspace?.id ?? null;
const keyOf = (o: string | null, thread: string): string => `${o ?? ''}|${thread}`;

/** Undo is no longer on offer for it (taken, or its time is up). Nothing if it was not waiting. */
function settle(o: string | null, thread: string): Pending | undefined {
  const key = keyOf(o, thread);
  const p = pending.get(key);
  if (!p) return undefined;
  pending.delete(key);
  clearTimeout(p.timer);
  toasts.dismiss(p.toast);
  return p;
}

/** Its tab may show again (if the request is still there). */
function unhide(o: string | null, thread: string): void {
  ui.closingRequests = ui.closingRequests.filter((c) => !(c.office === o && c.thread === thread));
}

/** What a close asks first: the question, what is lost or kept in a few more words, and the word on the button that goes on. */
export type CloseQuestion = { ask: string; detail: string; yes: 'Close' | 'Discard' };

/**
 * What closing a request asks before anything happens. Null: it leaves nothing behind, so it
 * closes at once, with Undo. The core deletes no work on a close: a running plan is cancelled,
 * but every branch and worktree stays, a finished plan stays in Changes (where it can still be
 * merged), and a pull request is not touched. The request's tab is where that work is merged
 * from, though: a close with finished work that is not merged (a finished plan, or the done
 * tasks of a plan that failed or was cancelled), or with a pull request still open, first says
 * what becomes of them. So does a close while its Lead works (in a turn, or starting or thinking
 * before it) or before it answered your newest message: the close stops it and drops the answer.
 * A new request with nothing asked in it has no work: it asks only when
 * its composer holds text you did not send.
 */
export async function closeQuestion(r: Request): Promise<CloseQuestion | null> {
  if (r.phase === 'new') {
    const holdsText = ui.draftRequests.some((d) => d.thread === r.thread && d.kept);
    return holdsText ? { ask: 'Discard this draft?', detail: 'What you wrote in it was not sent: it goes with the tab.', yes: 'Discard' } : null;
  }
  if (r.phase === 'running') return { ask: 'Cancel its running plan and close?', detail: 'Its unfinished tasks are cancelled. Nothing is deleted: what is done so far stays on its branch.', yes: 'Close' };
  // Its Lead is in a turn (planning, answering), or starts or thinks before the first turn of a
  // request with no plan yet: the close stops it mid-way. (With a plan, what becomes of the plan is asked below.)
  const planless = !store.plans.some((p) => (p.thread || 'lead') === r.thread);
  if (busy(r.thread) || (r.phase === 'planning' && planless)) return { ask: 'The Lead is working: close and stop it?', detail: 'Its turn stops where it is. Nothing is deleted.', yes: 'Close' };
  // Your newest message has no answer yet and can still get one (the Lead has not begun its turn): the close drops it.
  if (unanswered(r.thread)) return { ask: 'Your message has no answer yet: close anyway?', detail: 'The Lead stops before it answers. Nothing is deleted.', yes: 'Close' };
  const finished = store.plans.filter((p) => (p.thread || 'lead') === r.thread && p.status === 'done');
  const pr = (planId: string) => prKey({ kind: 'plan', id: planId });
  // Their pull requests, from Codly's own record (no network), where nothing read them yet.
  await Promise.all(finished.filter((p) => !(pr(p.id) in prUi.opened)).map((p) => loadPullRequest({ kind: 'plan', id: p.id })));
  // Not merged, and not kept on its branch by your own choice: "Ready to merge".
  const notMerged = (p: Plan) => !store.mergeOf(p.id) && !p.kept && !ui.keptPlans.includes(p.id);
  const unmerged = finished.filter(notMerged);
  const open = finished.some((p) => prUi.opened[pr(p.id)]?.state === 'open');
  // A plan that failed or was cancelled finished some of its tasks: their work is on the plan's
  // branch, in no branch of yours, and the request's tab is where you see that.
  const stopped = store.plans.filter((p) => (p.thread || 'lead') === r.thread && (p.status === 'failed' || p.status === 'cancelled') && notMerged(p));
  const left = store.tasks.filter((t) => t.status === 'done' && stopped.some((p) => p.id === t.plan_id)).length;
  if (!unmerged.length && !open && !left) return null;
  const stays = 'The pull request stays open: Codly does not close or merge it.';
  if (!unmerged.length && left) {
    const ask = left === 1 ? 'A task is done and not merged: its work stays on the plan’s branch. Close?' : 'Some tasks are done and not merged: their work stays on the plan’s branch. Close?';
    const kept = 'Nothing is deleted: you can still merge it in Changes.';
    return { ask, detail: open ? `${kept} ${stays}` : kept, yes: 'Close' };
  }
  if (!unmerged.length) return { ask: 'Its pull request stays open. Close?', detail: stays, yes: 'Close' };
  const branch = unmerged.length === 1 ? unmerged[0].integration_branch : null;
  const kept = `Nothing is deleted: ${branch ?? 'its work'} stays, and you can still merge it in Changes.`;
  return open
    ? { ask: 'Not merged yet: its branch and pull request stay. Close?', detail: `${kept} ${stays}`, yes: 'Close' }
    : { ask: 'Not merged yet: its branch stays. Close?', detail: kept, yes: 'Close' };
}

/**
 * An answer to your newest message can still come: it is on its way (not "Not sent"), or it is
 * the thread's newest message and no turn of its Lead began after it or took it in. A turn that
 * took it and ended with no reply (it failed, or you stopped it) leaves nothing to wait for.
 */
function unanswered(thread: string): boolean {
  if (store.outbox.some((o) => o.thread === thread && !o.error)) return true;
  let last: { author: string; ts: number } | null = null;
  for (const c of store.chats) if (c.thread === thread && (!last || c.ts >= last.ts)) last = c;
  if (last?.author !== 'user') return false;
  const at = last.ts;
  const lead = store.threadAgent(thread);
  const turns = lead ? (store.live[lead.id]?.turns ?? []) : [];
  return !turns.some((t) => t.start >= at || !!t.steers?.some((s) => s.ts >= at));
}

/** Close a request's tab, with Undo for a few seconds. You land on the tab beside it (a new request if it was the only one). */
export function closeWithUndo(thread: string, title: string): void {
  const o = office();
  if (pending.has(keyOf(o, thread))) return;
  const list = requests.list;
  const i = list.findIndex((r) => r.thread === thread);
  ui.closingRequests = [...ui.closingRequests, { office: o, thread }];
  if (ui.request === thread) {
    const beside = list[i + 1] ?? list[i - 1];
    if (beside) ui.openRequest(beside.thread);
    else ui.newRequest();
  }
  pending.set(keyOf(o, thread), {
    timer: setTimeout(() => void finish(o, thread), UNDO_MS),
    title,
    toast: toast({ kind: 'info', title: 'Request closed', body: title, duration: UNDO_MS, action: { label: 'Undo', run: () => undoClose(o, thread) } }),
  });
}

/** Undo: the tab is back and you are on it. Nothing was stopped or cancelled. */
function undoClose(o: string | null, thread: string): void {
  if (!settle(o, thread)) return;
  unhide(o, thread);
  if (office() === o) ui.openRequest(thread);
}

/** The time for Undo is up: the core closes the request. Its tab stays away meanwhile, and comes back if the core refuses. */
function finish(o: string | null, thread: string): Promise<void> {
  const p = settle(o, thread);
  if (!p) return Promise.resolve();
  const done: Promise<void> = close(o, thread, p.title).then(() => {
    running.delete(done);
    unhide(o, thread);
  });
  running.add(done);
  return done;
}

async function close(o: string | null, thread: string, title: string): Promise<void> {
  // The core closes threads of the open office only, and a thread of the same name in another
  // office is another request. `flushClosing` runs before an office is left, so this is a
  // request closed while the next repository was already opening: it stays open, and says so.
  if (office() !== o) {
    toast({ kind: 'attention', title: 'Request not closed', body: `“${title}” is still open: another repository opened first.` });
    return;
  }
  if (await closeRequest(thread)) ui.closedRev++;
}

/**
 * Another repository is about to open: every close of this office that still waits for Undo
 * is done now, and every close the core is working on is awaited. The core closes threads of
 * the open office only, so a close left for later would be dropped (after the toast said
 * "Request closed") or would reach a thread of the same name in the next office.
 */
export async function flushClosing(): Promise<void> {
  const o = office();
  for (const c of ui.closingRequests.filter((c) => c.office === o)) await finish(o, c.thread);
  await Promise.all(running);
}
