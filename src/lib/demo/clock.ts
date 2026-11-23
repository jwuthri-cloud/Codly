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

/**
 * `?hour=13` (docs/DEMO.md): the page's clock is moved so that it is that hour of the day now,
 * and it goes on from there. The office and the map light their scene by `new Date()`, so they
 * are lit for that hour, and every time on screen follows. `Date` is wrapped, not the scene:
 * the product reads its clock as it always does. Call it before the first virtual clock is made.
 */
export function setWallHour(hour: number) {
  const Real = Date;
  const at = new Real();
  // Today, at that hour, on the local clock (so a change of summer time today does not move it).
  const wanted = new Real(at.getTime());
  // (minutes stop at 59: a part of an hour that rounds up to 60 would be the next hour, and after 23 the next day)
  wanted.setHours(Math.floor(hour), Math.min(59, Math.round((hour % 1) * 60)), 0, 0);
  const offset = wanted.getTime() - at.getTime();
  globalThis.Date = new Proxy(Real, {
    // `new Date()` is now; a date made from a value is that value.
    construct: (target, args, newTarget) => Reflect.construct(target, args.length ? args : [target.now() + offset], newTarget),
    get: (target, key, receiver) => (key === 'now' ? () => target.now() + offset : Reflect.get(target, key, receiver)),
  });
}
