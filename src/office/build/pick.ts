// What build mode's cursor is over: the wall the ray from the camera hits first (as tall as it
// stands now: cut away, walls down), the side of it facing the camera, the door or window there,
// and the tile and room on the floor. Pure: no three.js.

import type { Opening } from './grid';
import type { OfficeLayout, Wall } from '../layout';

export interface Ray {
  x: number;
  z: number;
  o: [number, number, number];
  d: [number, number, number];
}

export interface WallHit {
  wall: Wall;
  /** Where the ray meets it, on the floor plan (on the wall's line). */
  x: number;
  z: number;
  /** How far along the wall, from its start. */
  at: number;
  /** The side facing the camera: 1 is the +z (or +x) side. */
  side: 1 | -1;
}

const HALF = 0.14;

/** The first wall the ray hits (each `height(id)` tall), or one the floor point is right next to. */
export function wallUnder(L: OfficeLayout, ray: Ray, height: (id: string) => number): WallHit | null {
  let best: { w: Wall; t: number; x: number; z: number } | null = null;
  const [ox, oy, oz] = ray.o;
  const [dx, dy, dz] = ray.d;
  for (const w of L.walls) {
    const horiz = w.z0 === w.z1;
    const min = [horiz ? w.x0 : w.x0 - HALF, 0, horiz ? w.z0 - HALF : w.z0];
    const max = [horiz ? w.x1 : w.x0 + HALF, Math.max(0.05, height(w.id)), horiz ? w.z0 + HALF : w.z1];
    let t0 = 0;
    let t1 = Infinity;
    const o = [ox, oy, oz];
    const d = [dx, dy, dz];
    let hit = true;
    for (let i = 0; i < 3 && hit; i++) {
      if (Math.abs(d[i]) < 1e-9) {
        if (o[i] < min[i] || o[i] > max[i]) hit = false;
        continue;
      }
      let a = (min[i] - o[i]) / d[i];
      let b = (max[i] - o[i]) / d[i];
      if (a > b) [a, b] = [b, a];
      t0 = Math.max(t0, a);
      t1 = Math.min(t1, b);
      if (t0 > t1) hit = false;
    }
    if (hit && (!best || t0 < best.t)) best = { w, t: t0, x: ox + dx * t0, z: oz + dz * t0 };
  }
  if (!best) {
    // Close to a wall on the floor (a low wall seen from above).
    let bd = 0.3;
    for (const w of L.walls) {
      const horiz = w.z0 === w.z1;
      const along = horiz ? Math.min(w.x1, Math.max(w.x0, ray.x)) : Math.min(w.z1, Math.max(w.z0, ray.z));
      const d = horiz ? Math.hypot(ray.x - along, ray.z - w.z0) : Math.hypot(ray.x - w.x0, ray.z - along);
      if (d < bd) (bd = d), (best = { w, t: 0, x: horiz ? along : w.x0, z: horiz ? w.z0 : along });
    }
  }
  if (!best) return null;
  const { w } = best;
  const horiz = w.z0 === w.z1;
  const x = horiz ? Math.min(w.x1, Math.max(w.x0, best.x)) : w.x0;
  const z = horiz ? w.z0 : Math.min(w.z1, Math.max(w.z0, best.z));
  const side = (horiz ? oz > w.z0 : ox > w.x0) ? 1 : -1;
  return { wall: w, x, z, at: horiz ? x - w.x0 : z - w.z0, side };
}

/** The door or window on a wall at `at` along it. */
export function openingOn(list: Opening[], wall: string, at: number): Opening | null {
  return list.find((o) => o.wall === wall && Math.abs(o.at - at) <= o.width / 2 + 0.05) ?? null;
}

/** The tile under a floor point (−1 off the lot), and its room (−1 outside). */
export function tileUnder(L: OfficeLayout, x: number, z: number): { tile: number; room: number } {
  if (x < 0 || z < 0 || x >= L.w || z >= L.d) return { tile: -1, room: -1 };
  const tile = Math.floor(z / 0.5) * L.tiles.cols + Math.floor(x / 0.5);
  return { tile, room: L.roomOf[tile] };
}
