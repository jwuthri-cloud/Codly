// The summary of your time away (AwayRow.svelte) and whether it is in the thread's view. When
// you come back the thread is at its end and the summary is above it: the row above the composer
// (NextRow.svelte) then offers a chip that takes you to it, and nothing scrolls by itself.

class AwayShown {
  /** The summary on screen now (one thread shows at a time), null: none. */
  row = $state<{ el: HTMLElement; minutes: number; inView: boolean } | null>(null);

  /** The summary's row is drawn: it is the one to jump to. Call the result when it goes. */
  show(el: HTMLElement, minutes: number): () => void {
    this.row = { el, minutes, inView: false };
    return () => {
      if (this.row?.el === el) this.row = null;
    };
  }

  /** The row came into the view, or left it. */
  seen(el: HTMLElement, inView: boolean) {
    if (this.row?.el === el && this.row.inView !== inView) this.row = { ...this.row, inView };
  }

  /** How long you were away, when the summary is out of view (null: none, or you can see it). */
  get hidden(): number | null {
    return this.row && !this.row.inView && this.row.el.isConnected ? this.row.minutes : null;
  }

  /** The chip was pressed: the summary comes into view. */
  reveal(reduced: boolean) {
    this.row?.el.scrollIntoView({ block: 'center', behavior: reduced ? 'auto' : 'smooth' });
  }
}
export const awayShown = new AwayShown();
