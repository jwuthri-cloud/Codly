// The office costs nothing while nobody can see it: covered (Work mode, the map, a hidden window,
// a lost GL context) the frame loop schedules nothing, and the crew's home-and-back tick stops;
// shown again, both catch up at once.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Agent } from '../../lib/gen/Agent';
import type { EventRow } from '../../lib/gen/EventRow';
import { FrameLoop, idleLife, Shown, Ticker, type LoopHost } from '../scheduler';
import { CALLED_IN, SimsTracker, type Busy } from '../sims';

function fakeHost() {
  let rafs = 0;
  let timers = 0;
  const host: LoopHost = {
    raf: (cb) => {
      rafs++;
      return setTimeout(() => cb(Date.now()), 16) as unknown as number;
    },
    caf: (id) => clearTimeout(id),
    setTimeout: (cb, ms) => {
      timers++;
      return setTimeout(cb, ms) as unknown as number;
    },
    clearTimeout: (id) => clearTimeout(id),
    now: () => Date.now(),
  };
  return { host, rafs: () => rafs, timers: () => timers };
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-25T12:00:00Z') });
});
afterEach(() => {
  vi.useRealTimers();
});

/** A loop that always wants the next frame (an office in full view, everyone breathing), wired
 *  to `Shown` the way the engine wires it. */
function alwaysAnimating() {
  const h = fakeHost();
  let frames = 0;
  let caughtUp = 0;
  const loop = new FrameLoop(() => {
    frames++;
    return { animating: true, wakeAt: Infinity, interval: 33 };
  }, h.host);
  const shown = new Shown((on) => {
    if (on) caughtUp++;
    loop.setVisible(on);
  });
  loop.wake();
  return { h, loop, shown, frames: () => frames, caughtUp: () => caughtUp };
}

describe('covered: nothing runs', () => {
  it('Work mode or the map: zero frames, zero animation frames and zero timers, even when events wake it', () => {
    const o = alwaysAnimating();
    vi.advanceTimersByTime(1000);
    expect(o.frames()).toBeGreaterThan(20);

    o.shown.cover('host', true);
    const frames = o.frames();
    const rafs = o.h.rafs();
    const timers = o.h.timers();
    for (let i = 0; i < 50; i++) {
      o.loop.wake(); // an event arrives
      vi.advanceTimersByTime(200);
    }
    expect(o.frames()).toBe(frames);
    expect(o.h.rafs()).toBe(rafs);
    expect(o.h.timers()).toBe(timers);
    expect(o.loop.active).toEqual({ raf: false, timer: false });
    expect(vi.getTimerCount()).toBe(0);
    o.loop.dispose();
  });

  it('shown again: the catch-up runs once and a frame is drawn within 250 ms', () => {
    const o = alwaysAnimating();
    o.shown.cover('host', true);
    vi.advanceTimersByTime(60_000);
    const frames = o.frames();
    expect(o.caughtUp()).toBe(0);

    o.shown.cover('host', false);
    expect(o.caughtUp()).toBe(1);
    vi.advanceTimersByTime(250);
    expect(o.frames()).toBeGreaterThan(frames);
    o.loop.dispose();
  });

  it('stays covered until every cover is gone (the window comes back while Work mode is on)', () => {
    const o = alwaysAnimating();
    o.shown.cover('host', true);
    o.shown.cover('window', true);
    o.shown.cover('context', true);
    const frames = o.frames();

    o.shown.cover('window', false);
    o.shown.cover('context', false);
    vi.advanceTimersByTime(2000);
    expect(o.shown.value).toBe(false);
    expect(o.frames()).toBe(frames);
    expect(o.caughtUp()).toBe(0);

    o.shown.cover('host', false);
    vi.advanceTimersByTime(100);
    expect(o.shown.value).toBe(true);
    expect(o.frames()).toBeGreaterThan(frames);
    expect(o.caughtUp()).toBe(1);
    o.loop.dispose();
  });

  it('says so only when the answer changes', () => {
    const seen: boolean[] = [];
    const shown = new Shown((on) => seen.push(on));
    shown.cover('host', false); // was not covered
    shown.cover('host', true);
    shown.cover('host', true);
    shown.cover('window', true);
    shown.cover('host', false);
    shown.cover('window', false);
    expect(seen).toEqual([false, true]);
  });
});

describe('idle life', () => {
  it('plays in full view; behind the chat only when asked; never with reduced motion', () => {
    expect(idleLife({ reducedMotion: false, backdrop: false, ambient: false })).toBe(true);
    expect(idleLife({ reducedMotion: false, backdrop: true, ambient: false })).toBe(false);
    expect(idleLife({ reducedMotion: false, backdrop: true, ambient: true })).toBe(true);
    expect(idleLife({ reducedMotion: true, backdrop: false, ambient: true })).toBe(false);
    expect(idleLife({ reducedMotion: true, backdrop: true, ambient: true })).toBe(false);
  });
});

describe('the home-and-back tick', () => {
  const MIN = 60_000;
  const HOUR = 60 * MIN;
  const ada = { id: 'ada', name: 'ada', provider: 'claude', role: 'worker', account: null, thread: null } as Agent;

  it('ticks every period while on, and leaves no timer behind when off', () => {
    let n = 0;
    const t = new Ticker(() => n++, 2000);
    vi.advanceTimersByTime(10_000);
    expect(n).toBe(0);
    expect(vi.getTimerCount()).toBe(0);

    t.set(true);
    expect(n).toBe(1); // at once
    vi.advanceTimersByTime(6000);
    expect(n).toBe(4);

    t.set(false);
    expect(t.on).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(60_000);
    expect(n).toBe(4);
  });

  it('turned on again it ticks at once, so the time away is caught up', () => {
    const at: number[] = [];
    const t = new Ticker(() => at.push(Date.now()), 2000);
    t.set(true);
    t.set(false);
    vi.advanceTimersByTime(3_600_000); // an hour in Work mode
    const back = Date.now();
    t.set(true);
    expect(at[at.length - 1]).toBe(back);
    t.set(false);
  });

  it('an hour without ticks loses nothing: the first tick back leaves the crew where ticking all along would have', () => {
    const working = () => true;
    const watched = new SimsTracker();
    const away = new SimsTracker();
    for (const s of [watched, away]) {
      s.setAgents([ada], 0);
      s.setActivity('ada', 0, 'work');
    }
    for (let t = 2000; t <= HOUR; t += 2000) watched.tick(t, working);
    away.tick(HOUR, working);
    const a = watched.stats('ada', HOUR, null).needs;
    const b = away.stats('ada', HOUR, null).needs;
    expect(a.energy).toBeLessThan(0.9); // an hour of work shows
    expect(b.energy).toBeCloseTo(a.energy, 2);
    expect(b.morale).toBeCloseTo(a.morale, 1);
    expect(b.hunger).toBeCloseTo(a.hunger, 2);
    expect(b.thirst).toBeCloseTo(a.thirst, 2);
  });

  /** Ada, empty, went home at 0: for 10 minutes. */
  function goneHome() {
    const s = new SimsTracker();
    s.setAgents([ada], 0);
    s.setEnergy('ada', 0, 0);
    s.sendHome('ada', 0);
    return s;
  }
  /** Something ada did (its tests passed), as the office hears it. */
  const from = (ts: number) => ({ id: 1, ts, agent_id: 'ada', task_id: null, run_id: null, kind: { type: 'tool_completed', call_id: 'c1', ok: true, tests_passed: 3, tests_failed: 0 } }) as EventRow;
  /** What OfficeView does with an event while the office is hidden (no clock tick). */
  function hears(s: SimsTracker, ev: EventRow, busy: Busy) {
    if (s.atHome(ev)) s.tick(ev.ts, () => busy);
    s.ingest(ev);
  }

  it('at home when the ticks stop: it came back when its time was up, and the time since is time at its desk', () => {
    const idle = () => false;
    const watched = goneHome();
    const away = goneHome();
    const peeked = goneHome(); // something reads it meanwhile (a sheet, a feeling from an event)
    for (let t = 2000; t <= HOUR; t += 2000) watched.tick(t, idle);
    peeked.stats('ada', 30 * MIN, null);
    for (const s of [away, peeked]) s.tick(HOUR, idle);
    const a = watched.stats('ada', HOUR, null);
    expect(a.rest.home).toBeNull();
    expect(a.needs.energy).toBeGreaterThan(0.9);
    expect(a.needs.energy).toBeLessThan(0.97); // full at 10 min, then 50 minutes at its desk
    expect(a.needs.hunger).toBeLessThan(1);
    for (const s of [away, peeked]) {
      const b = s.stats('ada', HOUR, null);
      expect(b.rest.home).toBeNull();
      expect(b.needs.energy).toBeCloseTo(a.needs.energy, 3);
      expect(b.needs.hunger).toBeCloseTo(a.needs.hunger, 3);
      expect(b.needs.thirst).toBeCloseTo(a.needs.thirst, 3);
      expect(b.needs.morale).toBeCloseTo(a.needs.morale, 2);
      expect(b.moodlets.some((m) => m.label === 'Rested')).toBe(false); // it came back long ago
      expect(s.drain().home.at(-1)).toEqual({ id: 'ada', until: null });
    }
  });

  it('given work as it comes back while nobody looks: its first event brings it back, and the work since tires it', () => {
    const start = 10 * MIN + 4000; // the core gave it a task when its time was up
    const watched = goneHome();
    for (let t = 2000; t <= 2 * HOUR; t += 2000) {
      watched.tick(t, () => t >= start);
      if (t === start) {
        watched.ingest(from(t));
        watched.setActivity('ada', t, 'work'); // the office sees it at its desk
      }
    }
    const away = goneHome();
    expect(away.atHome(from(start))).toBe(true);
    hears(away, from(start), 'task');
    expect(away.atHome(from(start))).toBe(false);
    // Back on time, so no Overtime: it was rested at 10 min, not when the event came.
    expect(away.stats('ada', start, null).moodlets.find((m) => m.key === 'rest')).toMatchObject({ label: 'Rested', at: 10 * MIN });
    away.setActivity('ada', start, 'work'); // it is in the office again: the director says what it does
    away.tick(2 * HOUR, () => true);
    const a = watched.stats('ada', 2 * HOUR, null).needs;
    const b = away.stats('ada', 2 * HOUR, null).needs;
    expect(a.energy).toBeLessThan(0.6); // close to two hours of work show
    expect(b.energy).toBeCloseTo(a.energy, 2);
    expect(b.hunger).toBeCloseTo(a.hunger, 2);
    expect(b.thirst).toBeCloseTo(a.thirst, 2);
    expect(b.morale).toBeCloseTo(a.morale, 1);
  });

  it('called in while nobody looks: its first event is the call-in (Overtime, half of what it banked)', () => {
    const start = 4 * MIN; // nobody else was free: the core started its task at home
    const watched = goneHome();
    for (let t = 2000; t <= start; t += 2000) watched.tick(t, () => (t >= start ? 'task' : false));
    watched.ingest(from(start));
    const away = goneHome();
    hears(away, from(start), 'task');
    const a = watched.stats('ada', start, null);
    const b = away.stats('ada', start, null);
    expect(b.rest.home).toBeNull();
    expect(b.needs.energy).toBeCloseTo(0.2, 3); // half of the 40% from four minutes at home
    expect(b.needs.energy).toBeCloseTo(a.needs.energy, 3);
    expect(b.needs.morale).toBeCloseTo(a.needs.morale, 2);
    expect(b.moodlets.find((m) => m.key === 'rest')).toMatchObject({ label: 'Overtime', why: CALLED_IN });
    // An event from someone who is not at home ticks nothing.
    expect(away.atHome(from(start + 1000))).toBe(false);
    expect(away.atHome({ ...from(start), agent_id: null })).toBe(false);
  });

  it('turning it on twice keeps one timer', () => {
    let n = 0;
    const t = new Ticker(() => n++, 2000);
    t.set(true);
    t.set(true);
    expect(n).toBe(1);
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(2000);
    expect(n).toBe(2);
    t.set(false);
  });
});
