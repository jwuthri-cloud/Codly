// With Reduce Motion the neighbourhood's walkers stand: no bob, and no frames of their own.
import { describe, expect, it } from 'vitest';
import { walkerBob } from '../scene';

const sceneTs = Object.values(import.meta.glob<string>('/src/world/scene.ts', { query: '?raw', import: 'default', eager: true }))[0];

describe('a walker of the neighbourhood', () => {
  it('bobs as it walks, and stands still with Reduce Motion', () => {
    expect(walkerBob(1000, 'ada', false)).not.toBe(walkerBob(2000, 'ada', false));
    expect([walkerBob(1000, 'ada', true), walkerBob(2000, 'ada', true)]).toEqual([0, 0]);
  });

  it('asks for no frames while it stands: the scene reads the system setting', () => {
    expect(sceneTs).toMatch(/const still = ui\.reducedMotion;/);
    expect(sceneTs).toMatch(/walking: !still && walkers\.length > 0/);
  });
});
