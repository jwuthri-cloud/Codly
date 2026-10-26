// Virtual clock for the scripted demo session. Script sleeps are scheduled in virtual time and
// the speed can change at any moment (pending sleeps are rescheduled). Starting with
// `skipMs > 0` backdates the clock and runs fast until virtual time catches up with the wall
// clock, so a fast-forwarded session still has a realistic, consistent history.

type Sleeper = { at: number; resolve: () => void };

export class VirtualClock {
  private vBase: number;
  private rBase: number;
  private speed: number;
  private target: number;
  private sleepers: Sleeper[] = [];
  private handle: ReturnType<typeof setTimeout> | null = null;

  constructor(opts: { speed?: number; skipMs?: number } = {}) {
    this.target = Math.max(0.1, opts.speed ?? 1);
    const skip = Math.max(0, opts.skipMs ?? 0);
    this.vBase = Date.now() - skip;
    this.rBase = performance.now();
    this.speed = skip > 0 ? 80 : this.target;
    if (skip > 0) this.watchCatchUp();
  }

  now(): number {
    return Math.round(this.vBase + (performance.now() - this.rBase) * this.speed);
  }

  setSpeed(s: number) {
    this.vBase = this.now();
    this.rBase = performance.now();
    this.speed = Math.max(0.1, s);
    this.reschedule();
  }

  sleep(ms: number): Promise<void> {
    return new Promise((resolve) => {
      this.sleepers.push({ at: this.now() + Math.max(0, ms), resolve });
      this.reschedule();
    });
  }

  private watchCatchUp() {
    const tick = () => {
      if (this.now() >= Date.now()) this.setSpeed(this.target);
      else setTimeout(tick, 20);
    };
    setTimeout(tick, 20);
  }

  private reschedule() {
    if (this.handle) clearTimeout(this.handle);
    this.handle = null;
    if (!this.sleepers.length) return;
    let at = Infinity;
    for (const s of this.sleepers) at = Math.min(at, s.at);
    const delay = Math.max(0, (at - this.now()) / this.speed);
    this.handle = setTimeout(() => this.fire(), delay);
  }

  private fire() {
    this.handle = null;
    const t = this.now() + 1;
    const due = this.sleepers.filter((s) => s.at <= t).sort((a, b) => a.at - b.at);
    this.sleepers = this.sleepers.filter((s) => s.at > t);
    for (const s of due) s.resolve();
    this.reschedule();
  }
}
