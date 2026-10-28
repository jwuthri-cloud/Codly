// Motion is continuous: no pose jumps between two angles from one frame to the next any more
// (the old stepped loops did), and two characters never move in sync.
import { describe, expect, it } from 'vitest';
import type { Pose } from '../behavior';
import { J, targets, type MotionCtx, type PlayCue } from '../gfx/motion';

const POSES: Pose[] = ['sit', 'type', 'read', 'think', 'phone', 'sleep', 'slump', 'stand', 'browse', 'write', 'watch', 'raise', 'celebrate', 'drop', 'talk', 'review', 'wave', 'gamepad', 'foosball', 'pool'];
const at = (pose: Pose, t: number, seed: number, cue: PlayCue | null = null) => {
  const out = new Float32Array(J);
  const x: MotionCtx = { t, now: t * 1000, seed, cue, live: true };
  targets(out, pose, pose === 'sit' || pose === 'type' || pose === 'gamepad', x);
  return out;
};

describe('motion', () => {
  it('every pose moves smoothly at 60 fps for a minute (no frame-to-frame jumps)', () => {
    for (const pose of POSES) {
      let prev = at(pose, 0, 7);
      let worst = 0;
      for (let f = 1; f < 3600; f++) {
        const cur = at(pose, f / 60, 7);
        for (let j = 0; j < J; j++) worst = Math.max(worst, Math.abs(cur[j] - prev[j]));
        prev = cur;
      }
      expect(worst, pose).toBeLessThan(0.12);
    }
  });

  it('two characters typing are not in sync', () => {
    let diff = 0;
    for (let f = 0; f < 600; f++) diff += Math.abs(at('type', f / 60, 1)[7] - at('type', f / 60, 2)[7]);
    expect(diff / 600).toBeGreaterThan(0.005);
  });

  it('a game reaction shows, then fades: a goal against makes you groan with your hands on your head', () => {
    const cue = (react: PlayCue['react']): PlayCue => ({ intensity: 0.5, lean: 0, phase: null, t: 0, react });
    const groan = { kind: 'groan' as const, at: 10_000 };
    const calm = at('foosball', 10.5, 3, cue(null));
    expect(at('foosball', 10.5, 3, cue(groan))[8]).toBeLessThan(calm[8] - 1); // half a second in: arms up
    const after = at('foosball', 13, 3, cue(groan)); // it has ended
    expect(Math.abs(after[8] - at('foosball', 13, 3, cue(null))[8])).toBeLessThan(1e-6);
  });

  it('the pool shooter bends to aim and stands back up to watch', () => {
    const cue = (phase: PlayCue['phase'], t: number): PlayCue => ({ intensity: 0.8, lean: 0, phase, t, react: null });
    expect(at('pool', 1, 4, cue('aim', 0.5))[1]).toBeGreaterThan(0.5);
    expect(at('pool', 1, 4, cue('watch', 1))[1]).toBeLessThan(0.2);
  });
});
