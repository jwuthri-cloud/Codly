import { describe, expect, it } from 'vitest';
import type { LotSlot } from '../../lib/gen/LotSlot';
import { along, homeOf, momentOf, NEAR, officeFront, pathOf, streetPath, WALK_MAX_MS, type Pt } from '../home';
import { plotsOf } from '../map';
import { doorOf, townOf, type Rect } from '../town';

const slot = (id: string, x: number, z: number, size: LotSlot['size']): LotSlot => ({
  id,
  x,
  z,
  size,
  owned: true,
  workspace_id: id,
  name: id,
  crew: 0,
  price: 0,
  template: null,
  gift: false,
  sells_for: 0,
});

const plots = plotsOf([slot('a', 0, 0, 'large'), slot('b', 1, 0, 'small'), slot('c', 0, 1, 'medium')]);
const town = townOf(plots);
const ids = ['ada', 'bo', 'cy', 'dee', 'eve', 'fay', 'gus', 'hal', 'ivy', 'jo'];

const within = (r: Rect, p: Pt) => p.x >= r.x0 - 1e-6 && p.x <= r.x1 + 1e-6 && p.z >= r.z0 - 1e-6 && p.z <= r.z1 + 1e-6;
const strictlyIn = (r: Rect, p: Pt) => p.x > r.x0 + 1e-6 && p.x < r.x1 - 1e-6 && p.z > r.z0 + 1e-6 && p.z < r.z1 - 1e-6;
/** Points every half metre along a segment. */
function samples(a: Pt, b: Pt): Pt[] {
  const n = Math.max(1, Math.ceil(Math.hypot(b.x - a.x, b.z - a.z) / 0.5));
  return Array.from({ length: n + 1 }, (_, i) => ({ x: a.x + ((b.x - a.x) * i) / n, z: a.z + ((b.z - a.z) * i) / n }));
}

describe('agents at home, on the map', () => {
  it('picks the same house every time, one of the nearest to its office', () => {
    for (const p of plots)
      for (const id of ids) {
        const i = homeOf(id, town, p);
        expect(i).toBeGreaterThanOrEqual(0);
        expect(homeOf(id, townOf(plots), p)).toBe(i);
        expect(town.houses[i].kind).toBe('house');
        const f = officeFront(p);
        const d = (k: number) => Math.hypot(doorOf(town.houses[k]).x - f.x, doorOf(town.houses[k]).z - f.z);
        const closer = town.houses.filter((h, k) => h.kind === 'house' && d(k) < d(i)).length;
        expect(closer).toBeLessThan(NEAR);
      }
    // Different agents of an office don't all live in one house.
    expect(new Set(ids.map((id) => homeOf(id, town, plots[0]))).size).toBeGreaterThan(1);
  });

  it('walks from the office front to the house door on the streets and sidewalks only', () => {
    const footprints = town.houses.map((h) => ({ x0: h.x - h.w / 2, z0: h.z - h.d / 2, x1: h.x + h.w / 2, z1: h.z + h.d / 2 }));
    for (const p of plots)
      for (const id of ids) {
        const h = town.houses[homeOf(id, town, p)];
        const path = streetPath(town, p, h);
        expect(path[0]).toEqual(officeFront(p));
        expect(path[path.length - 1]).toEqual(doorOf(h));
        // Square to the grid: every leg runs along x or along z.
        for (let i = 1; i < path.length; i++) expect(path[i].x === path[i - 1].x || path[i].z === path[i - 1].z).toBe(true);
        for (let i = 1; i < path.length; i++) {
          // The first leg (office front to the sidewalk) and the last (garden path) cross a verge or
          // a yard; everything between is on a sidewalk or a road.
          const pts = samples(path[i - 1], path[i]);
          const street = i > 1 && i < path.length - 1;
          for (const q of pts) {
            if (street) expect(town.walks.some((w) => within(w, q)) || town.roads.some((r) => within(r, q))).toBe(true);
            expect(plots.some((o) => strictlyIn(o, q))).toBe(false);
            expect(footprints.some((f) => strictlyIn(f, q))).toBe(false);
          }
        }
      }
  });

  it('is out on the street for its first and last minute away, home in between, gone after', () => {
    const p = plots[0];
    const path = pathOf(streetPath(town, p, town.houses[homeOf('ada', town, p)]));
    const a = { id: 'ada', name: 'Ada', color: 2, since: 1_000_000, until: 1_000_000 + 10 * 60_000 };
    expect(momentOf(a, path, a.since - 1)).toBeNull();
    const start = momentOf(a, path, a.since)!;
    expect(start.phase).toBe('out');
    expect({ x: start.at.x, z: start.at.z }).toEqual(path.pts[0]);
    const mid = momentOf(a, path, a.since + 10_000)!;
    expect(mid.phase).toBe('out');
    expect(momentOf(a, path, a.since + WALK_MAX_MS + 1)!.phase).toBe('home');
    expect(momentOf(a, path, a.until - WALK_MAX_MS - 1)!.phase).toBe('home');
    const back = momentOf(a, path, a.until - 1)!;
    expect(back.phase).toBe('back');
    expect(Math.hypot(back.at.x - path.pts[0].x, back.at.z - path.pts[0].z)).toBeLessThan(0.1);
    expect(momentOf(a, path, a.until)).toBeNull();
    // It walks back the way it came.
    const out = momentOf(a, path, a.since + 15_000)!;
    const ms = out.next - a.since;
    const ret = momentOf(a, path, a.until - 15_000)!;
    expect(ret.at.x).toBeCloseTo(along(path.pts, 15_000 / ms).x);
    expect(ret.at.z).toBeCloseTo(along(path.pts, 15_000 / ms).z);
    // Not known when it left: it's simply home (never invented walking out).
    expect(momentOf({ ...a, since: null }, path, a.since + 1)!.phase).toBe('home');
  });
});
