// Zero CPU when still: with a settled office, the frame loop requests no animation frames and
// arms no timers over 10 s. The step below is the engine's real per-frame update for
// characters, effects and the camera (everything except the WebGL draw call itself).
import { Texture } from 'three';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CameraRig } from '../camera';
import { Character, lookFor, PropKit } from '../gfx/character';
import { FxSystem } from '../gfx/fx';
import { FrameLoop, type LoopHost, type StepResult } from '../scheduler';

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

function office() {
  const props = new PropKit();
  const blank = new Texture();
  const env = { emotes: () => blank, props };
  const people = ['a', 'b', 'c'].map((id, i) => {
    const c = new Character(id, lookFor(id, i, i === 0 ? 'lead' : 'worker', 'claude'), env);
    c.teleport({ x: i * 2, z: 0, face: 0 }, 'desk', Date.now(), false);
    c.setPose('sit', Date.now(), 0.33);
    return c;
  });
  const fx = new FxSystem();
  const rig = new CameraRig(16 / 10);
  rig.set({ x: 0, z: 0, dist: 40 });
  // `live` mirrors the engine: continuous motion at a ~30 fps budget while the office is on screen.
  const step = (now: number, dt: number, live = false): StepResult => {
    let busy = rig.update(dt);
    let wakeAt = Infinity;
    for (const c of people) {
      const r = c.update(now, dt, false, live);
      busy = r.moving || busy;
      wakeAt = Math.min(wakeAt, r.nextAt);
    }
    busy = fx.update(now) || busy;
    return { animating: busy, wakeAt, interval: live && busy ? 33 : 0 };
  };
  return { step, people };
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-25T12:00:00Z') });
});
afterEach(() => {
  vi.useRealTimers();
});

describe('render on demand', () => {
  it('a still office schedules zero animation frames and zero timers over 10 s', () => {
    const { step } = office();
    const h = fakeHost();
    let frames = 0;
    const loop = new FrameLoop((now, dt) => {
      frames++;
      return step(now, dt);
    }, h.host);
    loop.wake(); // mount: the pose blends settle
    vi.advanceTimersByTime(2000);
    const settled = frames;
    const rafs = h.rafs();
    const timers = h.timers();
    expect(settled).toBeGreaterThan(5);
    vi.advanceTimersByTime(10_000);
    expect(h.rafs() - rafs).toBe(0);
    expect(h.timers() - timers).toBe(0);
    expect(frames).toBe(settled);
    expect(loop.active).toEqual({ raf: false, timer: false });
    expect(vi.getTimerCount()).toBe(0);
  });

  it('typing ticks on a shared clock (a few frames a second), never every display frame', () => {
    const { step, people } = office();
    for (const c of people) c.setPose('type', Date.now(), 0.33);
    const h = fakeHost();
    let frames = 0;
    const loop = new FrameLoop((now, dt) => {
      frames++;
      return step(now, dt);
    }, h.host);
    loop.wake();
    vi.advanceTimersByTime(1000); // blends settle
    const f0 = frames;
    vi.advanceTimersByTime(3000);
    const perSecond = (frames - f0) / 3;
    // Three typists share one 150 ms clock: ~6.7 wakeups/s, not 3×, not 60.
    expect(perSecond).toBeGreaterThan(5);
    expect(perSecond).toBeLessThan(8);
    loop.dispose();
  });

  it('walking runs every frame, then stops by itself on arrival', () => {
    const { step, people } = office();
    const h = fakeHost();
    const loop = new FrameLoop(step, h.host);
    loop.wake();
    vi.advanceTimersByTime(2000);
    const before = h.rafs();
    people[1].walk([[4, 3]], { x: 6, z: 3, face: Math.PI }, 'st:board', Date.now());
    loop.wake();
    vi.advanceTimersByTime(4000);
    expect(h.rafs() - before).toBeGreaterThan(100);
    expect(people[1].walking).toBe(false);
    expect(people[1].arrivedKey).toBe('st:board');
    const after = h.rafs();
    vi.advanceTimersByTime(10_000);
    expect(h.rafs()).toBe(after);
    expect(loop.active.raf).toBe(false);
  });

  it('live: everyone breathes at about 30 frames a second, and stepped mode (reduced motion) costs nothing when still', () => {
    const { step } = office();
    const h = fakeHost();
    let live = true;
    let frames = 0;
    const loop = new FrameLoop((now, dt) => {
      frames++;
      return step(now, dt, live);
    }, h.host);
    loop.wake();
    vi.advanceTimersByTime(1000);
    const f0 = frames;
    vi.advanceTimersByTime(3000);
    const perSecond = (frames - f0) / 3;
    expect(perSecond).toBeGreaterThan(24);
    expect(perSecond).toBeLessThan(34);
    live = false; // reduced motion: stepped, still sitters cost nothing
    vi.advanceTimersByTime(2000);
    const f1 = frames;
    vi.advanceTimersByTime(5000);
    expect(frames).toBe(f1);
    loop.dispose();
  });

  it('hidden or off-screen: nothing runs, and waking does nothing until visible', () => {
    const h = fakeHost();
    let n = 0;
    const loop = new FrameLoop(() => {
      n++;
      return { animating: true, wakeAt: Infinity };
    }, h.host);
    loop.setVisible(false);
    loop.wake();
    vi.advanceTimersByTime(2000);
    expect(n).toBe(0);
    loop.setVisible(true);
    vi.advanceTimersByTime(100);
    expect(n).toBeGreaterThan(0);
    loop.setOnScreen(false);
    const k = n;
    vi.advanceTimersByTime(2000);
    expect(n).toBe(k);
    loop.dispose();
  });
});
