// Furniture uses: what each piece is for, where people go to use it, and agents picking
// furniture on their breaks.
import { describe, expect, it } from 'vitest';
import { BreakRoom, type Spot } from '../breakroom';
import type { Piece } from '../buy';
import { spotsOf, usesOf } from '../use';

const sofa: Piece = { uid: 's', item: 'kenney.loungeSofa', x: 10, y: 0, z: 10, rot: 0, dims: { w: 2.1, d: 0.9, h: 0.7 } };

describe('furniture uses', () => {
  it('knows what pieces are for', () => {
    expect(usesOf('kenney.loungeSofa')).toEqual(['sit', 'nap']);
    expect(usesOf('kenney.chairDesk')).toEqual(['sit']);
    expect(usesOf('kenney.bookcaseOpen')).toEqual(['read']);
    expect(usesOf('kenney.kitchenCoffeeMachine')).toEqual(['coffee']);
    expect(usesOf('kenney.televisionModern')).toEqual(['watch']);
    expect(usesOf('kenney.radio')).toEqual(['dance']);
    expect(usesOf('kenney.rugRound')).toEqual([]);
  });

  it('seats people along a sofa, facing its front, and puts a TV watcher in front', () => {
    const seats = spotsOf(sofa, 'sit');
    expect(seats).toHaveLength(3);
    [9.3, 10, 10.7].forEach((x, i) => expect(seats[i].x).toBeCloseTo(x));
    expect(seats.every((s) => s.pose === 'sit' && s.face === 0 && s.seat! > 0.25 && s.seat! < 0.45)).toBe(true);
    expect(spotsOf(sofa, 'nap')).toHaveLength(1);
    // Turned to face east: seats run along z, a standing spot is east of it, facing west.
    const tv = spotsOf({ ...sofa, item: 'kenney.televisionModern', rot: Math.PI / 2 }, 'watch')[0];
    expect(tv.x).toBeGreaterThan(11);
    expect(tv.z).toBeCloseTo(10);
    expect(tv.face).toBeCloseTo(Math.PI * 1.5);
  });

  it('sends a reader to a bought bookcase when the library is full, and lets others lounge', () => {
    const room = new BreakRoom();
    const spots: Spot[] = [
      { uid: 'shelf', use: 'read', n: 1 },
      { uid: 'sofa', use: 'sit', n: 3 },
    ];
    const seats = room.update([{ id: 'r1', prefer: 'library' }, { id: 'r2', prefer: 'library' }, { id: 'r3', prefer: 'library' }], 0, spots);
    expect([...seats.values()].filter((s) => s.game === 'library')).toHaveLength(2);
    expect([...seats.values()].find((s) => s.game === 'piece')).toMatchObject({ piece: 'shelf', use: 'read' });
    // Selling the piece frees the seat: the reader goes back to being matched.
    const after = room.update([{ id: 'r1', prefer: 'library' }, { id: 'r2', prefer: 'library' }, { id: 'r3', prefer: 'library' }], 1, [spots[1]]);
    expect([...after.values()].some((s) => s.piece === 'shelf')).toBe(false);
  });

  it('never seats two people on the same spot', () => {
    const room = new BreakRoom();
    const ids = Array.from({ length: 12 }, (_, i) => ({ id: `a${i}`, prefer: 'nap' as const }));
    const seats = [...room.update(ids, 0, [{ uid: 'sofa', use: 'nap', n: 1 }]).values()];
    const keys = seats.filter((s) => s.game === 'piece').map((s) => `${s.piece}:${s.slot}`);
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe('break activities end, like Sims actions', () => {
  it('drinks the coffee, then does something else', () => {
    const room = new BreakRoom();
    const t0 = 1_000_000;
    expect(room.update([{ id: 'a', prefer: 'coffee' }], t0).get('a')?.game).toBe('coffee');
    expect(room.update([{ id: 'a', prefer: 'coffee' }], t0 + 20_000).get('a')?.game).toBe('coffee');
    // The coffee is drunk: not the kitchen again for a while, even though it likes coffee.
    const next = room.update([{ id: 'a', prefer: 'coffee' }], t0 + 26_000).get('a');
    expect(next?.game).toBe('console');
    expect(room.nextAt()).toBeLessThan(Infinity);
  });

  it('plays a game for a while, then the pair moves on to another one', () => {
    const room = new BreakRoom({ prefer: 'pool' });
    const t0 = 2_000_000;
    const first = room.update(['a', 'b'], t0);
    expect([first.get('a')?.game, first.get('b')?.game]).toEqual(['pool', 'pool']);
    const later = room.update(['a', 'b'], t0 + 151_000);
    expect(later.get('a')?.game).not.toBe('pool');
    expect(later.get('b')?.game).not.toBe('pool');
  });
});
