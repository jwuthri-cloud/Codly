// Characters: charming low-poly chibi people built procedurally from the agent's seed — skin,
// hair style and color, clothes in the agent's identity color, a small provider badge on the
// chest — with role silhouettes: the Lead wears a blazer and stands a bit taller, a reviewer
// wears a cardigan and round glasses, workers wear a tee with a hood or headphones, a helper
// ("intern", a subagent) is small and wears a cap in its parent's color.
//
// Animation is procedural: a few joints (hips, torso lean, head, shoulders, elbows, legs), a walk
// cycle while moving, pose blends on change, and each pose's own motion as continuous curves
// (gfx/motion.ts). While the office is live (on screen, motion allowed) every
// character also breathes, blinks, shifts its weight and glances around (or at what the engine
// points it to: a ball, the TV, someone walking by). Otherwise poses tick in steps on a shared
// clock and a character standing or sitting still costs nothing.

import {
  BufferAttribute,
  BufferGeometry,
  ConeGeometry,
  Euler,
  LatheGeometry,
  Matrix4,
  Group,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  Object3D,
  Quaternion,
  SphereGeometry,
  Sprite,
  SpriteMaterial,
  TorusGeometry,
  Vector2,
  Vector3,
  type Texture,
} from 'three';
import type { Provider } from '../../lib/gen/Provider';
import type { Role } from '../../lib/gen/Role';
import type { Emote, Pose, Prop } from '../behavior';
import { addLife, gazeWeight, glancing, J, type Joints, loopPeriod, type PlayCue, REACT_S, reactPose, type ReactKind, SIT_POSES, targets } from './motion';
import { HAIR, HAIR_WEIGHTS, identity, lin, mix, PANTS, PROVIDER, SKIN, type RGB } from '../palette';
import { hash, rng } from '../text';
import { traitsOf } from '../traits';
import { Builder } from './builder';
import type { EmoteKind } from './textures';

// ───────────── look ─────────────

export interface Look {
  skin: RGB;
  hair: RGB;
  style: number;
  shirt: RGB;
  shirtDark: RGB;
  pants: RGB;
  shoes: RGB;
  role: Role;
  provider: Provider;
  accessory: 'none' | 'hood' | 'headphones';
  intern: boolean;
  scale: number;
  blush: boolean;
  glasses: boolean;
  /** The iris, low in the eye. */
  eye: RGB;
  lashes: boolean;
  /** The agent's `look_seed` this was made from. */
  seed: string;
}

/** Hairstyles `lookFor` can draw (0 … LOOK_STYLES − 1). */
export const LOOK_STYLES = 11;
export type LookPick = 'style' | 'hair' | 'skin' | 'acc' | 'glasses';

/** The person's picks (the look editor) after the `#` of a `look_seed`; the rest is the seed. */
export function lookPicks(seed: string): { base: string; picks: URLSearchParams } {
  const i = seed.indexOf('#');
  return i < 0 ? { base: seed, picks: new URLSearchParams() } : { base: seed.slice(0, i), picks: new URLSearchParams(seed.slice(i + 1)) };
}

/** `seed` with one pick changed (null drops it, back to what the seed draws). */
export function withPick(seed: string, key: LookPick, value: string | null): string {
  const { base, picks } = lookPicks(seed);
  if (value == null) picks.delete(key);
  else picks.set(key, value);
  const rest = picks.toString();
  return rest ? `${base}#${rest}` : base;
}

function pick<T>(r: () => number, list: T[], weights?: number[]): T {
  if (!weights) return list[Math.floor(r() * list.length)];
  const total = weights.reduce((a, b) => a + b, 0);
  let x = r() * total;
  for (let i = 0; i < list.length; i++) {
    x -= weights[i];
    if (x <= 0) return list[i];
  }
  return list[list.length - 1];
}

export function lookFor(seed: string, color: number, role: Role, provider: Provider, intern = false): Look {
  const { base, picks } = lookPicks(seed);
  const r = rng(hash(base || 'agent'));
  const id = identity(color, 'light');
  const skin = lin(pick(r, SKIN));
  const hair = lin(pick(r, HAIR, HAIR_WEIGHTS));
  const style = Math.floor(r() * 8);
  const pants = lin(pick(r, PANTS));
  const acc = r();
  const look: Look = {
    skin,
    hair,
    style,
    shirt: id.shirt,
    shirtDark: id.shirtDark,
    pants,
    shoes: lin(r() < 0.5 ? '#2b2b30' : r() < 0.5 ? '#f1ede6' : '#7a4b33'),
    role,
    provider,
    accessory: role === 'worker' || role === 'solo' ? (acc < 0.45 ? 'hood' : acc < 0.75 ? 'headphones' : 'none') : 'none',
    intern,
    scale: (intern ? 0.72 : role === 'lead' ? 1.06 : 1) * 1.2,
    blush: r() < 0.6,
    glasses: role === 'reviewer',
    eye: lin(EYES[0]),
    lashes: false,
    seed,
  };
  // Later styles (quiff, afro, undercut) draw last, so everyone else keeps the look they had.
  const extra = r();
  if (extra < 0.3) look.style = 8 + Math.floor((extra / 0.3) * 3);
  look.eye = lin(pick(r, EYES, [10, 6, 3, 3, 4]));
  look.lashes = r() < 0.45;
  // The person's picks win over the dice.
  const chosen = (k: LookPick, n: number) => {
    const v = Number(picks.get(k));
    return picks.has(k) && Number.isInteger(v) && v >= 0 && v < n ? v : null;
  };
  const myStyle = chosen('style', LOOK_STYLES);
  if (myStyle != null) look.style = myStyle;
  const myHair = chosen('hair', HAIR.length);
  if (myHair != null) look.hair = lin(HAIR[myHair]);
  const mySkin = chosen('skin', SKIN.length);
  if (mySkin != null) look.skin = lin(SKIN[mySkin]);
  const myAcc = picks.get('acc');
  if ((role === 'worker' || role === 'solo') && (myAcc === 'hood' || myAcc === 'headphones' || myAcc === 'none')) look.accessory = myAcc;
  if (picks.has('glasses')) look.glasses = picks.get('glasses') === '1';
  return look;
}

// ───────────── rig dimensions (scale 1) ─────────────

const HIP = 0.36;
const SHOULDER_Y = 0.285;
const SHOULDER_X = 0.175;
const NECK = 0.32;
const HEAD_R = 0.27;
const HEAD_Y = 0.24;
/** The head is a soft, slightly wide ellipsoid (a mochi, not a ball). */
const HX = 0.282;
const HY = 0.258;
const HZ = 0.255;
const UPPER = 0.14;
const FORE = 0.15;
const LEG_X = 0.085;
/** Leg segments: thigh (hip → knee) and shin (knee → ankle); the shoe hangs at the ankle. */
const THIGH = 0.17;
const SHIN = HIP - THIGH - 0.03;

/** Joint vector (gfx/motion.ts): y, lean, headPitch, headYaw, headRoll, armLx, armLz, elbowL, armRx, armRz, elbowR, legL, legR, twist */

const WHITE: RGB = [1, 1, 1];
const DARK_EYE: RGB = lin('#1f1820');
const MOUTH: RGB = lin('#6e2f2b');
const TONGUE: RGB = lin('#e77b74');
const CREAM: RGB = lin('#f6f2ea');
const EYES = ['#5a3a26', '#6b4a2a', '#3d6a8c', '#4d6e3c', '#2e2a2a'];
const TAU = Math.PI * 2;
const PI = Math.PI;

/** The faces a head can wear; `Character.face` indexes this list. */
const FACES = ['open', 'closed', 'happy', 'worried', 'curious'] as const;
type Face = (typeof FACES)[number];

// ───────────── sculpting: shapes that follow a surface ─────────────

/** A point on a surface (x, y in, z and the outward normal out); faces and clothes are painted on these. */
type Surface = (x: number, y: number) => [number, number, number, number, number, number];

/** The front of an ellipsoid. */
function ellipsoid(cx: number, cy: number, cz: number, rx: number, ry: number, rz: number): Surface {
  return (x, y) => {
    const u = (x - cx) / rx;
    const v = (y - cy) / ry;
    const z = cz + rz * Math.sqrt(Math.max(1e-4, 1 - u * u - v * v));
    const n = new Vector3((x - cx) / (rx * rx), (y - cy) / (ry * ry), (z - cz) / (rz * rz)).normalize();
    return [x, y, z, n.x, n.y, n.z];
  };
}

/** The front of a lathe (radius `r(y)`, squashed by `k` front to back). */
function lathed(r: (y: number) => number, k: number): Surface {
  return (x, y) => {
    const R = r(y);
    const z = k * Math.sqrt(Math.max(1e-4, R * R - x * x));
    const n = new Vector3(x, 0, z / (k * k)).normalize();
    return [x, y, z, n.x, n.y, n.z];
  };
}

const I4 = new Matrix4();

/** Triangles (already placed) into a builder. */
function tris(b: Builder, pos: number[], nor: number[], color: RGB) {
  const g = new BufferGeometry();
  g.setAttribute('position', new BufferAttribute(new Float32Array(pos), 3));
  g.setAttribute('normal', new BufferAttribute(new Float32Array(nor), 3));
  b.add(g, color, I4);
  g.dispose();
}

/** A filled convex shape painted on a surface, `lift` above it: rings that bend with it. */
function paint(b: Builder, s: Surface, pts: [number, number][], color: RGB, lift = 0.002, rings = 3) {
  let area = 0;
  for (let i = 0; i < pts.length; i++) {
    const [x0, y0] = pts[i];
    const [x1, y1] = pts[(i + 1) % pts.length];
    area += x0 * y1 - x1 * y0;
  }
  if (area < 0) pts = pts.slice().reverse();
  // Short edges and enough rings, so a big shape bends with the surface instead of cutting into it.
  const dense: [number, number][] = [];
  let size = 0;
  for (let i = 0; i < pts.length; i++) {
    const [x0, y0] = pts[i];
    const [x1, y1] = pts[(i + 1) % pts.length];
    const n = Math.max(1, Math.ceil(Math.hypot(x1 - x0, y1 - y0) / 0.01));
    for (let k = 0; k < n; k++) dense.push([x0 + ((x1 - x0) * k) / n, y0 + ((y1 - y0) * k) / n]);
    size = Math.max(size, Math.hypot(x1 - x0, y1 - y0));
  }
  pts = dense;
  rings = Math.max(rings, Math.ceil(size / 0.02));
  let cx = 0;
  let cy = 0;
  for (const [x, y] of pts) {
    cx += x / pts.length;
    cy += y / pts.length;
  }
  const at = (i: number, k: number) => {
    const [x, y] = pts[i % pts.length];
    const f = k / rings;
    const p = s(cx + (x - cx) * f, cy + (y - cy) * f);
    return [p[0] + p[3] * lift, p[1] + p[4] * lift, p[2] + p[5] * lift, p[3], p[4], p[5]];
  };
  const pos: number[] = [];
  const nor: number[] = [];
  const v = (q: number[]) => {
    pos.push(q[0], q[1], q[2]);
    nor.push(q[3], q[4], q[5]);
  };
  for (let k = 0; k < rings; k++)
    for (let i = 0; i < pts.length; i++) {
      const a = at(i, k);
      const c = at(i + 1, k + 1);
      v(a), v(at(i, k + 1)), v(c);
      if (k) v(a), v(c), v(at(i + 1, k));
    }
  tris(b, pos, nor, color);
}

/** An ellipse's outline points. */
function oval(cx: number, cy: number, rx: number, ry: number, rot = 0, n = 18): [number, number][] {
  const out: [number, number][] = [];
  const c = Math.cos(rot);
  const s = Math.sin(rot);
  for (let i = 0; i < n; i++) {
    const a = (i / n) * TAU;
    const x = Math.cos(a) * rx;
    const y = Math.sin(a) * ry;
    out.push([cx + x * c - y * s, cy + x * s + y * c]);
  }
  return out;
}

/** A stroke along a path painted on a surface (brows, smiles, lashes), tapered at both ends when `taper`. */
function stroke(b: Builder, s: Surface, path: [number, number][], width: number, color: RGB, lift = 0.003, taper = 0) {
  if (path.length === 2) path = curve(path[0][0], path[1][0], (_x, t) => path[0][1] + (path[1][1] - path[0][1]) * t, Math.ceil(Math.hypot(path[1][0] - path[0][0], path[1][1] - path[0][1]) / 0.01));
  const pos: number[] = [];
  const nor: number[] = [];
  const side: number[][] = [];
  for (let i = 0; i < path.length; i++) {
    const [x0, y0] = path[Math.max(0, i - 1)];
    const [x1, y1] = path[Math.min(path.length - 1, i + 1)];
    const d = Math.hypot(x1 - x0, y1 - y0) || 1;
    const t = i / (path.length - 1);
    const w = (width / 2) * (1 - taper + taper * Math.sqrt(Math.sin(PI * t)));
    const nx = (-(y1 - y0) / d) * w;
    const ny = ((x1 - x0) / d) * w;
    for (const sg of [1, -1]) {
      const p = s(path[i][0] + nx * sg, path[i][1] + ny * sg);
      side.push([p[0] + p[3] * lift, p[1] + p[4] * lift, p[2] + p[5] * lift, p[3], p[4], p[5]]);
    }
  }
  const v = (q: number[]) => {
    pos.push(q[0], q[1], q[2]);
    nor.push(q[3], q[4], q[5]);
  };
  for (let i = 0; i < path.length - 1; i++) {
    const [l0, r0, l1, r1] = [side[i * 2], side[i * 2 + 1], side[i * 2 + 2], side[i * 2 + 3]];
    v(l0), v(r0), v(r1);
    v(l0), v(r1), v(l1);
  }
  tris(b, pos, nor, color);
}

/** Points along y = f(x) for x in [x0, x1]. */
function curve(x0: number, x1: number, f: (x: number, t: number) => number, n = 10): [number, number][] {
  const out: [number, number][] = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    const x = x0 + (x1 - x0) * t;
    out.push([x, f(x, t)]);
  }
  return out;
}

/** A solid of revolution around y (profile [radius, y] bottom to top), squashed front to back by `k`. */
function lathe(b: Builder, profile: [number, number][], color: RGB, k = 1, x = 0, y = 0, z = 0, seg = 28) {
  const g = new LatheGeometry(
    profile.map(([r, py]) => new Vector2(Math.max(1e-4, r), py)),
    seg,
  ).toNonIndexed();
  b.add(g, color, new Matrix4().compose(new Vector3(x, y, z), new Quaternion(), new Vector3(1, 1, k)));
  g.dispose();
}

/** A capsule hanging down from (x, y, z): radius r, `len` from the first cap's center to the last. */
function limb(b: Builder, y: number, r: number, len: number, color: RGB, rEnd = r) {
  // Tapered: a lathe of two half-circles joined by a straight side.
  const prof: [number, number][] = [];
  const n = 6;
  for (let i = 0; i <= n; i++) {
    const a = -PI / 2 + (i / n) * (PI / 2);
    prof.push([Math.cos(a) * rEnd, y - len + Math.sin(a) * rEnd]);
  }
  for (let i = 0; i <= n; i++) {
    const a = (i / n) * (PI / 2);
    prof.push([Math.cos(a) * r, y + Math.sin(a) * r]);
  }
  lathe(b, prof, color, 1, 0, 0, 0, 16);
}

/** A ring (cuffs, hems, collars) lying flat at height y. */
function ring(b: Builder, y: number, r: number, tube: number, color: RGB, k = 1, z = 0, tilt = 0) {
  const g = new TorusGeometry(r, tube, 8, 28).toNonIndexed();
  const m = new Matrix4().compose(new Vector3(0, y, z), new Quaternion().setFromEuler(new Euler(PI / 2 + tilt, 0, 0)), new Vector3(1, k, 1));
  b.add(g, color, m);
  g.dispose();
}

/** A hairline: how far down (θ, 0 the crown … π the chin) the hair reaches, all around (φ, 0 the face). */
type Line = (phi: number) => number;
/** θ at the front, the sides and the back, smooth in between. */
const hairline =
  (front: number, side: number, back: number, burns = 1): Line =>
  (phi) => {
    const c = Math.cos(phi);
    // Around the ear: a sideburn in front of it, the hair lifting over it, a soft wave on the fringe.
    const a = Math.abs(Math.atan2(Math.sin(phi), c));
    const ear = burns * (0.1 * PI * Math.exp(-((a - 1.22) ** 2) / 0.012) - 0.05 * PI * Math.exp(-((a - 1.62) ** 2) / 0.03));
    const wave = 0.018 * PI * Math.max(0, c) ** 3 * Math.cos(phi * 11);
    return side + ((front - back) / 2) * c + ((front + back) / 2 - side) * c * c + Math.min(ear, 0.9 * PI - side) + wave;
  };

/**
 * Hair as one sculpted shell over the head, down to its hairline (so there is a clean edge at the
 * forehead, the temples and the nape), puffed out where the style has volume. The edge folds in to
 * the scalp so it reads as a thickness, not a sheet.
 */
function shell(b: Builder, line: Line, t: number, color: RGB, puff: (theta: number, phi: number) => number = () => 0, w = 48, h = 14, gloss = 0.16) {
  const cols = w + 1;
  const rows = h + 2;
  const pos = new Float32Array(cols * rows * 3);
  for (let i = 0; i < cols; i++) {
    const phi = PI + (i / w) * TAU;
    const L = line(phi);
    for (let j = 0; j < rows; j++) {
      const lip = j === rows - 1;
      const th = L * (Math.min(j, h) / h);
      const k = lip ? 0.985 : 1 + puff(th, phi);
      const d = lip ? 0 : t;
      const o = (i * rows + j) * 3;
      pos[o] = (HX + d) * k * Math.sin(th) * Math.sin(phi);
      pos[o + 1] = HEAD_Y + (HY + d) * k * Math.cos(th);
      pos[o + 2] = (HZ + d) * k * Math.sin(th) * Math.cos(phi);
    }
  }
  const idx: number[] = [];
  for (let i = 0; i < w; i++)
    for (let j = 0; j < rows - 1; j++) {
      const a = i * rows + j;
      const c = (i + 1) * rows + j;
      idx.push(a, a + 1, c + 1, a, c + 1, c);
    }
  // Shade: faint strands, a glossy band around the crown, a little darker towards the edge.
  const shade = new Float32Array(cols * rows * 3);
  for (let i = 0; i < cols; i++) {
    const phi = PI + (i / w) * TAU;
    const L = line(phi);
    for (let j = 0; j < rows; j++) {
      const th = L * (Math.min(j, h) / h);
      const u = th / L;
      const strand = 0.035 * Math.sin(phi * 26 + th * 4);
      const sheen = gloss * Math.exp(-((th - 0.26 * PI) ** 2) / 0.012) * (0.55 + 0.45 * Math.cos(phi));
      shade[(i * rows + j) * 3] = 1 + sheen + strand - 0.14 * u * u - (j === rows - 1 ? 0.2 : 0);
    }
  }
  const g = new BufferGeometry();
  g.setAttribute('position', new BufferAttribute(pos, 3));
  g.setAttribute('color', new BufferAttribute(shade, 3));
  g.setIndex(idx);
  g.computeVertexNormals();
  const flat = g.toNonIndexed();
  b.add(flat, color, I4);
  g.dispose();
  flat.dispose();
}

/** Bumps for curly textures (0 … 1). */
const bumps = (th: number, phi: number, n: number) => 0.5 + 0.5 * Math.sin(th * n * 1.7) * Math.sin(phi * n);
const smooth = (a: number, b: number, x: number) => {
  const t = clamp((x - a) / (b - a), 0, 1);
  return t * t * (3 - 2 * t);
};
/** Lower on one side of the forehead (a side-swept fringe): `dir` +1 its left, −1 its right. */
const sweep = (phi: number, dir: number, amt: number) => amt * Math.max(0, Math.cos(phi)) ** 2 * smooth(-0.6, 0.9, dir * Math.sin(phi));

/** A cone from `at` towards `dir` (spikes, strands). */
function spike(b: Builder, at: Vector3, dir: Vector3, len: number, r: number, color: RGB) {
  const g = new ConeGeometry(1, 1, 8, 1).toNonIndexed();
  const q = new Quaternion().setFromUnitVectors(new Vector3(0, 1, 0), dir.clone().normalize());
  b.add(g, color, new Matrix4().compose(at.clone().addScaledVector(dir.clone().normalize(), len / 2), q, new Vector3(r, len, r)));
  g.dispose();
}

/** A point on the hair shell's surface (for things that sit on it). */
function onHead(th: number, phi: number, k: number) {
  return new Vector3(HX * k * Math.sin(th) * Math.sin(phi), HEAD_Y + HY * k * Math.cos(th), HZ * k * Math.sin(th) * Math.cos(phi));
}

// ───────────── the face ─────────────

const HEAD = ellipsoid(0, HEAD_Y, 0, HX, HY, HZ);
/** Eyes sit a little below the middle of the head (that's what makes it cute). */
const EYE_Y = HEAD_Y - 0.035;
const EYE_X = 0.092;

/** Eyes, brows, nose, cheeks and mouth for one face, painted on the head. */
function face(h: Builder, L: Look, f: Face) {
  const S = HEAD;
  const brow = mix(L.hair, [0, 0, 0], 0.3);
  for (const sx of [-1, 1]) {
    const ex = sx * EYE_X;
    if (f === 'closed') {
      // A soft downward lid line.
      stroke(h, S, curve(ex - 0.03, ex + 0.03, (_x, t) => EYE_Y - 0.006 - 0.012 * Math.sin(PI * t)), 0.011, DARK_EYE, 0.003, 0.6);
    } else if (f === 'happy') {
      // ^ ^
      stroke(h, S, curve(ex - 0.032, ex + 0.032, (_x, t) => EYE_Y - 0.012 + 0.024 * Math.sin(PI * t)), 0.012, DARK_EYE, 0.003, 0.6);
    } else {
      // Big glossy eyes: dark, a warm iris low in the eye, two catch-lights (both up and to the
      // viewer's right, one light).
      const w = f === 'worried' ? 1.08 : 1;
      paint(h, S, oval(ex, EYE_Y, 0.031 * w, 0.044 * w), DARK_EYE, 0.002);
      paint(h, S, oval(ex, EYE_Y - 0.015 * w, 0.023 * w, 0.024 * w), L.eye, 0.0032);
      paint(h, S, oval(ex + 0.011, EYE_Y + 0.016 * w, 0.0115, 0.013, 0.3, 12), WHITE, 0.0045);
      paint(h, S, oval(ex - 0.01, EYE_Y - 0.021 * w, 0.0055, 0.0055, 0, 10), WHITE, 0.0045);
      if (L.lashes) stroke(h, S, curve(ex + sx * 0.022, ex + sx * 0.044, (_x, t) => EYE_Y + 0.03 + 0.016 * t * t), 0.009, DARK_EYE, 0.003, 0.5);
    }
    // Brows: arcs in a darker hair color; raised and tilted up inside when worried, one cocked when curious.
    const up = f === 'worried' ? 0.014 : f === 'curious' && sx > 0 ? 0.02 : f === 'happy' ? 0.008 : 0;
    const tilt = f === 'worried' ? 0.016 : f === 'curious' && sx > 0 ? -0.01 : -0.003;
    stroke(h, S, curve(ex - 0.03, ex + 0.03, (x, t) => EYE_Y + 0.066 + up + 0.008 * Math.sin(PI * t) + tilt * (1 - (sx * (x - ex)) / 0.03) * 0.5), 0.013, brow, 0.003, 0.7);
    // Cheeks.
    if (L.blush) paint(h, S, oval(sx * 0.158, EYE_Y - 0.058, 0.036, 0.02), mix(L.skin, lin('#ff7f73'), f === 'happy' ? 0.55 : 0.4), 0.0015);
  }
  // A little button nose.
  const n = S(0, EYE_Y - 0.035);
  h.ball(n[0], n[1], n[2] - 0.004, 0.018, 0.013, 0.014, mix(L.skin, lin('#c0705a'), 0.18), 10, 8);
  const my = EYE_Y - 0.075;
  if (f === 'happy') {
    // An open grin with a tongue.
    const pts: [number, number][] = [];
    for (let i = 0; i <= 12; i++) {
      const a = PI + (i / 12) * PI;
      pts.push([Math.cos(a) * 0.034, my + 0.008 + Math.sin(a) * 0.028]);
    }
    paint(h, S, pts, MOUTH, 0.0025);
    paint(h, S, oval(0, my - 0.011, 0.017, 0.008), TONGUE, 0.0035);
  } else if (f === 'worried') stroke(h, S, curve(-0.018, 0.018, (_x, t) => my - 0.004 + 0.009 * Math.sin(PI * t)), 0.008, MOUTH, 0.003, 0.5);
  else if (f === 'curious') paint(h, S, oval(0.008, my, 0.011, 0.013), MOUTH, 0.0025);
  else stroke(h, S, curve(-0.022, 0.022, (_x, t) => my + 0.004 - 0.01 * Math.sin(PI * t)), 0.0085, MOUTH, 0.003, 0.5);
}

function badgeGlyph(b: Builder, provider: Provider, z: number, fg: RGB) {
  const s = 0.024;
  const star = (n: number, r0: number, r1: number, rot = 0) => {
    const pts: [number, number][] = [];
    for (let i = 0; i < n * 2; i++) {
      const a = rot + (i / (n * 2)) * TAU;
      const rr = i % 2 ? r1 : r0;
      pts.push([Math.cos(a) * rr, Math.sin(a) * rr]);
    }
    b.poly(pts, z, fg);
  };
  switch (provider) {
    case 'claude':
      star(8, s, s * 0.35, PI / 2);
      break;
    case 'gemini':
      star(4, s * 1.05, s * 0.28, PI / 2);
      break;
    case 'codex': {
      const pts: [number, number][] = [];
      for (let i = 0; i < 6; i++) pts.push([Math.cos((i / 6) * TAU + PI / 6) * s, Math.sin((i / 6) * TAU + PI / 6) * s]);
      b.poly(pts, z, fg);
      break;
    }
    case 'opencode':
      b.poly(
        [
          [-s * 0.8, -s * 0.8],
          [s * 0.8, -s * 0.8],
          [s * 0.8, s * 0.8],
          [-s * 0.8, s * 0.8],
        ],
        z,
        fg,
      );
      break;
    case 'cursor':
      b.poly(
        [
          [0, s],
          [-s * 0.9, -s * 0.7],
          [s * 0.9, -s * 0.7],
        ],
        z,
        fg,
      );
      break;
    default:
      star(5, s, s * 0.45, PI / 2);
  }
}

// ───────────── the body ─────────────

/** The top's profile [radius, y] (body-local, hips at 0): a soft chest, narrow shoulders under the head. */
const TOP: [number, number][] = [
  [0.0, 0.03],
  [0.168, 0.03],
  [0.175, 0.05],
  [0.17, 0.1],
  [0.164, 0.155],
  [0.168, 0.21],
  [0.163, 0.255],
  [0.148, 0.292],
  [0.118, 0.32],
  [0.075, 0.338],
  [0.0, 0.345],
];
const PANTS_P: [number, number][] = [
  [0.0, -0.105],
  [0.11, -0.1],
  [0.15, -0.082],
  [0.163, -0.04],
  [0.165, 0.02],
  [0.16, 0.06],
  [0.0, 0.06],
];
/** Front to back, the body is a little flatter than wide. */
const K = 0.86;
const topR = (y: number) => {
  if (y <= TOP[1][1]) return TOP[1][0];
  for (let i = 2; i < TOP.length; i++)
    if (y <= TOP[i][1]) {
      const [r0, y0] = TOP[i - 1];
      const [r1, y1] = TOP[i];
      return r0 + ((r1 - r0) * (y - y0)) / (y1 - y0);
    }
  return 0;
};
const CHEST = lathed(topR, K);

/** The merged meshes of one character (in rig-local coordinates). */
function buildParts(look: Look) {
  const L = look;
  const torso = new Builder();
  const top = L.shirt;
  const trim = L.shirtDark;
  lathe(torso, PANTS_P, L.pants, K);
  // A tee hangs loose over the pants; a blazer and a cardigan are a touch longer.
  lathe(torso, L.role === 'lead' || L.role === 'reviewer' ? TOP.map(([r, y], i) => [r, i < 2 ? y - 0.035 : y] as [number, number]) : TOP, top, K);
  if (L.role === 'lead') {
    // Blazer: a white shirt in a V, a dark tie, lapels, two buttons and a pocket square.
    paint(torso, CHEST, [
      [0, 0.15],
      [0.068, 0.33],
      [-0.068, 0.33],
    ], CREAM, 0.002);
    paint(torso, CHEST, [
      [0, 0.16],
      [0.016, 0.26],
      [0.01, 0.3],
      [-0.01, 0.3],
      [-0.016, 0.26],
    ], lin('#2a2631'), 0.0035);
    for (const sx of [-1, 1]) {
      paint(torso, CHEST, [
        [sx * 0.004, 0.14],
        [sx * 0.07, 0.336],
        [sx * 0.1, 0.318],
        [sx * 0.082, 0.27],
        [sx * 0.098, 0.255],
        [sx * 0.036, 0.15],
      ], trim, 0.004);
    }
    for (const y of [0.1, 0.05]) paint(torso, CHEST, oval(0, y, 0.011, 0.011, 0, 10), mix(trim, [0, 0, 0], 0.35), 0.003);
    paint(torso, CHEST, [
      [-0.12, 0.2],
      [-0.07, 0.2],
      [-0.075, 0.225],
      [-0.1, 0.235],
      [-0.115, 0.225],
    ], CREAM, 0.003);
  } else if (L.role === 'reviewer') {
    // Cardigan: a shirt collar, a placket and buttons down the front.
    for (const sx of [-1, 1])
      paint(torso, CHEST, [
        [0, 0.31],
        [sx * 0.085, 0.335],
        [sx * 0.06, 0.27],
      ], CREAM, 0.004);
    stroke(torso, CHEST, [
      [0, 0.3],
      [0, 0.0],
    ], 0.016, trim, 0.002);
    for (let i = 0; i < 4; i++) paint(torso, CHEST, oval(0, 0.255 - i * 0.065, 0.01, 0.01, 0, 10), CREAM, 0.0045);
    ring(torso, 0.0, 0.17, 0.014, trim, K);
  } else if (L.accessory === 'hood') {
    // Hoodie: the hood bunched behind the neck, drawstrings, a kangaroo pocket, a ribbed hem.
    ring(torso, 0.3, 0.115, 0.05, trim, 1, -0.045, -0.5);
    for (const sx of [-1, 1]) {
      stroke(torso, CHEST, [
        [sx * 0.035, 0.305],
        [sx * 0.04, 0.2],
      ], 0.01, CREAM, 0.004);
      const tip = CHEST(sx * 0.04, 0.195);
      torso.ball(tip[0], tip[1], tip[2] + 0.005, 0.011, 0.014, 0.011, CREAM, 8, 6);
    }
    paint(torso, CHEST, [
      [-0.095, 0.055],
      [0.095, 0.055],
      [0.1, 0.075],
      [0.075, 0.14],
      [-0.075, 0.14],
      [-0.1, 0.075],
    ], mix(top, trim, 0.45), 0.0035);
    ring(torso, 0.045, 0.172, 0.018, trim, K);
  } else {
    // Tee: a crew collar and a hem.
    ring(torso, 0.33, 0.085, 0.017, trim, K);
    ring(torso, 0.04, 0.173, 0.012, trim, K);
  }
  if (L.intern) {
    // A tiny backpack with straps.
    torso.ball(0, 0.16, -0.17, 0.12, 0.13, 0.07, lin('#e0a458'), 16, 12);
    torso.ball(0, 0.11, -0.225, 0.08, 0.05, 0.03, lin('#c98b40'), 12, 8);
    for (const sx of [-1, 1]) stroke(torso, CHEST, [[sx * 0.09, 0.3], [sx * 0.1, 0.12]], 0.022, lin('#c98b40'), 0.004);
  }
  // Provider pin on the left chest.
  const pv = PROVIDER[L.provider] ?? PROVIDER.claude;
  const bx = 0.078;
  const by = 0.21;
  const at = CHEST(bx, by);
  const rot = Math.atan2(at[3], at[5]);
  const badge = new Builder();
  badge.frame(at[0] + at[3] * 0.008, by, at[2] + at[5] * 0.008, rot);
  badge.poly(oval(0, 0, 0.038, 0.038, 0, 20), 0, lin(pv.bg));
  badgeGlyph(badge, L.provider, 0.001, lin(pv.fg));
  torso.frame(at[0] + at[3] * 0.005, by, at[2] + at[5] * 0.005, rot).poly(oval(0, 0, 0.046, 0.046, 0, 20), 0, CREAM).frame();

  // Head, once per face (gfx swaps the geometry: a blink, a grin, a worried look).
  const heads = FACES.map((f) => {
    const h = new Builder();
    h.ball(0, HEAD_Y, 0, HX, HY, HZ, L.skin, 28, 20);
    h.ball(0, 0.03, -0.005, 0.06, 0.06, 0.055, L.skin, 12, 8);
    // Ears.
    for (const sx of [-1, 1]) {
      h.ball(sx * HX * 0.96, EYE_Y, -0.015, 0.032, 0.046, 0.03, mix(L.skin, [0, 0, 0], 0.04), 12, 8);
      h.ball(sx * (HX * 0.96 + 0.012), EYE_Y, -0.01, 0.014, 0.026, 0.016, mix(L.skin, lin('#b0604c'), 0.2), 8, 6);
    }
    face(h, L, f);
    // Big styles (bun, spikes, curls, quiff, afro, swept top) tuck under an intern's cap.
    hair(h, L.intern && [2, 3, 6, 8, 9, 10].includes(L.style) ? { ...L, style: 0 } : L);
    if (L.glasses) glasses(h);
    if (L.accessory === 'headphones' && !L.intern) headphones(h, L);
    if (L.intern) cap(h, L);
    return h.build();
  });

  // Arms: upper (sleeve) and forearm (skin or sleeve), hanging down from their pivots.
  const longSleeve = L.role === 'lead' || L.role === 'reviewer' || L.accessory === 'hood';
  const upper = new Builder();
  const fore = new Builder();
  if (longSleeve) {
    limb(upper, 0, 0.058, UPPER, top, 0.052);
    limb(fore, 0, 0.05, FORE - 0.02, top, 0.046);
    ring(fore, -FORE + 0.03, 0.044, 0.012, L.role === 'lead' ? CREAM : trim);
  } else {
    // A short sleeve, then the bare arm.
    limb(upper, 0, 0.06, 0.07, top, 0.062);
    ring(upper, -0.085, 0.056, 0.012, trim);
    limb(upper, -0.06, 0.04, UPPER - 0.06, L.skin, 0.038);
    limb(fore, 0, 0.039, FORE - 0.02, L.skin, 0.035);
  }
  // The hand hangs at the wrist, so it can bend (a pad, a cue, a rod handle): a mitten and a thumb.
  const hand = new Builder();
  hand.ball(0, -0.012, 0.004, 0.044, 0.05, 0.036, L.skin, 14, 10);
  hand.ball(0.006, -0.004, 0.034, 0.016, 0.026, 0.015, L.skin, 8, 6, new Euler(0.3, 0, 0));

  // Legs: thigh and shin (the knee bends) and the shoe at the ankle.
  const thigh = new Builder();
  limb(thigh, 0, 0.07, THIGH, L.pants, 0.062);
  const shin = new Builder();
  limb(shin, 0, 0.062, SHIN - 0.01, L.pants, 0.058);
  ring(shin, -SHIN + 0.035, 0.058, 0.01, mix(L.pants, [0, 0, 0], 0.25));
  const shoe = new Builder();
  const light = L.shoes[0] + L.shoes[1] + L.shoes[2] > 1.2;
  shoe.ball(0, 0.012, 0.028, 0.064, 0.05, 0.098, L.shoes, 18, 12);
  shoe.ball(0, 0.035, 0.07, 0.04, 0.02, 0.045, mix(L.shoes, light ? [0, 0, 0] : [1, 1, 1], 0.18), 10, 6);
  // A sole in contrast (light under dark shoes, dark under light ones).
  shoe.ball(0, -0.016, 0.03, 0.069, 0.02, 0.104, lin(light ? '#6b625a' : '#ece6dc'), 18, 6);
  return {
    torso: torso.build(),
    badge: badge.build(),
    heads,
    upper: upper.build(),
    fore: fore.build(),
    hand: hand.build(),
    thigh: thigh.build(),
    shin: shin.build(),
    shoe: shoe.build(),
  };
}

/** How high the hair rises over the crown (headphones and caps sit on it). */
function hairTop(style: number) {
  return style === 9 ? 0.11 : style === 6 ? 0.05 : style === 7 ? 0.01 : 0.03;
}

function hair(h: Builder, L: Look) {
  const c = L.hair;
  const darker = mix(c, [0, 0, 0], 0.22);
  const crown = (th: number, amt: number) => amt * Math.max(0, Math.cos(th)) ** 2;
  switch (L.style) {
    case 0: // short, swept to one side
      shell(h, (p) => hairline(0.33 * PI, 0.5 * PI, 0.76 * PI)(p) + sweep(p, 1, 0.13 * PI), 0.024, c, (th) => crown(th, 0.07));
      break;
    case 1: // bob with straight bangs, flaring out at the bottom
      shell(h, hairline(0.4 * PI, 0.78 * PI, 0.8 * PI), 0.026, c, (th, p) => crown(th, 0.06) + 0.11 * smooth(0.45 * PI, 0.8 * PI, th) * (1 - Math.max(0, Math.cos(p)) ** 6));
      break;
    case 2: // bun, the hair pulled back
      shell(h, hairline(0.3 * PI, 0.52 * PI, 0.78 * PI), 0.018, c, (th) => crown(th, 0.03));
      h.ball(0, HEAD_Y + 0.235, -0.12, 0.105, 0.1, 0.1, c, 16, 12);
      ring(h, HEAD_Y + 0.17, 0.06, 0.016, darker, 1, -0.09, 0.6);
      break;
    case 3: {
      // spiky
      shell(h, (p) => hairline(0.34 * PI, 0.5 * PI, 0.74 * PI)(p) + 0.05 * PI * Math.max(0, Math.cos(p)) ** 2 * (0.5 + 0.5 * Math.cos(p * 9)), 0.024, c, (th) => crown(th, 0.05));
      for (const [th, p] of [
        [0.05, 0],
        [0.22, 0.25],
        [0.22, -0.3],
        [0.24, 0.95],
        [0.24, -1],
        [0.28, 1.7],
        [0.28, -1.75],
        [0.3, 2.5],
        [0.3, -2.5],
        [0.25, PI],
        [0.14, 0.9 + PI / 2],
        [0.14, -0.9 - PI / 2],
      ] as const) {
        const at = onHead(th * PI, p, 1.03);
        const dir = at.clone().sub(new Vector3(0, HEAD_Y - 0.05, 0.02)).normalize();
        dir.y += 0.25;
        spike(h, at, dir, 0.14, 0.055, c);
      }
      break;
    }
    case 4: // long, past the shoulders
      shell(h, (p) => hairline(0.34 * PI, 0.72 * PI, 0.85 * PI)(p) + sweep(p, -1, 0.1 * PI), 0.024, c, (th, p) => crown(th, 0.05) + 0.05 * smooth(0.5 * PI, 0.8 * PI, th) * (1 - Math.max(0, Math.cos(p)) ** 4));
      h.ball(0, HEAD_Y - 0.2, -0.13, 0.25, 0.3, 0.13, c, 20, 14, new Euler(0.18, 0, 0));
      for (const sx of [-1, 1]) h.ball(sx * 0.21, HEAD_Y - 0.17, -0.05, 0.075, 0.21, 0.09, c, 14, 10, new Euler(0.1, 0, sx * 0.12));
      break;
    case 5: {
      // ponytail
      shell(h, hairline(0.3 * PI, 0.52 * PI, 0.78 * PI), 0.018, c, (th) => crown(th, 0.03));
      ring(h, HEAD_Y + 0.07, 0.04, 0.017, darker, 1, -HZ - 0.005, -1.2);
      h.ball(0, HEAD_Y - 0.03, -HZ - 0.065, 0.07, 0.16, 0.07, c, 16, 12, new Euler(0.45, 0, 0));
      h.ball(0, HEAD_Y - 0.17, -HZ - 0.09, 0.05, 0.09, 0.05, c, 12, 8, new Euler(0.15, 0, 0));
      break;
    }
    case 6: // curls
      shell(h, hairline(0.34 * PI, 0.56 * PI, 0.82 * PI), 0.03, c, (th, p) => 0.05 + crown(th, 0.08) + 0.05 * bumps(th, p, 7), 64, 20);
      break;
    case 7: // buzz cut
      shell(h, hairline(0.3 * PI, 0.47 * PI, 0.74 * PI), 0.007, mix(c, L.skin, 0.12));
      break;
    case 8: // quiff: short sides, the front swept up and back in a wave
      shell(h, hairline(0.29 * PI, 0.47 * PI, 0.74 * PI), 0.014, c, (th, p) => 0.3 * Math.exp(-((th - 0.22 * PI) ** 2) / 0.022) * Math.max(0, Math.cos(p)) ** 2 + crown(th, 0.07), 64, 20);
      break;
    case 9: // afro
      shell(h, hairline(0.31 * PI, 0.5 * PI, 0.8 * PI), 0.03, c, (th, p) => 0.22 + 0.14 * Math.max(0, Math.cos(th)) + 0.045 * bumps(th, p, 11), 72, 24, 0.08);
      break;
    case 10: // undercut: buzzed sides, a long top swept up and over to one side
      shell(h, hairline(0.3 * PI, 0.47 * PI, 0.74 * PI), 0.007, mix(darker, L.skin, 0.25), () => 0, 56, 16, 0);
      shell(h, (p) => hairline(0.28 * PI, 0.36 * PI, 0.5 * PI)(p) + sweep(p, 1, 0.1 * PI), 0.03, c, (th, p) => 0.16 * Math.max(0, Math.cos(th)) ** 2 * (1 + 0.5 * Math.max(0, Math.cos(p))) + 0.05 * Math.max(0, Math.sin(p)), 64, 18);
      break;
  }
}

function glasses(h: Builder) {
  const frame = lin('#3a2a24');
  const tor = new TorusGeometry(0.05, 0.0075, 6, 22).toNonIndexed();
  for (const sx of [-1, 1]) {
    const p = HEAD(sx * EYE_X, EYE_Y);
    const m = new Matrix4().compose(
      new Vector3(p[0] + p[3] * 0.022, p[1] + p[4] * 0.022, p[2] + p[5] * 0.022),
      new Quaternion().setFromUnitVectors(new Vector3(0, 0, 1), new Vector3(p[3], 0, p[5]).normalize()),
      new Vector3(1, 0.9, 1),
    );
    h.add(tor, frame, m);
    // The temple arm back to the ear.
    h.rod(new Vector3(sx * (EYE_X + 0.048), EYE_Y + 0.01, p[2] + 0.005), new Vector3(sx * HX * 0.98, EYE_Y + 0.01, -0.02), 0.006, frame, 6);
  }
  const mid = HEAD(0, EYE_Y + 0.008);
  h.box(0, EYE_Y + 0.008, mid[2] + 0.02, 0.05, 0.008, 0.008, frame);
  tor.dispose();
}

function headphones(h: Builder, L: Look) {
  const band = lin('#2f3138');
  const R = HX + hairTop(L.style) + 0.035;
  const tor = new TorusGeometry(R, 0.017, 8, 28, PI).toNonIndexed();
  h.add(tor, band, new Matrix4().makeTranslation(0, EYE_Y + 0.01, -0.015));
  tor.dispose();
  for (const sx of [-1, 1]) {
    h.ball(sx * (R - 0.005), EYE_Y, -0.015, 0.035, 0.066, 0.062, band, 14, 10);
    h.ball(sx * (R + 0.022), EYE_Y, -0.015, 0.012, 0.048, 0.045, sx < 0 ? L.shirt : L.shirtDark, 12, 8);
  }
}

function cap(h: Builder, L: Look) {
  // A dome over the hair (tipped back a little), a band and a brim over the forehead.
  const dome = new SphereGeometry(1, 28, 10, 0, TAU, 0, PI / 2).toNonIndexed();
  const m = new Matrix4().compose(new Vector3(0, HEAD_Y + 0.06, -0.01), new Quaternion().setFromEuler(new Euler(-0.12, 0, 0)), new Vector3(HX + 0.04, HY * 0.85, HZ + 0.04));
  h.add(dome, L.shirt, m);
  dome.dispose();
  ring(h, HEAD_Y + 0.07, HX + 0.03, 0.016, L.shirtDark, (HZ + 0.03) / (HX + 0.03), -0.01, -0.12);
  h.ball(0, HEAD_Y + 0.085, HZ + 0.05, 0.16, 0.014, 0.11, L.shirtDark, 18, 6, new Euler(0.14, 0, 0));
  h.ball(0, HEAD_Y + 0.28, -0.04, 0.025, 0.014, 0.025, L.shirtDark, 8, 4);
}

// ───────────── shared props ─────────────

export class PropKit {
  geos = new Map<Exclude<Prop, null>, BufferGeometry>();
  constructor() {
    const mk = (fn: (b: Builder) => void) => {
      const b = new Builder();
      fn(b);
      return b.build();
    };
    this.geos.set('paper', mk((b) => b.box(0, 0, 0.06, 0.15, 0.006, 0.2, lin('#fbf8f1')).box(0, 0.004, 0.04, 0.1, 0.002, 0.012, lin('#9aa0aa')).box(0, 0.004, 0.08, 0.11, 0.002, 0.012, lin('#9aa0aa'))));
    this.geos.set('book', mk((b) => b.box(0, 0, 0.07, 0.17, 0.045, 0.23, lin('#4f7c8a')).box(0, 0, 0.07, 0.16, 0.035, 0.225, lin('#fbf8f1'))));
    this.geos.set('card', mk((b) => b.box(0, 0, 0.05, 0.16, 0.006, 0.11, lin('#fdf7e6')).box(0, 0.004, 0.012, 0.16, 0.002, 0.025, lin('#e0a458'))));
    this.geos.set('envelope', mk((b) => b.box(0, 0, 0.05, 0.18, 0.012, 0.12, lin('#f3e6cc')).box(0, 0.007, 0.03, 0.12, 0.002, 0.05, lin('#c45b4a'))));
    this.geos.set('folder', mk((b) => b.box(0, 0, 0.07, 0.2, 0.025, 0.25, lin('#e0b66a')).box(0, 0.014, 0.07, 0.17, 0.004, 0.22, lin('#fbf8f1'))));
    this.geos.set('phone', mk((b) => b.box(0, 0, 0.02, 0.04, 0.09, 0.016, lin('#26272c'))));
    this.geos.set('marker', mk((b) => b.cyl(0, -0.02, 0.02, 0.012, 0.13, lin('#2f6fd6'), 6)));
    this.geos.set('note', mk((b) => b.box(0, 0, 0.04, 0.08, 0.004, 0.08, lin('#ffd75e'))));
    this.geos.set('mug', mk((b) => b.cyl(0, -0.04, 0.03, 0.035, 0.08, lin('#e9e2d6'), 10)));
    this.geos.set('gamepad', mk((b) => b.box(0.05, 0, 0.03, 0.15, 0.03, 0.07, lin('#f1f1f4')).ball(-0.02, -0.01, 0.05, 0.035, 0.025, 0.04, lin('#26272c'), 8, 6).ball(0.12, -0.01, 0.05, 0.035, 0.025, 0.04, lin('#26272c'), 8, 6)));
    this.geos.set('paddle', mk((b) => b.cyl(0, -0.07, 0.01, 0.012, 0.07, lin('#7a5234'), 8).ball(0, -0.13, 0.01, 0.065, 0.07, 0.008, lin('#d6403a'), 14, 6)));
    this.geos.set('cue', mk((b) => b.cyl(0, -0.45, 0.02, 0.016, 1.1, lin('#c9a46a'), 6, 0.008).cyl(0, -0.45, 0.02, 0.018, 0.3, lin('#3a2a20'), 6)));
  }
  dispose() {
    for (const g of this.geos.values()) g.dispose();
  }
}

// ───────────── the character ─────────────

const ease = (t: number) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2);
const clamp = (x: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, x));
const BLEND_MS = 240;
/** Poses whose idle moments include a look at the phone (gfx/motion.ts `glancing`). */
const GLANCE_POSES = new Set<Pose>(['sit', 'stand', 'type']);
const WALK_SPEED = 2.1;
const STRIDE = 0.62;

export interface CharacterEnv {
  emotes: (k: EmoteKind) => Texture;
  props: PropKit;
}

export class Character {
  readonly id: string;
  readonly root = new Group();
  private body = new Group();
  private head = new Group();
  private headMesh: Mesh;
  private armL = new Group();
  private armR = new Group();
  private foreL = new Group();
  private foreR = new Group();
  private legL = new Group();
  private legR = new Group();
  private kneeL = new Group();
  private kneeR = new Group();
  private ankleL = new Group();
  private ankleR = new Group();
  private wristL = new Group();
  private wristR = new Group();
  /** Sitting down or getting up: through a crouch, not a straight blend. */
  private transit: { mid: Joints; at: number; ms: number } | null = null;
  readonly hand = new Object3D();
  private propMesh: Mesh;
  private mat: MeshStandardMaterial;
  private geos: BufferGeometry[] = [];
  private heads: BufferGeometry[];
  private eyes = 0;
  readonly look: Look;
  private env: CharacterEnv;

  // emotes
  private emote: Emote = null;
  private emoteSprite: Sprite;
  private emoteMat: SpriteMaterial;
  private emoteAt = 0;
  private cloud: Group | null = null;
  private cloudAt = 0;
  private drops: Mesh[] = [];

  // state
  pose: Pose = 'stand';
  prop: Prop = null;
  seated = false;
  seatY = 0;
  private cur = new Float32Array(J);
  private from = new Float32Array(J);
  private goal = new Float32Array(J);
  private blendAt = -1e9;
  private step = 0;
  private phase = 0;
  private path: [number, number][] = [];
  private final: { x: number; z: number; face: number } | null = null;
  walking = false;
  private yawGoal = 0;
  private fade: { from: number; to: number; at: number; ms: number; then?: () => void } | null = null;
  opacity = 1;
  private jumpUntil = 0;
  /** Seconds-per-unit multiplier (crowds walk a touch slower). */
  speed = WALK_SPEED;
  arrivedKey: string | null = null;
  destKey: string | null = null;
  emoteScale = 1;
  /** Where to look (world), set by the engine; null = its own glances. */
  gaze: Vector3 | null = null;
  /** Its game's cue (gfx/games.ts); null when not playing. */
  cue: PlayCue | null = null;
  /** A personal reaction playing over its pose (traits.ts), started at `at`. */
  private reacting: { kind: ReactKind; at: number } | null = null;
  /** How it walks, from its needs (set by the engine): tired drags, happy bounces. */
  gait: 'tired' | 'happy' | null = null;
  /** Too much coffee (set by the engine): it fidgets. */
  jittery = false;
  /** Its phone is out for a quick look (only while it holds nothing else). */
  private glance = false;

  // its own rhythm: two characters never move in sync
  private readonly seed: number;
  private readonly tempo: number;
  private readonly offset: number;
  private readonly breathPeriod: number;
  private readonly lazy: boolean;
  private readonly rand: () => number;
  private gazeYaw = 0;
  private gazePitch = 0;
  private wanderAt = 0;
  private wanderYaw = 0;
  private wanderPitch = 0;
  private blinkAt = -1;
  private blinkUntil = 0;

  constructor(id: string, look: Look, env: CharacterEnv) {
    this.id = id;
    this.look = look;
    this.env = env;
    const h = hash(id);
    this.seed = h & 0x7fffffff;
    this.rand = rng(h ^ 0x9e3779b9);
    this.tempo = 0.88 + this.rand() * 0.28;
    this.offset = this.rand() * 1000;
    this.breathPeriod = 3.6 + this.rand() * 1.4;
    this.lazy = traitsOf(id).includes('lazy');
    this.mat = new MeshStandardMaterial({ vertexColors: true, roughness: 0.72, metalness: 0, transparent: false });
    // A soft warm rim (in the surface's own color), so people stand out from the floor like clay.
    this.mat.onBeforeCompile = (sh) => {
      sh.fragmentShader = sh.fragmentShader.replace(
        '#include <emissivemap_fragment>',
        '#include <emissivemap_fragment>\n  totalEmissiveRadiance += diffuseColor.rgb * vec3(1.0, 0.93, 0.85) * 0.32 * pow(1.0 - saturate(dot(normalize(vViewPosition), normal)), 2.5);',
      );
    };
    this.mat.customProgramCacheKey = () => 'character-rim';
    const parts = buildParts(look);
    this.heads = parts.heads;
    this.geos.push(parts.torso, parts.badge, ...parts.heads, parts.upper, parts.fore, parts.hand, parts.thigh, parts.shin, parts.shoe);
    const mesh = (g: BufferGeometry, parent: Object3D) => {
      const m = new Mesh(g, this.mat);
      m.castShadow = true;
      m.receiveShadow = false;
      parent.add(m);
      return m;
    };
    const badgeMat = new MeshBasicMaterial({ vertexColors: true });
    this.extraMats.push(badgeMat);
    this.root.add(this.body);
    this.body.position.y = HIP;
    mesh(parts.torso, this.body);
    const badge = new Mesh(parts.badge, badgeMat);
    this.body.add(badge);
    this.body.add(this.head);
    this.head.position.y = NECK;
    this.headMesh = mesh(this.heads[0], this.head);
    for (const [arm, fore, wrist, sx] of [
      [this.armL, this.foreL, this.wristL, 1],
      [this.armR, this.foreR, this.wristR, -1],
    ] as const) {
      arm.position.set(sx * SHOULDER_X, SHOULDER_Y, 0);
      this.body.add(arm);
      mesh(parts.upper, arm);
      fore.position.y = -UPPER;
      arm.add(fore);
      mesh(parts.fore, fore);
      wrist.position.y = -FORE;
      fore.add(wrist);
      mesh(parts.hand, wrist);
    }
    for (const [leg, knee, ankle, sx] of [
      [this.legL, this.kneeL, this.ankleL, 1],
      [this.legR, this.kneeR, this.ankleR, -1],
    ] as const) {
      leg.position.set(sx * LEG_X, HIP, 0);
      this.root.add(leg);
      mesh(parts.thigh, leg);
      knee.position.y = -THIGH;
      leg.add(knee);
      mesh(parts.shin, knee);
      ankle.position.y = -SHIN;
      knee.add(ankle);
      mesh(parts.shoe, ankle);
    }
    this.hand.position.set(0, -0.02, 0.02);
    this.wristR.add(this.hand);
    this.propMesh = new Mesh(env.props.geos.get('paper')!, this.mat);
    this.propMesh.castShadow = true;
    this.propMesh.visible = false;
    this.hand.add(this.propMesh);
    this.emoteMat = new SpriteMaterial({ transparent: true, depthWrite: false });
    this.emoteSprite = new Sprite(this.emoteMat);
    this.emoteSprite.visible = false;
    this.emoteSprite.renderOrder = 10;
    this.root.add(this.emoteSprite);
    this.root.scale.setScalar(look.scale);
    targets(this.goal, 'stand', false, this.ctx(0, 0, false));
    this.cur.set(this.goal);
    this.apply();
  }

  private ctx(tMs: number, now: number, live: boolean) {
    return { t: (tMs / 1000) * this.tempo + this.offset, now, seed: this.seed, cue: this.cue, live, lazy: this.lazy, tired: this.gait === 'tired', jittery: this.jittery };
  }

  private extraMats: (MeshBasicMaterial | MeshStandardMaterial)[] = [];

  get position() {
    return this.root.position;
  }

  /** Head top in world space (for labels and emotes). */
  headTop(out: Vector3): Vector3 {
    return out.set(0, HEAD_Y + HEAD_R + 0.04, 0).applyMatrix4(this.head.matrixWorld);
  }

  // ───── locomotion ─────

  /** Walk along `path` (world x/z points), ending exactly at `final` facing `final.face`. */
  walk(path: [number, number][], final: { x: number; z: number; face: number }, key: string, now: number) {
    this.path = path.slice();
    this.final = final;
    this.destKey = key;
    this.arrivedKey = null;
    if (this.seated) {
      this.seated = false;
      this.setPose('stand', now, this.seatY);
    }
    this.walking = true;
  }

  /** Jump straight to a place (reduced motion: a quick crossfade; or first placement). */
  teleport(final: { x: number; z: number; face: number }, key: string, now: number, crossfade: boolean) {
    const go = () => {
      this.root.position.set(final.x, 0, final.z);
      this.root.rotation.y = final.face;
      this.yawGoal = final.face;
      this.path = [];
      this.final = null;
      this.walking = false;
      this.arrivedKey = key;
      this.destKey = key;
    };
    if (!crossfade) {
      go();
      return;
    }
    this.fadeTo(0, now, 75, () => {
      go();
      this.fadeTo(1, performance.now(), 75);
    });
  }

  fadeTo(to: number, now: number, ms: number, then?: () => void) {
    this.fade = { from: this.opacity, to, at: now, ms, then };
    this.mat.transparent = true;
  }

  // ───── pose / props / emotes ─────

  setPose(pose: Pose, now: number, seatY?: number) {
    const seated = SIT_POSES.has(pose) && seatY != null && seatY > 0;
    if (pose === this.pose && seated === this.seated) return;
    const sitting = seated !== this.seated;
    this.pose = pose;
    this.seated = seated;
    if (seatY != null) this.seatY = seatY;
    this.from.set(this.cur);
    this.blendAt = now;
    // Sitting down / getting up passes through a crouch: knees bent, leaning forward.
    if (sitting) {
      const mid = new Float32Array(this.cur);
      mid[11] = mid[12] = -0.95;
      mid[14] = mid[15] = 1.15;
      mid[16] = mid[17] = -0.2;
      mid[1] = 0.38;
      mid[23] = 0.55;
      mid[5] = mid[8] = -0.55;
      mid[7] = mid[10] = -0.4;
      this.transit = { mid, at: now, ms: 560 };
    }
    if (pose === 'celebrate') this.jumpUntil = now + 1500;
  }

  /** Play a personal reaction (a thump, a facepalm…) over whatever it is doing. */
  react(kind: ReactKind, now: number) {
    this.reacting = { kind, at: now };
  }

  setProp(prop: Prop) {
    if (prop === this.prop) return;
    this.prop = prop;
    const g = prop ? this.env.props.geos.get(prop) : null;
    if (g) {
      this.propMesh.geometry = g;
      this.propMesh.visible = true;
    } else this.propMesh.visible = false;
  }

  private showPhone(on: boolean) {
    if (on === this.glance) return;
    this.glance = on;
    if (this.prop) return;
    this.propMesh.geometry = this.env.props.geos.get('phone')!;
    this.propMesh.visible = on;
  }

  setEmote(e: Emote, now: number) {
    if (e === this.emote) return;
    this.emote = e;
    this.emoteAt = now;
    const kind: EmoteKind | null = e === 'alert' ? 'alert' : e === 'question' ? 'question' : e === 'zz' ? 'zz' : e === 'think' ? 'think3' : e === 'check' ? 'check' : null;
    if (kind) {
      this.emoteMat.map = this.env.emotes(kind);
      this.emoteMat.depthTest = !(e === 'alert' || e === 'question');
      this.emoteMat.needsUpdate = true;
      this.emoteSprite.visible = true;
    } else this.emoteSprite.visible = false;
    this.eyes = FACES.indexOf(e === 'zz' ? 'closed' : e === 'check' ? 'happy' : e === 'rain' || e === 'alert' ? 'worried' : e === 'question' || e === 'think' ? 'curious' : 'open');
    this.blinkUntil = 0;
    this.headMesh.geometry = this.heads[this.eyes];
    if (e === 'rain') this.makeCloud(now);
    else if (this.cloud) {
      this.root.remove(this.cloud);
      this.cloud = null;
    }
  }

  private makeCloud(now: number) {
    if (this.cloud) return;
    const b = new Builder();
    const grey = lin('#8f96a3');
    for (const [x, y, z, r] of [
      [0, 0, 0, 0.16],
      [0.14, -0.02, 0.02, 0.12],
      [-0.14, -0.03, 0, 0.12],
      [0.05, 0.08, -0.02, 0.12],
    ])
      b.blob(x, y, z, r, r * 0.8, r, grey, 1);
    const g = b.build();
    this.geos.push(g);
    const cloud = new Group();
    const cm = new Mesh(g, this.mat);
    cloud.add(cm);
    const dropMat = new MeshBasicMaterial({ color: 0x7fb2ff });
    this.extraMats.push(dropMat);
    const db = new Builder();
    db.box(0, 0, 0, 0.012, 0.06, 0.012, [1, 1, 1]);
    const dg = db.build();
    this.geos.push(dg);
    this.drops = [];
    for (let i = 0; i < 5; i++) {
      const d = new Mesh(dg, dropMat);
      d.position.set(-0.12 + i * 0.06, -0.12, (i % 2) * 0.04 - 0.02);
      cloud.add(d);
      this.drops.push(d);
    }
    cloud.position.set(0, NECK + HIP + HEAD_Y + HEAD_R + 0.32, 0);
    this.root.add(cloud);
    this.cloud = cloud;
    this.cloudAt = now;
  }

  // ───── frame ─────

  /**
   * Advance by dt. `live` = continuous motion (breathing, glances, smooth loops); otherwise
   * loops tick in steps. Returns whether it needs continuous frames (live, walking, blending,
   * fading, jumping) and when its next stepped tick is due (Infinity when static).
   */
  update(now: number, dt: number, reducedMotion: boolean, live = false): { moving: boolean; nextAt: number } {
    live = live && !reducedMotion;
    let moving = false;
    let nextAt = Infinity;

    // Fades.
    if (this.fade) {
      const f = this.fade;
      const t = Math.min(1, (now - f.at) / f.ms);
      this.opacity = f.from + (f.to - f.from) * t;
      this.mat.opacity = this.opacity;
      if (t >= 1) {
        this.fade = null;
        if (this.opacity >= 1) this.mat.transparent = false;
        f.then?.();
      }
      moving = true;
    }

    // Walking.
    if (this.walking) {
      moving = true;
      const pace = this.gait === 'tired' ? 0.75 : this.gait === 'happy' ? 1.12 : 1;
      let remaining = this.speed * pace * dt;
      const p = this.root.position;
      while (remaining > 0) {
        const next = this.path[0] ?? (this.final ? [this.final.x, this.final.z] : null);
        if (!next) break;
        const dx = next[0] - p.x;
        const dz = next[1] - p.z;
        const d = Math.hypot(dx, dz);
        if (d > 1e-4) this.yawGoal = Math.atan2(dx, dz);
        if (d <= remaining) {
          p.x = next[0];
          p.z = next[1];
          remaining -= d;
          if (this.path.length) this.path.shift();
          else {
            this.walking = false;
            this.yawGoal = this.final!.face;
            this.arrivedKey = this.destKey;
            // Ease out of the stride into whatever pose comes next.
            this.from.set(this.cur);
            this.blendAt = now;
            break;
          }
        } else {
          p.x += (dx / d) * remaining;
          p.z += (dz / d) * remaining;
          remaining = 0;
        }
      }
      this.phase += ((this.speed * pace * dt) / STRIDE) * Math.PI * 2;
    }

    // Turning.
    let dy = this.yawGoal - this.root.rotation.y;
    dy = Math.atan2(Math.sin(dy), Math.cos(dy));
    if (Math.abs(dy) > 0.004) {
      const k = 1 - Math.exp(-dt * (this.walking ? 14 : 10));
      this.root.rotation.y += dy * k;
      moving = true;
    } else this.root.rotation.y = this.yawGoal;

    // Pose targets: continuous while live; otherwise stepped loops tick on the shared clock.
    const period = this.walking || live ? 0 : loopPeriod(this.pose) * (reducedMotion ? 4 : 1);
    let tMs = live ? now : 0;
    if (period) {
      this.step = Math.floor(now / period);
      nextAt = (this.step + 1) * period;
      tMs = this.step * period;
    }
    if (this.walking) {
      targets(this.goal, 'stand', false, this.ctx(0, now, false)); // no fidgets mid-stride
      this.showPhone(false);
      const s = Math.sin(this.phase);
      const c = Math.cos(this.phase);
      this.goal[11] = s * 0.6;
      this.goal[12] = -s * 0.6;
      // The knee lifts while the leg swings forward, the foot rolls off behind; hips sway.
      this.goal[14] = 0.08 + Math.max(0, -c) * 0.85;
      this.goal[15] = 0.08 + Math.max(0, c) * 0.85;
      this.goal[16] = 0.25 * Math.max(0, s);
      this.goal[17] = 0.25 * Math.max(0, -s);
      this.goal[20] = s * 0.035;
      this.goal[5] = -s * 0.5;
      this.goal[8] = s * 0.5;
      this.goal[7] = this.goal[10] = -0.35;
      this.goal[1] = 0.06;
      this.goal[0] = Math.abs(Math.cos(this.phase)) * 0.03;
      this.goal[13] = -s * 0.06; // the shoulders counter the hips
      if (this.gait === 'tired') {
        // Slumped, head low, arms barely swinging.
        this.goal[1] += 0.14;
        this.goal[2] += 0.22;
        this.goal[5] *= 0.45;
        this.goal[8] *= 0.45;
        this.goal[0] *= 0.4;
      } else if (this.gait === 'happy') {
        // A spring in the step, chin up, arms swinging.
        this.goal[0] *= 2.4;
        this.goal[5] *= 1.4;
        this.goal[8] *= 1.4;
        this.goal[2] -= 0.08;
      }
      if (this.prop) {
        // Carry in front.
        this.goal[8] = -0.55;
        this.goal[10] = -1.1;
        this.goal[9] = -0.1;
      }
    } else {
      const cx = this.ctx(tMs, now, live);
      targets(this.goal, this.pose, this.seated, cx);
      this.showPhone(!this.prop && GLANCE_POSES.has(this.pose) && glancing(cx) > 0.25);
      if (live) {
        addLife(this.goal, this.pose, this.seated, now / 1000, this.seed, this.breathPeriod);
        this.look_(now, dt);
      }
      if (this.reacting) {
        const age = (now - this.reacting.at) / 1000;
        const dur = REACT_S[this.reacting.kind];
        if (age >= dur) this.reacting = null;
        else {
          reactPose(this.goal, this.reacting.kind, Math.min(1, age / 0.15) * Math.min(1, (dur - age) / 0.35), age);
          moving = true;
        }
      }
    }
    if (this.jumpUntil > now && !reducedMotion) {
      this.goal[0] += Math.abs(Math.sin((now / 1000) * 8.5)) * 0.14;
      moving = true;
    }

    // Blend into a new pose, then follow the goal exactly (no per-frame drift when still).
    const bt = (now - this.blendAt) / BLEND_MS;
    const tr = this.transit;
    const tt = tr ? (now - tr.at) / tr.ms : 1;
    if (tr && tt < 1 && !this.walking) {
      // Through the crouch: a quadratic curve from → mid → goal.
      const k = ease(Math.max(0, tt));
      const a = (1 - k) * (1 - k);
      const b = 2 * (1 - k) * k;
      const g = k * k;
      for (let i = 0; i < J; i++) this.cur[i] = a * this.from[i] + b * tr.mid[i] + g * this.goal[i];
      moving = true;
    } else if (bt < 1 && !this.walking && !tr) {
      const k = ease(Math.max(0, bt));
      for (let i = 0; i < J; i++) this.cur[i] = this.from[i] + (this.goal[i] - this.from[i]) * k;
      moving = true;
    } else if (this.walking) {
      const k = 1 - Math.exp(-dt * 18);
      for (let i = 0; i < J; i++) this.cur[i] += (this.goal[i] - this.cur[i]) * k;
    } else this.cur.set(this.goal);
    if (tr && (tt >= 1 || this.walking)) this.transit = null;
    this.apply();
    this.blink(now, live);
    if (live) moving = true;

    // Emotes: "!" pulses three times, thought dots step at 2 Hz, rain drizzles for 8 s.
    if (this.emoteSprite.visible) {
      const base = (0.34 * this.emoteScale) / this.look.scale;
      let s = base;
      if ((this.emote === 'alert' || this.emote === 'question') && !reducedMotion) {
        const t = (now - this.emoteAt) / 1000;
        if (t < 2.4) {
          s = base * (1 + 0.16 * Math.max(0, Math.sin(t * Math.PI * 2.5)));
          moving = true;
        }
      }
      if (this.emote === 'think') {
        const per = reducedMotion ? 1000 : 500;
        const k = Math.floor(now / per) % 3;
        const kind = (['think1', 'think2', 'think3'] as const)[k];
        const map = this.env.emotes(kind);
        if (this.emoteMat.map !== map) {
          this.emoteMat.map = map;
          this.emoteMat.needsUpdate = true;
        }
        nextAt = Math.min(nextAt, (Math.floor(now / per) + 1) * per);
      }
      this.emoteSprite.scale.set(s, s, 1);
      const y = NECK + HIP + HEAD_Y + HEAD_R + this.sitLift() + 0.26 + this.cur[0];
      this.emoteSprite.position.set(this.seated ? 0 : 0, y + (this.pose === 'raise' ? 0.12 : 0), this.seated ? 0.1 : 0);
    }
    if (this.cloud) {
      const t = now - this.cloudAt;
      const y = NECK + HIP + HEAD_Y + HEAD_R + this.sitLift() + 0.3;
      this.cloud.position.y = y;
      const live = t < 8000 && !reducedMotion;
      for (let i = 0; i < this.drops.length; i++) {
        const d = this.drops[i];
        const ph = live ? ((now / 450 + i * 0.37) % 1) : 0.5;
        d.position.y = -0.1 - ph * 0.28;
        d.visible = live || i % 2 === 0;
      }
      if (live) {
        // Drizzle in 4 Hz steps (not every frame).
        nextAt = Math.min(nextAt, (Math.floor(now / 250) + 1) * 250);
      }
    }
    return { moving, nextAt };
  }

  /** Glances: towards `gaze` when set, else wherever its eyes wander (smoothed like a real head). */
  private look_(now: number, dt: number) {
    const w = gazeWeight(this.pose);
    let yaw: number;
    let pitch: number;
    if (this.gaze) {
      const p = this.root.position;
      const eye = (this.seated ? this.seatY + 0.6 : 1.0) * this.look.scale;
      const dx = this.gaze.x - p.x;
      const dz = this.gaze.z - p.z;
      const r = this.root.rotation.y;
      const lx = dx * Math.cos(r) - dz * Math.sin(r);
      const lz = dx * Math.sin(r) + dz * Math.cos(r);
      yaw = Math.atan2(lx, lz);
      pitch = Math.atan2(eye - this.gaze.y, Math.hypot(lx, lz));
      if (Math.abs(yaw) > 1.7) yaw = 0; // behind it: not worth breaking its neck
    } else {
      if (now >= this.wanderAt) {
        this.wanderYaw = (this.rand() - 0.5) * 0.9;
        this.wanderPitch = (this.rand() - 0.5) * 0.24;
        this.wanderAt = now + 2500 + this.rand() * 4500;
      }
      yaw = this.wanderYaw;
      pitch = this.wanderPitch;
    }
    const k = 1 - Math.exp(-dt * 6);
    this.gazeYaw += (clamp(yaw, -1.15, 1.15) - this.gazeYaw) * k;
    this.gazePitch += (clamp(pitch, -0.45, 0.6) - this.gazePitch) * k;
    this.goal[3] += this.gazeYaw * w * 0.75;
    this.goal[13] += this.gazeYaw * w * 0.25;
    // The head carries the torso's lean: look at a target relative to it.
    if (this.gaze) this.goal[2] += (this.gazePitch - this.goal[1] - this.goal[2]) * w;
    else this.goal[2] += this.gazePitch * w * 0.5;
  }

  /** Blinks every few seconds (sometimes twice), only with open eyes and only while live. */
  private blink(now: number, live: boolean) {
    if (!live || this.eyes !== 0 || this.pose === 'sleep') {
      if (this.blinkUntil) {
        this.blinkUntil = 0;
        this.headMesh.geometry = this.heads[this.eyes];
      }
      return;
    }
    if (this.blinkAt < 0) this.blinkAt = now + this.rand() * 3000;
    if (this.blinkUntil && now >= this.blinkUntil) {
      this.blinkUntil = 0;
      this.headMesh.geometry = this.heads[0];
    } else if (!this.blinkUntil && now >= this.blinkAt) {
      this.headMesh.geometry = this.heads[1];
      this.blinkUntil = now + 110;
      this.blinkAt = now + (this.rand() < 0.15 ? 280 : 2200 + this.rand() * 3600);
    }
  }

  /** The joints → the scene graph. */
  /** How far the hips sit down (joint 23 blends 0 standing → 1 seated). */
  private sitLift() {
    return this.cur[23] * (this.seatY / this.look.scale + 0.04 - HIP);
  }

  private apply() {
    const c = this.cur;
    const y = HIP + this.sitLift() + c[0];
    this.body.position.y = y;
    this.body.rotation.set(c[1], c[13], c[20]);
    this.head.rotation.set(c[2], c[3], c[4]);
    this.armL.rotation.set(c[5], 0, c[6]);
    this.foreL.rotation.set(c[7], 0, 0);
    this.wristL.rotation.x = c[18];
    this.armR.rotation.set(c[8], 0, -c[9]);
    this.foreR.rotation.set(c[10], 0, 0);
    this.wristR.rotation.x = c[19];
    this.legL.position.y = y;
    this.legR.position.y = y;
    this.legL.rotation.set(c[11], 0, c[21]);
    this.legR.rotation.set(c[12], 0, -c[22]);
    this.kneeL.rotation.x = c[14];
    this.kneeR.rotation.x = c[15];
    this.ankleL.rotation.x = c[16];
    this.ankleR.rotation.x = c[17];
    if (this.propMesh.visible) {
      // Hold flat things up towards the face when reading; level otherwise. The pool cue lies
      // along the table when aiming and stands upright otherwise, whatever the arm does.
      const reading = !this.walking && (this.pose === 'read' || this.pose === 'review');
      let r = reading ? 0.9 : this.prop === 'phone' || this.prop === 'paddle' || (!this.prop && this.glance) ? 0 : this.prop === 'cue' ? 1.45 : 1.2;
      if (this.prop === 'cue' && !this.walking && this.pose === 'pool') {
        const aiming = this.cue?.phase === 'aim' || this.cue?.phase === 'strike';
        r = (aiming ? 1.67 : 0.08) - (c[1] + c[8] + c[10] + c[19]);
      }
      this.propMesh.rotation.set(r, 0, 0);
    }
  }

  /** A ghost (an agent let go, by the memory archive): see-through, cold, no shadow. */
  haunt(now: number) {
    this.mat.emissive.setRGB(0.1, 0.18, 0.32);
    this.mat.depthWrite = false;
    this.root.traverse((o) => (o.castShadow = false));
    this.fadeTo(0.34, now, 900);
  }

  setHighlight(on: boolean) {
    this.mat.emissive.setRGB(on ? 0.06 : 0, on ? 0.05 : 0, on ? 0.04 : 0);
  }

  dispose() {
    for (const g of this.geos) g.dispose();
    this.mat.dispose();
    this.emoteMat.dispose();
    for (const m of this.extraMats) m.dispose();
    this.root.removeFromParent();
  }
}
