// Quarter turns (Q/E): the camera goes all the way around, and the drag orbit follows the turn.
import { describe, expect, it } from 'vitest';
import { CameraRig, DEFAULT_YAW } from '../camera';

describe('camera turns', () => {
  it('turns a quarter at a time, all the way around', () => {
    const rig = new CameraRig(1.5);
    for (let i = 0; i < 4; i++) rig.turn(1);
    expect(rig.goal.yaw).toBeCloseTo(DEFAULT_YAW + 2 * Math.PI);
    rig.turn(-1);
    expect(rig.goal.yaw).toBeCloseTo(DEFAULT_YAW + 1.5 * Math.PI);
  });

  it('keeps the drag orbit near the current quarter', () => {
    const rig = new CameraRig(1.5);
    rig.turn(1);
    rig.orbit(-10_000, 0);
    expect(rig.goal.yaw).toBeCloseTo(DEFAULT_YAW + Math.PI / 2 + 0.75);
  });
});
