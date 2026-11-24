// When you last had each thread on screen (`ui.lastSeen`, by office), kept across a restart: one
// localStorage key per office, with the 50 threads you left most recently. When a thread is
// stamped is the rule of `Presence` (away.ts): when it leaves the screen, when you look at it
// again, and once a minute while you look at it. A request you close leaves the map, and is not
// stamped again once it left the tabs.
import { requests } from '../lib/state/requests.svelte';
import { store } from '../lib/state/store.svelte';
import { isRequestThread, ui } from '../lib/state/ui.svelte';

/** How many threads an office keeps. */
export const SEEN_CAP = 50;

const keyOf = (office: string) => `codly.seen.${office}`;

/** The newest `cap` entries of `seen`. */
export function capSeen(seen: Record<string, number>, cap = SEEN_CAP): Record<string, number> {
  const entries = Object.entries(seen);
  if (entries.length <= cap) return seen;
  return Object.fromEntries(entries.sort((a, b) => b[1] - a[1]).slice(0, cap));
}

/** What an office keeps: empty when none, when the storage cannot be read, or when it holds something else. */
export function readSeen(raw: string | null): Record<string, number> {
  try {
    const v: unknown = JSON.parse(raw ?? '{}');
    if (!v || typeof v !== 'object' || Array.isArray(v)) return {};
    return Object.fromEntries(Object.entries(v).filter((e): e is [string, number] => typeof e[1] === 'number'));
  } catch {
    return {};
  }
}

/** The office's map is in memory from now on (read once from the storage). Call it from an effect, not while the page is drawn. */
export function loadSeen(office: string) {
  if (office in ui.lastSeen) return;
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(keyOf(office));
  } catch {
    /* private mode: nothing kept */
  }
  ui.lastSeen = { ...ui.lastSeen, [office]: readSeen(raw) };
}

/** When you last had `thread` of `office` on screen (undefined: not known). */
export const seenAt = (office: string | null | undefined, thread: string): number | undefined => (office ? ui.lastSeen[office]?.[thread] : undefined);

function put(office: string, seen: Record<string, number>) {
  ui.lastSeen = { ...ui.lastSeen, [office]: seen };
  try {
    localStorage.setItem(keyOf(office), JSON.stringify(seen));
  } catch {
    /* private mode: kept for this session only */
  }
}

/**
 * Whether `thread` of `office` can be stamped: a request must be one of the open office's tabs.
 * A request you just closed has left them (its stamp on the way out would put it back in the
 * map); a thread of another office, or one that is no request (a 1:1 chat), is not asked.
 */
export function stampable(office: string, thread: string): boolean {
  return !isRequestThread(thread) || office !== store.workspace?.id || requests.list.some((r) => r.thread === thread);
}

/** You had `thread` on screen until `at`. Refused for a thread that is not open (`open`). */
export function stampSeen(office: string | null | undefined, thread: string, at: number, open: (office: string, thread: string) => boolean = stampable) {
  if (office && open(office, thread)) put(office, capSeen({ ...ui.lastSeen[office], [thread]: at }));
}

/** Threads that were closed leave the map. */
export function forgetSeen(office: string | null | undefined, threads: readonly string[]) {
  const seen = office ? ui.lastSeen[office] : undefined;
  if (!office || !seen || !threads.some((t) => t in seen)) return;
  put(office, Object.fromEntries(Object.entries(seen).filter(([t]) => !threads.includes(t))));
}
