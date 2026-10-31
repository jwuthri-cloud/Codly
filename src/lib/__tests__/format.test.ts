import { describe, expect, it } from 'vitest';
import { coins, usd } from '../util/format';

describe('coins', () => {
  it('are 🪙, never $ (real API cost)', () => {
    expect(coins(0)).toBe('🪙 0');
    expect(coins(250)).toBe('🪙 250');
    expect(coins(10000)).toBe('🪙 10,000');
    expect(coins(12.6)).toBe('🪙 13');
    expect(coins(-40)).toBe('🪙 -40');
    expect(usd(250).startsWith('$')).toBe(true);
  });
});
