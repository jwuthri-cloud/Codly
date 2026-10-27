import { describe, expect, it } from 'vitest';
import type { Memory } from '../gen/Memory';
import { memoryView } from '../util/memory';

const mem = (text: string, o: Partial<Memory> = {}) => ({ text, title: '', tags: [], refs: [], ...o }) as unknown as Memory;

describe('memoryView', () => {
  it('uses structured fields and strips the duplicated prefix', () => {
    const v = memoryView(mem('t1 Add games: Added a corner.', { title: 'Add games', tags: ['t1'] }));
    expect(v).toEqual({ title: 'Add games', body: 'Added a corner.', key: 't1' });
  });
  it('splits legacy summaries', () => {
    const v = memoryView(mem('t1 Full access approval mode in the core: Added ApprovalMode::Full.'));
    expect(v).toEqual({ title: 'Full access approval mode in the core', body: 'Added ApprovalMode::Full.', key: 't1' });
  });
  it('falls back to the first sentence', () => {
    const v = memoryView(mem('Use pnpm, not npm. It is faster here.'));
    expect(v.title).toBe('Use pnpm, not npm.');
    expect(v.body).toBe('It is faster here.');
  });
});
