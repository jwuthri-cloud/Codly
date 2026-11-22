// Closing a request can be taken back for a few seconds. Its tab leaves at once and a toast
// offers Undo; only when that time is up does the core close it (its Lead stops, its unfinished
// plans are cancelled, its messages move to the closed requests). Until then nothing has
// changed, so Undo loses nothing: the Lead never stopped and the plan was never cancelled.
// If Codly quits before the time is up, the request is simply still open the next time.
// Opening another repository does not wait: what was closed is closed first (`flushClosing`).
import { closeRequest } from './actions';
import { requests } from './requests.svelte';
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
