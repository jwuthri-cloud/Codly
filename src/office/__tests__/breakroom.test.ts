// Breaks are social: two-player games need a partner, the PS5 takes one or two, pairs stay
// together, and someone left alone waits a little, then moves on.
import { describe, expect, it } from 'vitest';
import { BreakRoom, LONELY_MS } from '../breakroom';

const games = (m: Map<string, { game: string; slot: number }>) => Object.fromEntries([...m].map(([id, s]) => [id, `${s.game}:${s.slot}`]));

describe('break room', () => {
  it('one idle agent plays the PS5 alone; the next one joins on the second controller', () => {
    const r = new BreakRoom();
    expect(games(r.update(['ada'], 0))).toEqual({ ada: 'console:0' });
    expect(games(r.update(['ada', 'bo'], 1000))).toEqual({ ada: 'console:0', bo: 'console:1' });
  });

  it('two agents free at once pair up at a table; a third goes to the PS5; a fourth and fifth take the other table', () => {
    const r = new BreakRoom();
    const s = r.update(['ada', 'bo', 'cy'], 0);
    expect(s.get('ada')!.game).not.toBe('console');
    expect(s.get('bo')!.game).toBe(s.get('ada')!.game);
    expect([s.get('ada')!.slot, s.get('bo')!.slot]).toEqual([0, 1]);
    expect(s.get('cy')!.game).toBe('console');
    const t = r.update(['ada', 'bo', 'cy', 'dee', 'eli'], 1000);
    const other = s.get('ada')!.game === 'pool' ? 'foosball' : 'pool';
    expect([t.get('dee')!.game, t.get('eli')!.game]).toEqual([other, other]);
    expect(t.get('ada')).toEqual(s.get('ada')); // pairs are stable
  });

  it('a partner called back to work: the other waits, someone new can join, else it moves to the PS5', () => {
    const r = new BreakRoom();
    const s = r.update(['ada', 'bo'], 0);
    const table = s.get('ada')!.game;
    expect(r.update(['ada'], 1000).get('ada')!.game).toBe(table); // still waiting
    expect(r.nextAt()).toBe(1000 + LONELY_MS);
    expect(r.update(['ada', 'cy'], 2000).get('cy')).toEqual({ game: table, slot: 1 }); // cy joins
    r.update(['ada'], 3000); // cy leaves too
    expect(r.update(['ada'], 3000 + LONELY_MS).get('ada')).toEqual({ game: 'console', slot: 0 });
  });

  it('everything taken: the rest stay at their desks', () => {
    const r = new BreakRoom();
    const ids = ['a', 'b', 'c', 'd', 'e', 'f', 'g'];
    const s = r.update(ids, 0);
    expect(s.size).toBe(6); // pool 2 + foosball 2 + PS5 2
    expect(s.has('g')).toBe(false);
  });

  it('a preferred game (demo ?play=) is used first', () => {
    expect(games(new BreakRoom({ prefer: 'pool' }).update(['a', 'b'], 0))).toEqual({ a: 'pool:0', b: 'pool:1' });
    expect(games(new BreakRoom({ prefer: 'console' }).update(['a', 'b'], 0))).toEqual({ a: 'console:0', b: 'console:1' });
  });
});
