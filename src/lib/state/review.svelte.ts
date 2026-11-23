// Your review of a diff, for this session: the comments you wrote and did not send yet, the
// files you ticked as viewed, and the files you opened or folded. Kept per diff (a task's or a
// plan's), in memory only: a restart starts clean, and sending the comments clears them.
// Comments on finished work are sent as the message of a new request: Send back puts them in
// that request's composer (`hand`), and they stay on the diff until the message is sent.
import type { DiffFile } from '../util/diff';
import { fileMark, heavy, sameSpot, withComment, type Anchor, type ReviewComment } from '../util/review';

/** A diff's name here: "task:<id>" or "plan:<id>". */
export const reviewKey = (t: { kind: 'plan' | 'task'; id: string }): string => `${t.kind}:${t.id}`;

/** The request whose composer holds a diff's comments: its office and thread (thread names
 *  repeat across offices), the message that was put there (`text`), and the comments that
 *  message was built from (`comments`). */
type Handed = { office: string | null; thread: string; text: string; comments: ReviewComment[] };

const NONE: ReviewComment[] = [];
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

  /**
   * A message (`text`) was sent in a request. Where it has the message Send back put in the
   * request's composer, the comments of that message are sent and leave their diff: a comment
   * you wrote or changed since is not in it, and stays. A message without it sent no comment:
   * they all stay on their diff, and the request does not hold them any more.
   */
  sent(office: string | null, thread: string, text: string) {
    for (const [key, to] of Object.entries(this.handed)) {
      if (to.office !== office || to.thread !== thread) continue;
      if (text.includes(to.text)) {
        const left = this.comments(key).filter((c) => !to.comments.some((s) => sameSpot(s, c) && s.text === c.text));
        this.drafts = left.length ? { ...this.drafts, [key]: left } : without(this.drafts, key);
      }
      this.handed = without(this.handed, key);
    }
  }

  /** A request was closed with nothing sent: the comments its composer held stay on their diffs, as before Send back. */
  unhand(office: string | null, thread: string) {
    this.handed = Object.fromEntries(Object.entries(this.handed).filter(([, to]) => to.office !== office || to.thread !== thread));
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
