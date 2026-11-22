// Your review of a diff, for this session: the comments you wrote and did not send yet, and the
// files you ticked as viewed. Kept per diff (a task's or a plan's), in memory only: a restart
// starts clean, and sending the comments clears them.
import type { DiffFile } from '../util/diff';
import { fileMark, withComment, type Anchor, type ReviewComment } from '../util/review';

/** A diff's name here: "task:<id>" or "plan:<id>". */
export const reviewKey = (t: { kind: 'plan' | 'task'; id: string }): string => `${t.kind}:${t.id}`;

const NONE: ReviewComment[] = [];
const without = <T>(o: Record<string, T>, key: string): Record<string, T> => Object.fromEntries(Object.entries(o).filter(([k]) => k !== key));

class Review {
  private drafts = $state.raw<Record<string, ReviewComment[]>>({});
  /** Diff → path → the mark of the file's diff when you ticked it (`fileMark`). */
  private seen = $state.raw<Record<string, Record<string, string>>>({});
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

  setViewed(key: string, f: DiffFile, on: boolean) {
    const rest = without(this.seen[key] ?? {}, f.path);
    this.seen = { ...this.seen, [key]: on ? { ...rest, [f.path]: this.mark(f) } : rest };
  }
}

export const review = new Review();
