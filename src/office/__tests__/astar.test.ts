import { describe, expect, it } from 'vitest';
import { aStar, lineOfSight, smooth } from '../astar';
import { cellOf, officeLayout, passableFn } from '../layout';

const grid = (rows: string[]) => {
  const h = rows.length;
  const w = rows[0].length;
  return { w, h, pass: (x: number, y: number) => rows[y]?.[x] === '.' };
};

describe('A*', () => {
  it('walks straight across an empty room', () => {
    const g = grid(['.....', '.....', '.....']);
    const p = aStar(g.w, g.h, 0, 1, 4, 1, g.pass)!;
    expect(p).toEqual([
      [1, 1],
      [2, 1],
      [3, 1],
      [4, 1],
    ]);
  });

  it('goes around furniture and never cuts a corner', () => {
    const g = grid(['.....', '.##..', '.##..', '.....']);
    const p = aStar(g.w, g.h, 0, 0, 3, 3, g.pass)!;
    expect(p.at(-1)).toEqual([3, 3]);
    for (const [x, y] of p) expect(g.pass(x, y)).toBe(true);
    // Consecutive steps that move diagonally must have both orthogonal neighbours free.
    let prev: [number, number] = [0, 0];
    for (const c of p) {
      const dx = c[0] - prev[0];
      const dy = c[1] - prev[1];
      if (dx && dy) expect(g.pass(prev[0] + dx, prev[1]) && g.pass(prev[0], prev[1] + dy)).toBe(true);
      prev = c;
    }
  });

  it('returns null when the goal is walled off', () => {
    const g = grid(['..#..', '..#..', '..#..']);
    expect(aStar(g.w, g.h, 0, 0, 4, 2, g.pass)).toBeNull();
  });

  it('prefers straight runs (turn penalty) and smoothing keeps only corners in line of sight', () => {
    const g = grid(['.......', '.......', '.......', '.......']);
    const raw = aStar(g.w, g.h, 0, 0, 6, 3, g.pass)!;
    const s = smooth([0, 0], raw, g.pass);
    expect(s.at(-1)).toEqual([6, 3]);
    expect(s.length).toBe(1);
    expect(lineOfSight(0, 0, 6, 3, g.pass)).toBe(true);
    const w = grid(['...#...', '...#...', '.......']);
    expect(lineOfSight(0, 0, 6, 0, w.pass)).toBe(false);
  });
});

describe('the office floor plan', () => {
  const L = officeLayout();
  const pass = passableFn(L);
  const door = cellOf(L, L.door.inside.x, L.door.inside.z);

  it('every desk seat, helper spot and station slot is reachable from the front door', () => {
    const targets: [string, number, number][] = [];
    for (const d of L.desks) {
      targets.push([`desk ${d.index} seat`, d.seat.x, d.seat.z]);
      targets.push([`desk ${d.index} stand`, d.stand.x, d.stand.z]);
      targets.push([`desk ${d.index} intern`, d.intern.x, d.intern.z]);
    }
    for (const st of Object.values(L.stations)) st.slots.forEach((s, i) => targets.push([`${st.id}:${i}`, s.x, s.z]));
    for (const [name, x, z] of targets) {
      const [gx, gz] = cellOf(L, x, z);
      const p = aStar(L.cols, L.rows, door[0], door[1], gx, gz, pass);
      expect(p, name).not.toBeNull();
    }
  });

  it('the games corner: console, foosball and pool slots are all reachable from the door', () => {
    for (const id of ['games.console', 'games.foosball', 'games.pingpong', 'games.pool'] as const) {
      expect(L.stations[id].slots).toHaveLength(2);
      for (const s of L.stations[id].slots) {
        const [gx, gz] = cellOf(L, s.x, s.z);
        expect(aStar(L.cols, L.rows, door[0], door[1], gx, gz, pass), `${id} ${s.x},${s.z}`).not.toBeNull();
      }
    }
  });

  it('has one desk per Lead in the corner office and stable pods for everyone else', () => {
    expect(L.desks.filter((d) => d.lead)).toHaveLength(3);
    expect(L.desks.filter((d) => !d.lead).length).toBeGreaterThanOrEqual(16);
    expect(L.rooms.map((r) => r.id).sort()).toEqual(['archive', 'lead', 'library', 'lobby', 'lounge', 'mail', 'meeting', 'server', 'studio']);
  });
});
