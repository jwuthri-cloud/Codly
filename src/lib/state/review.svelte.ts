// Your review of a diff, for this session: the comments you wrote and did not send yet, the
// files you ticked as viewed, and the files you opened or folded. Kept per diff (a task's or a
// plan's), in memory only: a restart starts clean, and sending the comments clears them.
// Comments on finished work are sent as the message of a new request: Send back puts them in
// that request's composer (`hand`), and they stay on the diff until the message is sent.
import type { Task } from '../gen/Task';
import type { DiffFile } from '../util/diff';
import { fileMark, fileOwners, heavy, sameSpot, sentIn, withComment, type Anchor, type ReviewComment } from '../util/review';
import { store } from './store.svelte';

/** A diff's name here: "task:<id>", "plan:<id>" or "agent:<id>" (a 1:1 agent's whole work). */
export const reviewKey = (t: { kind: 'plan' | 'task' | 'agent'; id: string }): string => `${t.kind}:${t.id}`;

/** The request whose composer holds a diff's comments: its office and thread (thread names
 *  repeat across offices), the message that was put there (`text`), and the comments that
 *  message was built from (`comments`). */
type Handed = { office: string | null; thread: string; text: string; comments: ReviewComment[] };

const NONE: ReviewComment[] = [];
/** The message Send back put in a request holds this comment as it is now. */
const holds = (to: Handed, c: ReviewComment): boolean => to.comments.some((s) => sameSpot(s, c) && s.text === c.text);
const without = <T>(o: Record<string, T>, key: string): Record<string, T> => Object.fromEntries(Object.entries(o).filter(([k]) => k !== key));

class Review {
  private drafts = $state.raw<Record<string, ReviewComment[]>>({});
  /** Diff → the request that holds its comments, ready to send. */
  private handed = $state.raw<Record<string, Handed>>({});
  /** Diff → path → the mark of the file's diff when you ticked it (`fileMark`). */
  private seen = $state.raw<Record<string, Record<string, string>>>({});
  /** Diff → path → a file you opened (false) or folded (true) by hand. */
  private folds = $state.raw<Record<string, Record<string, boolean>>>({});
  /** A file's mark is computed once: a parsed file never changes. */
  private marks = new WeakMap<DiffFile, string>();

  comments(key: string): ReviewComment[] {
    return this.drafts[key] ?? NONE;
  }

  /** Save the comment at a place of the diff (one per place); empty text removes it. */
  comment(key: string, at: Anchor, text: string) {
    this.drafts = { ...this.drafts, [key]: withComment(this.comments(key), at, text) };
  }

  /** Sent, or discarded: the diff has no comments left. */
  clear(key: string) {
    if (this.drafts[key]) this.drafts = without(this.drafts, key);
    if (this.handed[key]) this.handed = without(this.handed, key);
  }

  /** Send back on finished work: the diff's comments are in a request's composer now. They stay on the diff. */
  hand(key: string, to: Handed) {
    this.handed = { ...this.handed, [key]: to };
  }

  /** The request that holds the diff's comments, if one does. */
  handedTo(key: string): Handed | null {
    return this.handed[key] ?? null;
  }

  /** How many of the diff's comments, as they are now, are in the message Send back put in a request (0 when none was). */
  inHand(key: string): number {
    const to = this.handed[key];
    return to ? this.comments(key).filter((c) => holds(to, c)).length : 0;
  }

  /**
   * A message (`text`) was sent in a request. The comments Send back put in that request's
   * composer that the message still names (`sentIn`: you may have changed the words around
   * them first) are sent and leave their diff. A comment you wrote or changed on the diff
   * since is not in the message, and stays; so does one you took out of the message. A message
   * that names none sent no comment: they all stay on their diff. In each case the request
   * does not hold them any more.
   */
  sent(office: string | null, thread: string, text: string) {
    for (const [key, to] of Object.entries(this.handed)) {
      if (to.office !== office || to.thread !== thread) continue;
      const went = { ...to, comments: sentIn(text, to.comments) };
      if (went.comments.length) {
        const left = this.comments(key).filter((c) => !holds(went, c));
        this.drafts = left.length ? { ...this.drafts, [key]: left } : without(this.drafts, key);
      }
      this.handed = without(this.handed, key);
    }
  }

  private mark(f: DiffFile): string {
    let m = this.marks.get(f);
    if (m == null) this.marks.set(f, (m = fileMark(f)));
    return m;
  }

  /** Ticked, and the file's diff is still the one you looked at. */
  viewed(key: string, f: DiffFile): boolean {
    const m = this.seen[key]?.[f.path];
    return m != null && m === this.mark(f);
  }

  /** Ticking folds the file away, until its diff changes; unticking opens it. */
  setViewed(key: string, f: DiffFile, on: boolean) {
    const rest = without(this.seen[key] ?? {}, f.path);
    this.seen = { ...this.seen, [key]: on ? { ...rest, [f.path]: this.mark(f) } : rest };
    // From a tick on, the tick decides: a fold kept by hand would keep the file folded when it
    // changes later and is not viewed any more.
    const byHand = without(this.folds[key] ?? {}, f.path);
    this.folds = { ...this.folds, [key]: on ? byHand : { ...byHand, [f.path]: false } };
  }

  /** Is the file folded in this diff? As you left it by hand; else a file you ticked as viewed
   *  and a heavy one (a lockfile, generated, large) are folded, and the others are open. */
  folded(key: string, f: DiffFile): boolean {
    return this.folds[key]?.[f.path] ?? (this.viewed(key, f) || heavy(f) != null);
  }

  /** Open (false) or fold (true) a file by hand. */
  fold(key: string, path: string, on: boolean) {
    this.folds = { ...this.folds, [key]: { ...this.folds[key], [path]: on } };
  }
}

export const review = new Review();

/** Which task wrote each file of a plan's or a task's diff (`fileOwners`), from what Codly saw:
 *  the files of its commits, then the ones its agent changes now. */
export function taskOwners(scope: readonly Task[]): Map<string, Task> {
  return fileOwners(scope, function* (t) {
    for (const m of store.taskMarks[t.id] ?? []) yield* m.files ?? [];
    const live = t.assignee ? store.live[t.assignee]?.files : null;
    if (live) for (const [path, f] of Object.entries(live)) if (f.modified) yield path;
  });
}
