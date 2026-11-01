// Agents at home, on the map (SIMS.md "Shifts, home…", PROGRESSION §5). Each agent at home
// (the core's `away_until` still ahead) lives in one of the town's houses, picked from its id
// among those nearest its office. For about a minute after it left it walks there from its
// office's front door along the sidewalks (to the block's corner, then down the streets); while
// it is home that house's windows are lit with a tag over it ("🏠 Ada · back in 6 min"); in its
// last minute away it walks back the same way. Only real stays: `since` and `until` come from
// the core (an agent whose leaving time isn't known is simply home, it never walks out).
//
// The route and the timing are pure (tested); `HomeGfx` draws the walkers as two instanced
// meshes (a body in the agent's colour and a head) and the lit windows as one merged mesh.

import { CapsuleGeometry, Color, InstancedMesh, Mesh, MeshStandardMaterial, Object3D, SphereGeometry, type BufferGeometry, Group } from 'three';
import { Builder } from '../office/gfx/builder';
import { color, identity, type RGB } from '../office/palette';
import type { Plot } from './map';
import { doorOf, houseWindows, WALK, type House, type Rect, type Town } from './town';

export interface Pt {
  x: number;
  z: number;
}

/** An agent at home (the core's `AwayAgent`, or one of the open office's agents). */
export interface Away {
  id: string;
  name: string;
  color: number;
  /** When it left (ms), if known. */
  since: number | null;
  until: number;
}

/** How many of the houses nearest its office an agent's home is picked from. */
export const NEAR = 6;
/** Walking pace (m/s), and the bounds of a walk: about a minute, never more than a third of the stay. */
export const PACE = 1.4;
export const WALK_MIN_MS = 20_000;
export const WALK_MAX_MS = 60_000;
/** The walking line on a sidewalk, from the block's edge (the kerb): clear of the lamps and the
 *  trees, which stand ~1.2 m in. */
export const LANE = WALK - 0.55;

/** A stable 32-bit hash of a string (FNV-1a). */
export function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Where an office's people come and go: the middle of its plot's south edge (the street side). */
export const officeFront = (p: Plot): Pt => ({ x: p.cx, z: p.z1 });

const inside = (r: Rect, p: Pt, m = 0.01) => p.x >= r.x0 - m && p.x <= r.x1 + m && p.z >= r.z0 - m && p.z <= r.z1 + m;
const dist = (a: Pt, b: Pt) => Math.hypot(a.x - b.x, a.z - b.z);

/** The house an agent goes home to (an index into `town.houses`), or -1 if the town has none:
 *  one of the `NEAR` homes nearest its office's front, picked from its id. */
export function homeOf(id: string, town: Town, plot: Plot): number {
  const front = officeFront(plot);
  const near = town.houses
    .map((h, i) => ({ i, d: dist(doorOf(h), front), h }))
    .filter((x) => x.h.kind === 'house')
    .sort((a, b) => a.d - b.d || a.i - b.i)
    .slice(0, NEAR);
  return near.length ? near[hash(id) % near.length].i : -1;
}

/** The walk from an office's front door to a house's, on the street grid: down to the block's
 *  south sidewalk, along it to a corner, up or down the cross street's sidewalk to the house's
 *  street, along that to its gate, and up its garden path. Crossings are at the corners. Every
 *  block in a column shares its west and east edges (and every block in a row its north and
 *  south ones), so these lines run on sidewalks all the way, never through a plot or a house. */
export function streetPath(town: Town, plot: Plot, house: House): Pt[] {
  const a = officeFront(plot);
  const door = doorOf(house);
  const ba = town.blocks.find((b) => inside(b, a));
  const bh = town.blocks.find((b) => inside(b, door));
  if (!ba || !bh) return [a, door];
  const za = ba.z1 - LANE;
  // The house's street: south of its block if it faces south (or sideways), north if it faces north.
  const zh = house.face === 2 ? bh.z0 + LANE : bh.z1 - LANE;
  const pts: Pt[] = [a, { x: a.x, z: za }];
  if (Math.abs(za - zh) > 0.01) {
    // To the corner of the office's block on the house's side, then along the cross street.
    const corners = [ba.x0 + LANE, ba.x1 - LANE];
    const xv = corners.sort((p, q) => Math.abs(a.x - p) + Math.abs(p - door.x) - (Math.abs(a.x - q) + Math.abs(q - door.x)))[0];
    pts.push({ x: xv, z: za }, { x: xv, z: zh });
  }
  pts.push({ x: door.x, z: zh }, door);
  // Drop repeats (a door straight across from the corner, say).
  return pts.filter((p, i) => i === 0 || dist(p, pts[i - 1]) > 0.01);
}

export function pathLength(path: Pt[]): number {
  let n = 0;
  for (let i = 1; i < path.length; i++) n += dist(path[i - 1], path[i]);
  return n;
}

/** The point `f` (0..1) of the way along a path, and the way it faces there (radians about y). */
export function along(path: Pt[], f: number): Pt & { heading: number } {
  const total = pathLength(path);
  let left = Math.min(1, Math.max(0, f)) * total;
  for (let i = 1; i < path.length; i++) {
    const a = path[i - 1];
    const b = path[i];
    const l = dist(a, b);
    if (left <= l || i === path.length - 1) {
      const k = l > 0 ? Math.min(1, left / l) : 1;
      return { x: a.x + (b.x - a.x) * k, z: a.z + (b.z - a.z) * k, heading: Math.atan2(b.x - a.x, b.z - a.z) };
    }
    left -= l;
  }
  return { ...path[0], heading: 0 };
}

/** How long the walk takes each way (ms): its length at a walking pace, about a minute at most,
 *  and never more than a third of the stay (a short stay leaves time at home). */
export function walkMs(length: number, a: Pick<Away, 'since' | 'until'>): number {
  const ms = Math.min(WALK_MAX_MS, Math.max(WALK_MIN_MS, (length / PACE) * 1000));
  return a.since == null ? ms : Math.min(ms, (a.until - a.since) / 3);
}

export type Phase = 'out' | 'home' | 'back';

export interface Moment {
  phase: Phase;
  /** Where it is (walking), on the ground. */
  at: Pt & { heading: number };
  /** When this moment ends (ms): the next phase, or back at the office. */
  next: number;
}

/** Where an agent at home is at `now` (ms), or null when it isn't away (yet, or any more). */
export function momentOf(a: Away, path: Path, now: number): Moment | null {
  if (now >= a.until || (a.since != null && now < a.since)) return null;
  const ms = walkMs(path.length, a);
  const back = a.until - ms;
  if (a.since != null && now < a.since + ms) return { phase: 'out', at: along(path.pts, (now - a.since) / ms), next: a.since + ms };
  if (now < back) return { phase: 'home', at: { ...path.pts[path.pts.length - 1], heading: 0 }, next: back };
  const at = along(path.pts, 1 - (now - back) / ms);
  return { phase: 'back', at: { ...at, heading: at.heading + Math.PI }, next: a.until };
}

export interface Path {
  pts: Pt[];
  length: number;
}

export const pathOf = (pts: Pt[]): Path => ({ pts, length: pathLength(pts) });

/** The tag's line for an agent at home: "🏠 Ada · back in 6 min" (as the office's own tag). */
export const homeLine = (name: string, until: number, now: number): string => `🏠 ${name} · back in ${Math.max(1, Math.ceil((until - now) / 60_000))} min`;

// ───────────── drawing ─────────────

const CAP = 64;
/** Figures are drawn a little larger than life, so they read from the map's height. */
const SCALE = 1.35;

export class HomeGfx {
  readonly group = new Group();
  private body: InstancedMesh;
  private head: InstancedMesh;
  private bodyGeo: BufferGeometry = new CapsuleGeometry(0.26, 0.8, 3, 8).translate(0, 0.66, 0);
  private headGeo: BufferGeometry = new SphereGeometry(0.22, 10, 8).translate(0, 1.5, 0);
  private bodyMat = new MeshStandardMaterial({ roughness: 0.7, metalness: 0 });
  private headMat = new MeshStandardMaterial({ color: new Color().setRGB(0.62, 0.42, 0.3), roughness: 0.8, metalness: 0 });
  /** The windows of the houses someone is home in, lit. */
  private lit: Mesh | null = null;
  private litKey = '';
  private litMat = new MeshStandardMaterial({ roughness: 0.3, metalness: 0 });
  private o = new Object3D();
  private c = new Color();

  constructor() {
    this.body = new InstancedMesh(this.bodyGeo, this.bodyMat, CAP);
    this.head = new InstancedMesh(this.headGeo, this.headMat, CAP);
    for (const m of [this.body, this.head]) {
      m.count = 0;
      // Instances roam the whole map: the geometry's own bounds would cull them.
      m.frustumCulled = false;
      this.group.add(m);
    }
  }

  /** The walkers now: where each stands (on the sidewalk, 0.12 up) and its colour. */
  setWalkers(walkers: { at: Pt & { heading: number }; color: number; bob: number }[], theme: 'light' | 'dark') {
    const n = Math.min(CAP, walkers.length);
    for (let i = 0; i < n; i++) {
      const w = walkers[i];
      this.o.position.set(w.at.x, 0.12 + Math.abs(Math.sin(w.bob)) * 0.08, w.at.z);
      this.o.rotation.set(0, w.at.heading, Math.sin(w.bob) * 0.06);
      this.o.scale.setScalar(SCALE);
      this.o.updateMatrix();
      this.body.setMatrixAt(i, this.o.matrix);
      this.head.setMatrixAt(i, this.o.matrix);
      const rgb: RGB = identity(w.color, theme).shirt;
      this.body.setColorAt(i, this.c.setRGB(rgb[0], rgb[1], rgb[2]));
    }
    this.body.count = this.head.count = n;
    this.body.instanceMatrix.needsUpdate = this.head.instanceMatrix.needsUpdate = true;
    if (this.body.instanceColor) this.body.instanceColor.needsUpdate = true;
  }

  /** Light the windows of these houses (a merged mesh, rebuilt only when the set changes). */
  setLit(houses: House[], key: string, night: boolean, lamp: RGB) {
    // Brighter than the neighbours' windows at night (the town lights them all then).
    this.litMat.color.copy(color(night ? [1, 0.8, 0.5] : [1, 0.86, 0.6]));
    this.litMat.emissive.copy(color(lamp)).multiplyScalar(night ? 2.6 : 0.8);
    if (key === this.litKey) return;
    this.litKey = key;
    if (this.lit) {
      this.group.remove(this.lit);
      this.lit.geometry.dispose();
      this.lit = null;
    }
    if (!houses.length) return;
    const b = new Builder();
    // A little proud of the town's own (dark) panes and a touch larger, so they cover them
    // without fighting for the same depth.
    for (const h of houses) for (const [x, y, z, w, hh, d] of houseWindows(h, 0.16)) b.box(x, y, z, w + 0.06, hh + 0.06, d, [1, 1, 1]);
    this.lit = new Mesh(b.build(), this.litMat);
    this.group.add(this.lit);
  }

  dispose() {
    this.bodyGeo.dispose();
    this.headGeo.dispose();
    this.bodyMat.dispose();
    this.headMat.dispose();
    this.litMat.dispose();
    this.lit?.geometry.dispose();
    this.body.dispose();
    this.head.dispose();
  }
}
