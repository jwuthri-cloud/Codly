// Render on demand: requestAnimationFrame only while something animates or the camera
// moves; one timer for the next scheduled change (a dwell ending, an effect starting);
// nothing at all when the office is still, the page is hidden, or the view is off screen.
//
// The host is injected so the zero-frames-when-idle guarantee is testable in Node.

export interface LoopHost {
  raf(cb: (t: number) => void): number;
  caf(id: number): void;
  setTimeout(cb: () => void, ms: number): number;
  clearTimeout(id: number): void;
  now(): number;
}

export interface StepResult {
  /** Keep rendering next frame. */
  animating: boolean;
  /** Absolute time (host clock) of the next scheduled change, or Infinity. */
  wakeAt: number;
  /** Frame interval to aim for while animating (ms); 0 = every display frame. */
  interval?: number;
}

export type StepFn = (now: number, dt: number) => StepResult;

export const browserHost = (): LoopHost => ({
  raf: (cb) => requestAnimationFrame(cb),
  caf: (id) => cancelAnimationFrame(id),
  setTimeout: (cb, ms) => window.setTimeout(cb, ms),
  clearTimeout: (id) => window.clearTimeout(id),
  now: () => performance.now(),
});

export class FrameLoop {
  private rafId = 0;
  private timer = 0;
  private last = 0;
  private visible = true;
  private onScreen = true;
  private disposed = false;
  readonly stats = { frames: 0, timers: 0 };

  constructor(
    private step: StepFn,
    private host: LoopHost,
  ) {}

  get active() {
    return { raf: this.rafId !== 0, timer: this.timer !== 0 };
  }

  /** Something changed (event, input, resize, theme): draw a frame soon. */
  wake = () => {
    if (this.disposed || !this.visible || !this.onScreen || this.rafId) return;
    this.clearTimer();
    this.rafId = this.host.raf(this.frame);
  };

  private clearTimer() {
    if (this.timer) {
      this.host.clearTimeout(this.timer);
      this.timer = 0;
    }
  }

  private frame = () => {
    this.rafId = 0;
    if (this.disposed) return;
    const now = this.host.now();
    const dt = this.last ? Math.min((now - this.last) / 1000, 0.1) : 1 / 60;
    this.last = now;
    this.stats.frames++;
    const r = this.step(now, dt);
    if (!this.visible || !this.onScreen || this.disposed) return;
    if (this.rafId) return; // woken during the step: a frame is already queued
    if (r.animating) {
      const iv = r.interval ?? 0;
      if (iv > 20) {
        // Throttled (backdrop / ambient only): sleep, then sync to the display.
        this.clearTimer();
        this.timer = this.host.setTimeout(() => {
          this.timer = 0;
          this.stats.timers++;
          this.wake();
        }, iv - 16);
      } else this.rafId = this.host.raf(this.frame);
      return;
    }
    this.last = 0; // never integrate across an idle gap
    this.clearTimer();
    if (Number.isFinite(r.wakeAt)) {
      const ms = Math.max(0, r.wakeAt - now);
      this.timer = this.host.setTimeout(() => {
        this.timer = 0;
        this.stats.timers++;
        this.wake();
      }, ms);
    }
  };

  setVisible(v: boolean) {
    this.visible = v;
    if (v) this.wake();
    else this.sleep();
  }

  setOnScreen(v: boolean) {
    this.onScreen = v;
    if (v) this.wake();
    else this.sleep();
  }

  private sleep() {
    if (this.rafId) this.host.caf(this.rafId);
    this.rafId = 0;
    this.clearTimer();
    this.last = 0;
  }

  dispose() {
    this.disposed = true;
    this.sleep();
  }
}

/** Why the office is not drawn: the window is hidden, the host covers the office (Work mode,
 *  the map), or the GL context is lost. */
export type Cover = 'window' | 'host' | 'context';

/**
 * Whether the office is drawn at all: only while nothing covers it. `onchange` fires when the
 * answer changes, never for a second cover on top of the first.
 */
export class Shown {
  private covers = new Set<Cover>();

  constructor(private onchange: (shown: boolean) => void) {}

  get value() {
    return this.covers.size === 0;
  }

  cover(by: Cover, on: boolean) {
    const was = this.value;
    if (on) this.covers.add(by);
    else this.covers.delete(by);
    if (this.value !== was) this.onchange(this.value);
  }
}

/**
 * Does the office keep moving when nothing happens (breathing, glances, the games)? In full
 * view, yes. Behind the chat only when the user asked for it (the `world_ambient` setting):
 * there the office is drawn when something happens and costs nothing otherwise. Never with
 * reduced motion.
 */
export function idleLife(o: { reducedMotion: boolean; backdrop: boolean; ambient: boolean }): boolean {
  return !o.reducedMotion && (!o.backdrop || o.ambient);
}

/**
 * A repeating tick that runs only while it is on. Turning it on ticks at once, so state that
 * is computed from the clock catches up with the time it was off; turning it off leaves no
 * timer behind.
 */
export class Ticker {
  private id: ReturnType<typeof setInterval> | null = null;

  constructor(
    private tick: () => void,
    private ms: number,
  ) {}

  get on() {
    return this.id != null;
  }

  set(on: boolean) {
    if (on === this.on) return;
    if (on) {
      this.tick();
      this.id = setInterval(this.tick, this.ms);
    } else {
      clearInterval(this.id!);
      this.id = null;
    }
  }
}
