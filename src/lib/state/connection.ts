// The page's link to Codly when a browser runs it on `codly serve`: up, down and trying again,
// or signed out. The store reports each connect that failed or reached Codly; this module
// keeps the state the bar at the top shows (src/shell/ConnectionBar.svelte) and the waits
// between tries. The app's window has no such link: its state stays "connected".

export type Link =
  | { kind: 'connected' }
  /** The event stream is down. `lastTry`: when the last try ended (ms since the epoch);
   *  `trying`: a try runs now. */
  | { kind: 'down'; lastTry: number; trying: boolean }
  /** Codly answers, but this page has no valid sign-in (a restart made a new link). No loop. */
  | { kind: 'signedOut' }
  /** Just back after it was down: "Connected again", for `BACK_MS`. */
  | { kind: 'back' };

/** The waits before each try once the stream is down: the last one repeats with no end. */
export const WAITS_MS = [1000, 2000, 5000, 10_000];
/** How long "Connected again" stays. */
export const BACK_MS = 2000;

/** The wait before the next try, after `failures` tries that failed in a row (1 or more). */
export function waitAfter(failures: number): number {
  return WAITS_MS[Math.min(Math.max(failures, 1), WAITS_MS.length) - 1];
}

/** This page cannot reach Codly (down or signed out): the polls that call it skip. */
export const isDown = (l: Link): boolean => l.kind === 'down' || l.kind === 'signedOut';

export interface Clock {
  now(): number;
  setTimeout(f: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

const systemClock: Clock = {
  now: () => Date.now(),
  setTimeout: (f, ms) => setTimeout(f, ms),
  clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

/** The link's state and its tries. `attempt` connects once and reports its outcome back with
 *  `lost` or `reached` (so do the connects that start elsewhere: a workspace switch, a resync). */
export class Reconnect {
  link: Link = { kind: 'connected' };
  private failures = 0;
  private timer: unknown = null;
  private busy = false;

  constructor(
    private readonly attempt: () => Promise<void>,
    private readonly onChange: (l: Link) => void,
    private readonly clock: Clock = systemClock,
  ) {}

  /** A connect could not reach Codly (`signedOut`: Codly said this page is not signed in). */
  lost(signedOut: boolean): void {
    this.cancel();
    if (signedOut) {
      this.failures = 0;
      this.set({ kind: 'signedOut' });
      return;
    }
    this.failures++;
    this.set({ kind: 'down', lastTry: this.clock.now(), trying: false });
    this.schedule();
  }

  /** A connect reached Codly. */
  reached(): void {
    this.failures = 0;
    if (!isDown(this.link)) return; // "Connected again" keeps its time
    this.cancel();
    this.set({ kind: 'back' });
    this.timer = this.clock.setTimeout(() => {
      this.timer = null;
      this.set({ kind: 'connected' });
    }, BACK_MS);
  }

  /** "Try now": one try at once, unless one runs. Nothing to do while the link is up. */
  async tryNow(): Promise<void> {
    if (this.busy || !isDown(this.link)) return;
    this.cancel();
    this.busy = true;
    if (this.link.kind === 'down') this.set({ ...this.link, trying: true });
    try {
      await this.attempt();
    } finally {
      this.busy = false;
    }
    // A try a newer connect took over reports nothing, and that one may have reported while
    // this one ran (its wait then ended on a busy try): the waits go on.
    if (this.link.kind === 'down' && this.timer === null) this.schedule();
  }

  private schedule() {
    this.timer = this.clock.setTimeout(() => {
      this.timer = null;
      void this.tryNow();
    }, waitAfter(this.failures));
  }

  private cancel() {
    if (this.timer !== null) this.clock.clearTimeout(this.timer);
    this.timer = null;
  }

  private set(l: Link) {
    this.link = l;
    this.onChange(l);
  }
}
