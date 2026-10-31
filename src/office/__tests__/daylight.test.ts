import { describe, expect, it } from 'vitest';
import { daylight, themeAt } from '../palette';

describe('the office clock', () => {
  it('lights the office by the hour', () => {
    expect(daylight(12)).toEqual({ dark: 0, night: 0 });
    expect(daylight(20).dark).toBe(1);
    expect(daylight(2)).toEqual({ dark: 1, night: 1 });
    expect(daylight(7).dark).toBeCloseTo(0.5);
    expect([9, 18.5, 23, 6.5].map((h) => [themeAt(h).name, themeAt(h).window.sky])).toEqual([
      ['light', 'day'],
      ['dark', 'dusk'],
      ['dark', 'night'],
      ['dark', 'dusk'],
    ]);
    expect(themeAt(1).exposure).toBeLessThan(themeAt(19.5).exposure);
  });
});
