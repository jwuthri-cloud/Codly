// Undo / redo for build mode (⌘Z / ⇧⌘Z): every committed edit is a step, kept as the whole state
// after it (the lot and the pieces bought through Buy), so undoing is just going back to one.

export class History<T> {
  private past: T[] = [];
  private future: T[] = [];

  constructor(
    public now: T,
    private max = 100,
  ) {}

  /** A new step: what came after the current one is gone. */
  push(next: T) {
    this.past.push(this.now);
    if (this.past.length > this.max) this.past.shift();
    this.now = next;
    this.future = [];
  }

  undo(): T | null {
    const prev = this.past.pop();
    if (prev === undefined) return null;
    this.future.push(this.now);
    this.now = prev;
    return prev;
  }

  redo(): T | null {
    const next = this.future.pop();
    if (next === undefined) return null;
    this.past.push(this.now);
    this.now = next;
    return next;
  }

  /** Start over from here (a refused save puts the office back as it was). */
  reset(now: T) {
    this.past = [];
    this.future = [];
    this.now = now;
  }

  get canUndo() {
    return this.past.length > 0;
  }
  get canRedo() {
    return this.future.length > 0;
  }
}
