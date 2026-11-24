// Retry sends your message again, as a new message. The thread then holds the same words twice,
// with the failed turn between them: the second is drawn as a small row ("Sent again · 14:02"),
// not as a second bubble. And while the newest failed turn may be sent again, the head of a
// 1:1 thread offers the same Retry as its row. Pure.
import type { Item } from './spine';
import { canRetry } from './outcomes';

const norm = (s: string) => s.replace(/\s+/g, ' ').trim();

/**
 * The keys of your messages that send your message before them again: the same words, with a
 * turn between the two that failed or stopped (one that `canRetry`). A message that waits or
 * that was not sent stays whole: it has more to say than "sent again".
 */
export function sentAgain(items: Item[]): Set<string> {
  const out = new Set<string>();
  /** The words of your newest message that was drawn whole. */
  let last: string | null = null;
  /** A turn failed or stopped since your newest message. */
  let failed = false;
  for (const it of items) {
    if (it.kind === 'trouble' && canRetry(it.what)) failed = true;
    if (it.kind !== 'user') continue;
    const words = norm(it.msg.text);
    if (failed && words === last && !it.outgoing) out.add(it.key);
    else last = words;
    failed = false;
  }
  return out;
}

/**
 * Whether the trouble row at `at` offers Retry now: it has a message to send again, nothing was
 * said in the thread after it (by you or by an agent), and the agent is not in a turn.
 */
export function retryNow(items: Item[], at: number, busy: boolean): string | null {
  const it = items[at];
  if (busy || it?.kind !== 'trouble' || !it.retry) return null;
  for (let i = at + 1; i < items.length; i++) if (items[i].kind === 'user' || items[i].kind === 'say') return null;
  return it.retry;
}

/** The message the newest trouble row that offers Retry would send again (see `retryNow`), for the thread's head. */
export function headRetry(items: Item[], busy: boolean): string | null {
  for (let i = items.length - 1; i >= 0; i--) {
    const k = items[i].kind;
    if (k === 'user' || k === 'say') return null;
    const again = k === 'trouble' ? retryNow(items, i, busy) : null;
    if (again) return again;
  }
  return null;
}
