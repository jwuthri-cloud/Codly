// The town around the plots: streets on every cell boundary (asphalt, lane dashes, kerbed
// sidewalks), street lamps and trees along them, parked cars, and a ring of other people's houses
// and shops around your grid (and in any gap in it), so the lots sit in a neighbourhood rather
// than on a field. All of it is decor: nothing here is clickable or saved. The layout is pure and
// seeded by position, so it is the same every time for the same plots; `TownGfx` draws it in a
// handful of merged meshes, with lamps and windows that light up at night.

import { ConeGeometry, Group, Matrix4, Mesh, MeshStandardMaterial, Quaternion, Vector3, type BufferGeometry } from 'three';
import { Builder } from '../office/gfx/builder';
import { color, lin, type RGB } from '../office/palette';
import { PITCH_X, PITCH_Z, type Plot } from './map';

/** Road width (two lanes) and sidewalk width, in metres. */
export const ROAD = 8;
export const WALK = 2.2;
/** Blocks of other people's buildings around your grid, on every side. */
export const RING = 1;

export interface Rect {
  x0: number;
  z0: number;
  x1: number;
  z1: number;
}

export interface House {
  x: number;
  z: number;
  w: number;
  d: number;
  h: number;
  /** Turned to face the street: 0 south, 1 east, 2 north, 3 west (quarter turns). */
  face: number;
  kind: 'house' | 'shop';
  wall: number;
  roof: number;
}

export interface Town {
  /** Everything the town covers (the camera's bounds, the shadow box). */
  bounds: Rect;
  roads: Rect[];
  walks: Rect[];
  dashes: Rect[];
  lamps: { x: number; z: number }[];
  trees: { x: number; z: number; s: number }[];
  houses: House[];
  cars: { x: number; z: number; along: 'x' | 'z'; c: number }[];
}

/** A small seeded random (the same town for the same plots). */
function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    return (s >>> 0) / 4294967296;
  };
}

const key = (i: number, j: number) => `${i},${j}`;

export function townOf(plots: Plot[]): Town {
  // Cells by index from the plots' centres (plots sit centred in their cell).
  const ci = (x: number) => Math.round(x / PITCH_X);
  const cj = (z: number) => Math.round(z / PITCH_Z);
  const used = new Set(plots.map((p) => key(ci(p.cx), cj(p.cz))));
  const is = plots.map((p) => ci(p.cx));
  const js = plots.map((p) => cj(p.cz));
  const [i0, i1] = plots.length ? [Math.min(...is) - RING, Math.max(...is) + RING] : [-1, 1];
  const [j0, j1] = plots.length ? [Math.min(...js) - RING, Math.max(...js) + RING] : [-1, 1];
  // Offsets so cell (i, j) is centred where its plot is (the grid's centre may be half a cell off).
  const ox = plots.length ? plots[0].cx - ci(plots[0].cx) * PITCH_X : 0;
  const oz = plots.length ? plots[0].cz - cj(plots[0].cz) * PITCH_Z : 0;
  const bx = (i: number) => ox + (i - 0.5) * PITCH_X; // west boundary of column i
  const bz = (j: number) => oz + (j - 0.5) * PITCH_Z; // north boundary of row j
  const bounds = { x0: bx(i0) - ROAD / 2, z0: bz(j0) - ROAD / 2, x1: bx(i1 + 1) + ROAD / 2, z1: bz(j1 + 1) + ROAD / 2 };
  const t: Town = { bounds, roads: [], walks: [], dashes: [], lamps: [], trees: [], houses: [], cars: [] };
  const r = ROAD / 2;

  for (let i = i0; i <= i1 + 1; i++) t.roads.push({ x0: bx(i) - r, z0: bounds.z0, x1: bx(i) + r, z1: bounds.z1 });
  for (let j = j0; j <= j1 + 1; j++) t.roads.push({ x0: bounds.x0, z0: bz(j) - r, x1: bounds.x1, z1: bz(j) + r });

  for (let i = i0; i <= i1; i++)
    for (let j = j0; j <= j1; j++) {
      const rand = rng(((i + 1000) * 73856093) ^ ((j + 1000) * 19349663));
      // The block inside its four roads, and its sidewalks around the edge.
      const b = { x0: bx(i) + r, z0: bz(j) + r, x1: bx(i + 1) - r, z1: bz(j + 1) - r };
      t.walks.push(
        { x0: b.x0, z0: b.z0, x1: b.x1, z1: b.z0 + WALK },
        { x0: b.x0, z0: b.z1 - WALK, x1: b.x1, z1: b.z1 },
        { x0: b.x0, z0: b.z0 + WALK, x1: b.x0 + WALK, z1: b.z1 - WALK },
        { x0: b.x1 - WALK, z0: b.z0 + WALK, x1: b.x1, z1: b.z1 - WALK },
      );
      // Lamps at the corners and halfway along the long sides; trees between them.
      const inset = WALK * 0.55;
      for (const [x, z] of [
        [b.x0 + inset, b.z0 + inset],
        [b.x1 - inset, b.z0 + inset],
        [b.x0 + inset, b.z1 - inset],
        [b.x1 - inset, b.z1 - inset],
        [(b.x0 + b.x1) / 2, b.z1 - inset],
        [(b.x0 + b.x1) / 2, b.z0 + inset],
      ])
        t.lamps.push({ x, z });
      for (const f of [0.25, 0.75]) {
        t.trees.push({ x: b.x0 + (b.x1 - b.x0) * f, z: b.z1 - inset, s: 0.85 + rand() * 0.35 });
        t.trees.push({ x: b.x0 + (b.x1 - b.x0) * f, z: b.z0 + inset, s: 0.85 + rand() * 0.35 });
      }
      // A car or two parked along the street south of the block.
      const n = Math.floor(rand() * 3);
      for (let k = 0; k < n; k++) t.cars.push({ x: b.x0 + 4 + rand() * (b.x1 - b.x0 - 8), z: b.z1 + 2.2, along: 'x', c: Math.floor(rand() * 6) });
      if (rand() < 0.4) t.cars.push({ x: b.x1 + 2.2, z: b.z0 + 4 + rand() * (b.z1 - b.z0 - 8), along: 'z', c: Math.floor(rand() * 6) });

      if (used.has(key(i, j))) continue;
      // Someone else's block: a shop on a corner now and then, otherwise four houses facing the
      // streets, each with a tree in its yard.
      const yard = { x0: b.x0 + WALK + 1.5, z0: b.z0 + WALK + 1.5, x1: b.x1 - WALK - 1.5, z1: b.z1 - WALK - 1.5 };
      const W = yard.x1 - yard.x0;
      const D = yard.z1 - yard.z0;
      if (rand() < 0.3) {
        t.houses.push({ x: (yard.x0 + yard.x1) / 2, z: yard.z1 - D * 0.3, w: W * 0.8, d: D * 0.5, h: 4.5 + rand() * 3, face: 0, kind: 'shop', wall: Math.floor(rand() * 8), roof: 0 });
        t.trees.push({ x: yard.x0 + 2, z: yard.z0 + 2, s: 1.2 }, { x: yard.x1 - 2, z: yard.z0 + 2, s: 1.1 });
        continue;
      }
      for (const [qx, qz, face] of [
        [0.25, 0.75, 0],
        [0.75, 0.75, 0],
        [0.25, 0.25, 2],
        [0.75, 0.25, 2],
      ]) {
        const w = 7 + rand() * 3;
        const d = 6 + rand() * 2.5;
        const x = yard.x0 + W * qx;
        const z = yard.z0 + D * qz + (face === 0 ? 1.5 : -1.5);
        t.houses.push({ x, z, w, d, h: rand() < 0.35 ? 5.6 : 3, face, kind: 'house', wall: Math.floor(rand() * 8), roof: Math.floor(rand() * 4) });
        t.trees.push({ x: x + (qx < 0.5 ? -1 : 1) * (w / 2 + 1.4), z: z + (face === 0 ? -1 : 1) * (d / 2 + 0.5), s: 0.9 + rand() * 0.4 });
      }
    }

  // Dashes down the middle of each road, gaps at the crossings.
  const cross = (v: number, edges: number[]) => edges.some((e) => Math.abs(v - e) < r + 0.5);
  const xs: number[] = [];
  const zs: number[] = [];
  for (let i = i0; i <= i1 + 1; i++) xs.push(bx(i));
  for (let j = j0; j <= j1 + 1; j++) zs.push(bz(j));
  for (const x of xs) for (let z = bounds.z0 + 1; z < bounds.z1 - 2; z += 4) if (!cross(z + 1, zs)) t.dashes.push({ x0: x - 0.08, z0: z, x1: x + 0.08, z1: z + 2 });
  for (const z of zs) for (let x = bounds.x0 + 1; x < bounds.x1 - 2; x += 4) if (!cross(x + 1, xs)) t.dashes.push({ x0: x, z0: z - 0.08, x1: x + 2, z1: z + 0.08 });
  return t;
}

// ───────────── drawing ─────────────

const WALLS = ['#e9dcc4', '#d9c3a5', '#c9d4c0', '#bfcfd9', '#e6c9b8', '#d8d1c7', '#c7b7a3', '#efe6d6'].map(lin);
const ROOFS = ['#8a4b3c', '#5d6470', '#6e5a4a', '#3f4a5a'].map(lin);
const CARS = ['#c44d3f', '#3f6fb5', '#e4c35a', '#e8e8e8', '#2f3238', '#5f9a6a'].map(lin);
const ASPHALT = lin('#4a4d52');
const WALK_C = lin('#c9c4ba');
const KERB = lin('#a9a49a');
const DASH = lin('#eeeae0');
const TRUNK = lin('#6b4f3a');
const LEAF = [lin('#6f9a58'), lin('#5c8a4c'), lin('#7ea860')];
const POLE = lin('#3a3d42');
/** A square pyramid with its corners on the axes' diagonals: scaled, a hipped roof on any box. */
const ROOF = new ConeGeometry(Math.SQRT1_2, 1, 4, 1).rotateY(Math.PI / 4).toNonIndexed();

export class TownGfx {
  readonly group = new Group();
  private geos: BufferGeometry[] = [];
  private matte = new MeshStandardMaterial({ vertexColors: true, roughness: 0.9, metalness: 0 });
  /** Lamp heads and windows: dark by day, glowing at night. */
  private lamps = new MeshStandardMaterial({ color: 0xf4efe4, roughness: 0.4 });
  private glass = new MeshStandardMaterial({ color: 0x2c3448, roughness: 0.2, metalness: 0.1 });

  constructor(readonly town: Town) {
    const ground = new Builder();
    for (const q of town.roads) ground.span(q.x0, -0.01, q.z0, q.x1, 0.0, q.z1, ASPHALT);
    for (const q of town.dashes) ground.span(q.x0, 0.0, q.z0, q.x1, 0.012, q.z1, DASH);
    for (const q of town.walks) {
      ground.span(q.x0, 0, q.z0, q.x1, 0.12, q.z1, WALK_C);
    }
    const solid = new Builder();
    const lit = new Builder();
    const win = new Builder();
    for (const l of town.lamps) {
      solid.cyl(l.x, 0.12, l.z, 0.07, 4.2, POLE, 6);
      solid.box(l.x, 4.32, l.z, 0.5, 0.12, 0.5, POLE);
      lit.box(l.x, 4.2, l.z, 0.36, 0.1, 0.36, [1, 1, 1]);
    }
    for (const tr of town.trees) {
      const s = tr.s;
      solid.cyl(tr.x, 0.12, tr.z, 0.16 * s, 1.6 * s, TRUNK, 6);
      const leaf = LEAF[Math.floor((tr.x * 7 + tr.z * 3) % 3 + 3) % 3];
      solid.blob(tr.x, 0.12 + 2.4 * s, tr.z, 1.3 * s, 1.5 * s, 1.3 * s, leaf, 0, tr.x);
    }
    for (const c of town.cars) {
      const [w, d] = c.along === 'x' ? [3.8, 1.7] : [1.7, 3.8];
      const body = CARS[c.c];
      solid.box(c.x, 0.55, c.z, w, 0.6, d, body);
      solid.box(c.x, 1.05, c.z, c.along === 'x' ? 2 : 1.5, 0.5, c.along === 'x' ? 1.5 : 2, body);
      win.box(c.x, 1.05, c.z, c.along === 'x' ? 2.02 : 1.3, 0.38, c.along === 'x' ? 1.3 : 2.02, [1, 1, 1]);
      for (const [dx, dz] of [[-1, -1], [1, -1], [-1, 1], [1, 1]]) solid.box(c.x + dx * (w / 2 - 0.7) * (c.along === 'x' ? 1 : 0.55), 0.3, c.z + dz * (d / 2 - 0.7) * (c.along === 'z' ? 1 : 0.55), c.along === 'x' ? 0.6 : 0.25, 0.6, c.along === 'x' ? 0.25 : 0.6, lin('#1f2125'));
    }
    for (const h of town.houses) this.house(h, solid, win);
    for (const [b, m] of [
      [ground, this.matte],
      [solid, this.matte],
      [lit, this.lamps],
      [win, this.glass],
    ] as const) {
      if (b.empty) continue;
      const geo = b.build();
      this.geos.push(geo);
      const mesh = new Mesh(geo, m);
      mesh.receiveShadow = true;
      mesh.castShadow = b !== ground;
      this.group.add(mesh);
    }
  }

  private house(h: House, solid: Builder, win: Builder) {
    const wall = WALLS[h.wall];
    // Along the street (x when facing north or south), and how far the front is from the centre.
    const sideways = h.face % 2 === 1;
    const [w, d] = sideways ? [h.d, h.w] : [h.w, h.d];
    solid.box(h.x, h.h / 2, h.z, w, h.h, d, wall);
    // The front: +z when facing south, -z north, +x east, -x west.
    const fx = h.face === 1 ? 1 : h.face === 3 ? -1 : 0;
    const fz = h.face === 0 ? 1 : h.face === 2 ? -1 : 0;
    const front = (along: number, y: number, ww: number, hh: number, depth = 0.08): [number, number, number, number, number, number] => {
      const x = h.x + fx * (w / 2 + depth / 2) + (sideways ? 0 : along);
      const z = h.z + fz * (d / 2 + depth / 2) + (sideways ? along : 0);
      return [x, y, z, sideways ? depth : ww, hh, sideways ? ww : depth];
    };
    const len = sideways ? d : w;
    if (h.kind === 'shop') {
      // A shop: a flat roof with a parapet, a big window and an awning.
      solid.box(h.x, h.h + 0.25, h.z, w + 0.2, 0.5, d + 0.2, lin('#8e8a84'));
      win.box(...front(0, 1.5, len * 0.7, 1.9), [1, 1, 1]);
      const [ax, , az, aw, , ad] = front(0, 0, len * 0.8, 0, 1.1);
      solid.box(ax, 2.75, az, aw, 0.15, ad, CARS[(h.wall + 2) % CARS.length]);
      return;
    }
    // A house: a hipped roof, a door and windows on each storey.
    const rh = Math.min(w, d) * 0.42;
    solid.add(ROOF, ROOFS[h.roof], new Matrix4().compose(new Vector3(h.x, h.h + rh / 2, h.z), new Quaternion(), new Vector3(w + 0.6, rh, d + 0.6)), true);
    solid.box(...front(0, 1, 1, 2), lin('#6b4a36'));
    for (let y = 1.5; y < h.h; y += 2.6) for (const a of [-len * 0.3, len * 0.3]) win.box(...front(a, y, 1.2, 1.1), [1, 1, 1]);
  }

  /** Lamps and windows glow at night (the lamp's colour from the theme). */
  setNight(night: boolean, lamp: RGB) {
    this.lamps.emissive.copy(color(lamp)).multiplyScalar(night ? 2.2 : 0);
    this.glass.color.copy(color(night ? lin('#ffd9a0') : lin('#2c3448')));
    this.glass.emissive.copy(color(lamp)).multiplyScalar(night ? 0.9 : 0);
  }

  dispose() {
    for (const g of this.geos) g.dispose();
    this.matte.dispose();
    this.lamps.dispose();
    this.glass.dispose();
  }
}
