// Characters: charming low-poly chibi people built procedurally from the agent's seed — skin,
// hair style and color, clothes in the agent's identity color, a small provider badge on the
// chest — with role silhouettes: the Lead wears a blazer and stands a bit taller, a reviewer
// wears a cardigan and round glasses, workers wear a tee with a hood or headphones, a helper
// ("intern", a subagent) is small and wears a cap in its parent's color.
//
// Animation is procedural and cheap: a few joints (hips, torso lean, head, shoulders, elbows,
// legs), a walk cycle while moving, pose blends on change, and stepped loops (typing, page
// flips, writing, waving) that tick on a shared clock so a busy office wakes a few times a
// second instead of every frame. A character standing or sitting still costs nothing.

import {
  BufferGeometry,
  Group,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  Object3D,
  Sprite,
  SpriteMaterial,
  TorusGeometry,
  Vector3,
  type Texture,
} from 'three';
import type { Provider } from '../../lib/gen/Provider';
import type { Role } from '../../lib/gen/Role';
import type { Emote, Pose, Prop } from '../behavior';
import { HAIR, HAIR_WEIGHTS, identity, lin, mix, PANTS, PROVIDER, SKIN, type RGB } from '../palette';
import { hash, rng } from '../text';
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
  const r = rng(hash(seed || 'agent'));
  const id = identity(color, 'light');
  const skin = lin(pick(r, SKIN));
  const hair = lin(pick(r, HAIR, HAIR_WEIGHTS));
  const style = Math.floor(r() * 8);
  const pants = lin(pick(r, PANTS));
  const acc = r();
  return {
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
  };
}

// ───────────── rig dimensions (scale 1) ─────────────

const HIP = 0.36;
const SHOULDER_Y = 0.285;
const SHOULDER_X = 0.175;
const NECK = 0.32;
const HEAD_R = 0.27;
const HEAD_Y = 0.24;
const UPPER = 0.14;
const FORE = 0.15;
const LEG_X = 0.085;

/** Joint vector: y, lean, headPitch, headYaw, headRoll, armLx, armLz, elbowL, armRx, armRz, elbowR, legL, legR, twist */
const J = 14;
type Joints = Float32Array;

const WHITE: RGB = [1, 1, 1];
const DARK_EYE: RGB = lin('#2a2026');

function badgeGlyph(b: Builder, provider: Provider, z: number, fg: RGB) {
  const s = 0.026;
  const star = (n: number, r0: number, r1: number, rot = 0) => {
    const pts: [number, number][] = [];
    for (let i = 0; i < n * 2; i++) {
      const a = rot + (i / (n * 2)) * Math.PI * 2;
      const rr = i % 2 ? r1 : r0;
      pts.push([Math.cos(a) * rr, Math.sin(a) * rr]);
    }
    b.poly(pts, z, fg);
  };
  switch (provider) {
    case 'claude':
      star(8, s, s * 0.35, Math.PI / 2);
      break;
    case 'gemini':
      star(4, s * 1.05, s * 0.28, Math.PI / 2);
      break;
    case 'codex': {
      const pts: [number, number][] = [];
      for (let i = 0; i < 6; i++) pts.push([Math.cos((i / 6) * Math.PI * 2 + Math.PI / 6) * s, Math.sin((i / 6) * Math.PI * 2 + Math.PI / 6) * s]);
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
      star(5, s, s * 0.45, Math.PI / 2);
  }
}

/** The merged meshes of one character (in rig-local coordinates). */
function buildParts(look: Look) {
  const L = look;
  const torso = new Builder();
  const top = L.role === 'lead' ? L.shirt : L.role === 'reviewer' ? L.shirt : L.shirt;
  // Pants (hips) and the torso.
  torso.cyl(0, -0.02, 0, 0.165, 0.11, L.pants, 14, 0.16);
  torso.cyl(0, 0.07, 0, 0.165, 0.2, top, 14, 0.15);
  torso.ball(0, 0.265, 0, 0.165, 0.075, 0.15, top, 14, 8);
  if (L.role === 'lead') {
    // Blazer lapels over a white shirt and a dark tie.
    torso.poly(
      [
        [-0.07, 0.3],
        [0.07, 0.3],
        [0, 0.12],
      ],
      0.151,
      lin('#f6f2ea'),
    );
    torso.poly(
      [
        [-0.018, 0.27],
        [0.018, 0.27],
        [0.012, 0.15],
        [0, 0.13],
        [-0.012, 0.15],
      ],
      0.153,
      lin('#2e2a33'),
    );
    torso.box(0, 0.075, 0.152, 0.012, 0.11, 0.004, L.shirtDark);
    torso.ball(0.105, 0.2, 0.135, 0.022, 0.012, 0.01, lin('#f6f2ea'), 6, 4);
  } else if (L.role === 'reviewer') {
    // Cardigan: a white collar, buttons down the front.
    torso.ball(0, 0.3, 0.02, 0.1, 0.035, 0.11, lin('#f6f2ea'), 10, 6);
    for (let i = 0; i < 3; i++) torso.ball(0, 0.22 - i * 0.06, 0.152, 0.011, 0.011, 0.006, lin('#efe6d4'), 6, 4);
    torso.box(0, 0.12, 0.151, 0.006, 0.26, 0.004, L.shirtDark);
  } else if (L.accessory === 'hood') {
    torso.ball(0, 0.3, -0.11, 0.12, 0.07, 0.07, L.shirtDark, 10, 6);
    torso.box(-0.035, 0.18, 0.15, 0.008, 0.1, 0.006, lin('#f1ede6'));
    torso.box(0.035, 0.18, 0.15, 0.008, 0.1, 0.006, lin('#f1ede6'));
  } else {
    torso.ball(0, 0.305, 0.02, 0.075, 0.025, 0.1, L.shirtDark, 10, 6);
  }
  if (L.intern) {
    // A tiny backpack.
    torso.box(0, 0.15, -0.17, 0.2, 0.2, 0.08, lin('#e0a458'));
  }
  // Provider badge on the left chest.
  const pv = PROVIDER[L.provider] ?? PROVIDER.claude;
  const bx = 0.075;
  const by = 0.205;
  const bz = 0.143;
  const badge = new Builder();
  badge.frame(bx, by, bz, 0);
  const disc: [number, number][] = [];
  for (let i = 0; i < 16; i++) disc.push([Math.cos((i / 16) * Math.PI * 2) * 0.042, Math.sin((i / 16) * Math.PI * 2) * 0.042]);
  badge.poly(disc, 0, lin(pv.bg));
  badgeGlyph(badge, L.provider, 0.002, lin(pv.fg));
  const ring: [number, number][] = [];
  for (let i = 0; i < 16; i++) ring.push([Math.cos((i / 16) * Math.PI * 2) * 0.05, Math.sin((i / 16) * Math.PI * 2) * 0.05]);
  torso.frame(bx, by, bz - 0.002, 0).poly(ring, 0, lin('#fbf8f1')).frame();

  // Head (three variants for the eyes: open, closed, happy).
  const heads = (['open', 'closed', 'happy'] as const).map((eyes) => {
    const h = new Builder();
    h.ball(0, HEAD_Y, 0, HEAD_R, HEAD_R * 0.96, HEAD_R * 0.94, L.skin, 18, 12);
    h.ball(0, 0.02, 0, 0.055, 0.05, 0.055, L.skin, 8, 6);
    // Ears.
    for (const sx of [-1, 1]) h.ball(sx * HEAD_R * 0.97, HEAD_Y - 0.01, -0.01, 0.035, 0.05, 0.03, mix(L.skin, [0, 0, 0], 0.06), 8, 6);
    const ey = HEAD_Y - 0.01;
    const ez = HEAD_R * 0.9;
    for (const sx of [-1, 1]) {
      if (eyes === 'open') {
        h.ball(sx * 0.085, ey, ez, 0.026, 0.038, 0.02, DARK_EYE, 8, 6);
        h.ball(sx * 0.085 + 0.008, ey + 0.014, ez + 0.012, 0.008, 0.008, 0.004, WHITE, 6, 4);
      } else if (eyes === 'closed') h.box(sx * 0.085, ey - 0.008, ez + 0.006, 0.05, 0.009, 0.01, DARK_EYE);
      else {
        h.box(sx * 0.085 - 0.014, ey + 0.002, ez + 0.006, 0.032, 0.009, 0.01, DARK_EYE, sx * 0.0);
        h.ball(sx * 0.085, ey + 0.006, ez + 0.004, 0.026, 0.016, 0.01, DARK_EYE, 8, 4);
      }
      if (L.blush) h.ball(sx * 0.13, ey - 0.055, ez - 0.03, 0.03, 0.016, 0.01, mix(L.skin, lin('#ff8a7a'), 0.45), 8, 4);
    }
    // Mouth.
    h.box(0, ey - 0.075, ez + 0.012, eyes === 'happy' ? 0.05 : 0.032, 0.01, 0.01, lin('#7a3b34'));
    hair(h, L);
    if (L.role === 'reviewer') glasses(h);
    if (L.accessory === 'headphones') headphones(h, L);
    if (L.intern) cap(h, L);
    return h.build();
  });

  // Arms: upper (sleeve) and forearm (skin + hand), hanging down from their pivots.
  const sleeve = L.role === 'lead' || L.role === 'reviewer' ? L.shirt : L.shirt;
  const upper = new Builder();
  upper.cyl(0, -UPPER - 0.01, 0, 0.052, UPPER + 0.03, sleeve, 10, 0.06);
  upper.ball(0, 0, 0, 0.062, 0.06, 0.062, sleeve, 10, 6);
  const fore = new Builder();
  const longSleeve = L.role === 'lead' || L.role === 'reviewer' || L.accessory === 'hood';
  if (longSleeve) {
    fore.cyl(0, -FORE + 0.03, 0, 0.044, FORE - 0.02, sleeve, 10, 0.05);
    fore.cyl(0, -FORE + 0.02, 0, 0.047, 0.03, L.role === 'lead' ? lin('#f6f2ea') : L.shirtDark, 10);
  } else fore.cyl(0, -FORE + 0.02, 0, 0.038, FORE - 0.01, L.skin, 10, 0.042);
  fore.ball(0, -FORE - 0.005, 0.005, 0.05, 0.052, 0.045, L.skin, 10, 8);

  // Legs (with shoes).
  const leg = new Builder();
  leg.cyl(0, -HIP + 0.055, 0, 0.06, HIP - 0.05, L.pants, 10, 0.07);
  leg.ball(0, -HIP + 0.035, 0.03, 0.065, 0.04, 0.095, L.shoes, 10, 6);
  return { torso: torso.build(), badge: badge.build(), heads, upper: upper.build(), fore: fore.build(), leg: leg.build() };
}

function hair(h: Builder, L: Look) {
  const c = L.hair;
  const style = L.style;
  if (style === 7) {
    // Buzz cut.
    h.ball(0, HEAD_Y + 0.025, -0.02, HEAD_R + 0.008, HEAD_R * 0.9, HEAD_R * 0.92, c, 16, 10);
    return;
  }
  // The cap of hair sits a little back and up, so the face stays clear.
  h.ball(0, HEAD_Y + 0.05, -0.035, HEAD_R + 0.018, HEAD_R * 0.93, HEAD_R + 0.008, c, 16, 10);
  if (style !== 6) {
    // Fringe.
    const side = style % 2 ? 1 : -1;
    h.ball(side * 0.05, HEAD_Y + 0.13, 0.15, 0.17, 0.07, 0.085, c, 12, 8);
    h.ball(-side * 0.1, HEAD_Y + 0.12, 0.13, 0.1, 0.06, 0.07, c, 10, 6);
  }
  switch (style) {
    case 1: // bob
      for (const sx of [-1, 1]) h.ball(sx * 0.19, HEAD_Y - 0.03, -0.03, 0.09, 0.17, 0.16, c, 10, 8);
      break;
    case 2: // bun
      h.ball(0, HEAD_Y + 0.26, -0.1, 0.095, 0.09, 0.095, c, 10, 8);
      break;
    case 3: // spiky
      for (let i = 0; i < 6; i++) {
        const a = (i / 6) * Math.PI * 2;
        h.cyl(Math.cos(a) * 0.1, HEAD_Y + 0.18, Math.sin(a) * 0.1 - 0.03, 0.06, 0.13, c, 5, 0.01);
      }
      break;
    case 4: // long
      for (const sx of [-1, 1]) h.ball(sx * 0.18, HEAD_Y - 0.08, -0.06, 0.09, 0.22, 0.15, c, 10, 8);
      h.ball(0, HEAD_Y - 0.12, -0.15, 0.2, 0.24, 0.11, c, 12, 8);
      break;
    case 5: // ponytail
      h.ball(0, HEAD_Y + 0.06, -0.26, 0.07, 0.07, 0.07, c, 10, 8);
      h.ball(0, HEAD_Y - 0.08, -0.3, 0.06, 0.15, 0.06, c, 10, 8);
      break;
    case 6: // curls
      for (let i = 0; i < 12; i++) {
        const a = (i / 12) * Math.PI * 2;
        const y = HEAD_Y + 0.1 + (i % 3) * 0.05;
        h.blob(Math.cos(a) * 0.19, y, Math.sin(a) * 0.17 - 0.03, 0.09, 0.09, 0.09, c, 0, a);
      }
      h.blob(0, HEAD_Y + 0.24, -0.02, 0.15, 0.08, 0.14, c, 0);
      break;
  }
}

function glasses(h: Builder) {
  const frame = lin('#3a2a24');
  const tor = new TorusGeometry(0.048, 0.008, 6, 16);
  const g = tor.toNonIndexed();
  for (const sx of [-1, 1]) {
    const m = new Object3D();
    m.position.set(sx * 0.085, HEAD_Y - 0.01, HEAD_R * 0.93);
    m.updateMatrix();
    h.add(g, frame, m.matrix);
  }
  h.box(0, HEAD_Y, HEAD_R * 0.95, 0.06, 0.008, 0.008, frame);
  tor.dispose();
  g.dispose();
}

function headphones(h: Builder, L: Look) {
  const band = lin('#2f3138');
  const tor = new TorusGeometry(HEAD_R + 0.03, 0.016, 6, 20, Math.PI);
  const g = tor.toNonIndexed();
  const m = new Object3D();
  m.position.set(0, HEAD_Y, -0.01);
  m.updateMatrix();
  h.add(g, band, m.matrix);
  for (const sx of [-1, 1]) h.cyl(sx * (HEAD_R + 0.005), HEAD_Y - 0.06, -0.01, 0.06, 0.05, sx < 0 ? L.shirtDark : band, 12);
  tor.dispose();
  g.dispose();
}

function cap(h: Builder, L: Look) {
  h.ball(0, HEAD_Y + 0.07, -0.01, HEAD_R + 0.025, HEAD_R * 0.75, HEAD_R + 0.02, L.shirt, 16, 8);
  h.box(0, HEAD_Y + 0.08, HEAD_R + 0.04, 0.24, 0.02, 0.16, L.shirtDark);
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
    this.geos.set('cue', mk((b) => b.cyl(0, -0.45, 0.02, 0.016, 1.1, lin('#c9a46a'), 6, 0.008).cyl(0, -0.45, 0.02, 0.018, 0.3, lin('#3a2a20'), 6)));
  }
  dispose() {
    for (const g of this.geos.values()) g.dispose();
  }
}

// ───────────── poses ─────────────

const TICK = 150;

/** Stepped-loop period (ms) per pose; 0 = static. */
function loopPeriod(pose: Pose): number {
  switch (pose) {
    case 'type':
      return TICK;
    case 'write':
    case 'gamepad':
      return TICK * 2;
    case 'browse':
    case 'raise':
    case 'talk':
    case 'play':
      return TICK * 3;
    case 'read':
    case 'review':
    case 'phone':
      return TICK * 4;
    default:
      return 0;
  }
}

const SIT_POSES = new Set<Pose>(['sit', 'type', 'read', 'think', 'phone', 'sleep', 'review', 'gamepad']);

function targets(out: Joints, pose: Pose, seated: boolean, step: number) {
  out.fill(0);
  const legs = seated ? -1.5 : 0;
  out[11] = legs;
  out[12] = legs;
  // Relaxed arms by default.
  out[5] = 0.02;
  out[6] = 0.08;
  out[7] = -0.12;
  out[8] = 0.02;
  out[9] = 0.08;
  out[10] = -0.12;
  const s = step;
  switch (pose) {
    case 'sit':
      out[1] = 0.02;
      out[5] = out[8] = -0.35;
      out[7] = out[10] = -0.85;
      out[6] = out[9] = 0.05;
      out[2] = 0.05;
      break;
    case 'type':
      out[1] = 0.12;
      out[5] = out[8] = -0.5;
      out[6] = out[9] = -0.06;
      out[7] = -1.05 + (s % 2 ? 0.1 : -0.04);
      out[10] = -1.05 + (s % 2 ? -0.04 : 0.1);
      out[2] = 0.2;
      out[3] = ((s >> 3) % 3) * 0.06 - 0.06;
      break;
    case 'read':
    case 'review':
      out[1] = 0.06;
      out[5] = out[8] = -0.62;
      out[6] = out[9] = -0.2;
      out[7] = out[10] = -1.15;
      out[10] += s % 2 ? -0.08 : 0;
      out[2] = 0.32;
      out[4] = s % 4 === 3 ? 0.05 : 0;
      break;
    case 'think':
      out[1] = -0.03;
      out[8] = -0.95;
      out[9] = -0.32;
      out[10] = -2.25;
      out[5] = -0.55;
      out[6] = -0.25;
      out[7] = -1.25;
      out[2] = -0.1;
      out[4] = 0.12;
      break;
    case 'phone':
      out[8] = -0.3;
      out[9] = 0.22;
      out[10] = -2.6;
      out[5] = -0.4;
      out[7] = -0.9;
      out[2] = s % 2 ? 0.08 : -0.02;
      out[4] = 0.16;
      break;
    case 'sleep':
      out[1] = 0.6;
      out[5] = out[8] = -1.05;
      out[6] = out[9] = -0.3;
      out[7] = out[10] = -1.25;
      out[2] = 0.5;
      out[4] = 0.25;
      break;
    case 'slump':
      out[1] = 0.24;
      out[2] = 0.5;
      out[5] = out[8] = seated ? -0.25 : -0.05;
      out[7] = out[10] = seated ? -0.5 : -0.05;
      out[6] = out[9] = 0.02;
      out[0] = seated ? 0 : -0.02;
      break;
    case 'stand':
      break;
    case 'browse':
      out[8] = s % 2 ? -2.0 : -2.3;
      out[9] = 0.08;
      out[10] = -0.25;
      out[5] = -0.55;
      out[6] = -0.12;
      out[7] = -1.2;
      out[2] = -0.18;
      out[3] = s % 4 < 2 ? 0.08 : -0.06;
      break;
    case 'write':
      out[8] = -2.0;
      out[9] = s % 2 ? 0.26 : 0.06;
      out[10] = -0.5;
      out[5] = -0.15;
      out[2] = -0.12;
      out[3] = s % 2 ? 0.05 : -0.03;
      break;
    case 'watch':
      out[5] = out[8] = -0.45;
      out[6] = out[9] = -0.38;
      out[7] = out[10] = -1.75;
      out[2] = -0.14;
      break;
    case 'raise':
      out[8] = -2.95;
      out[9] = s % 2 ? 0.32 : 0.1;
      out[10] = -0.15;
      out[2] = -0.08;
      out[5] = -0.05;
      break;
    case 'wave':
      out[8] = -2.7;
      out[9] = s % 2 ? 0.4 : 0.1;
      out[10] = -0.5;
      out[2] = -0.05;
      break;
    case 'celebrate':
      out[5] = out[8] = -2.85;
      out[6] = out[9] = 0.38;
      out[7] = out[10] = -0.12;
      out[2] = -0.15;
      break;
    case 'drop':
      out[1] = 0.14;
      out[8] = -1.2;
      out[10] = -0.25;
      out[2] = 0.18;
      break;
    case 'gamepad':
      // Both hands forward on a pad, thumbs tapping.
      out[1] = 0.1;
      out[5] = out[8] = -0.75;
      out[6] = out[9] = -0.3;
      out[7] = -1.0 + (s % 2 ? 0.06 : 0);
      out[10] = -1.0 + (s % 3 === 1 ? 0.06 : 0);
      out[2] = 0.08;
      break;
    case 'play':
      // Leaning over the table, both arms out, twisting the rods (or lining up a shot).
      out[1] = 0.32;
      out[5] = out[8] = -1.2;
      out[6] = out[9] = -0.1;
      out[7] = out[10] = -0.35;
      out[13] = s % 2 ? 0.14 : -0.14;
      out[2] = -0.25;
      break;
    case 'talk':
      out[8] = s % 2 ? -0.7 : -0.95;
      out[9] = 0.2;
      out[10] = -0.95;
      out[5] = -0.2;
      out[7] = -0.5;
      out[3] = s % 2 ? 0.08 : -0.05;
      break;
  }
}

// ───────────── the character ─────────────

const ease = (t: number) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2);
const BLEND_MS = 240;
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
  readonly hand = new Object3D();
  private propMesh: Mesh;
  private mat: MeshStandardMaterial;
  private geos: BufferGeometry[] = [];
  private heads: BufferGeometry[];
  private eyes: 0 | 1 | 2 = 0;
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

  constructor(id: string, look: Look, env: CharacterEnv) {
    this.id = id;
    this.look = look;
    this.env = env;
    this.mat = new MeshStandardMaterial({ vertexColors: true, roughness: 0.78, metalness: 0, transparent: false });
    const parts = buildParts(look);
    this.heads = parts.heads;
    this.geos.push(parts.torso, parts.badge, ...parts.heads, parts.upper, parts.fore, parts.leg);
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
    for (const [arm, fore, sx] of [
      [this.armL, this.foreL, 1],
      [this.armR, this.foreR, -1],
    ] as const) {
      arm.position.set(sx * SHOULDER_X, SHOULDER_Y, 0);
      this.body.add(arm);
      mesh(parts.upper, arm);
      fore.position.y = -UPPER;
      arm.add(fore);
      mesh(parts.fore, fore);
    }
    for (const [leg, sx] of [
      [this.legL, 1],
      [this.legR, -1],
    ] as const) {
      leg.position.set(sx * LEG_X, HIP, 0);
      this.root.add(leg);
      mesh(parts.leg, leg);
    }
    this.hand.position.set(0, -FORE - 0.02, 0.02);
    this.foreR.add(this.hand);
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
    targets(this.goal, 'stand', false, 0);
    this.cur.set(this.goal);
    this.apply();
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
    this.pose = pose;
    this.seated = seated;
    if (seatY != null) this.seatY = seatY;
    this.from.set(this.cur);
    this.blendAt = now;
    if (pose === 'celebrate') this.jumpUntil = now + 1500;
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
    this.eyes = e === 'zz' ? 1 : e === 'check' ? 2 : 0;
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
   * Advance by dt. Returns whether it needs continuous frames (walking, blending, fading,
   * jumping) and when its next stepped loop tick is due (Infinity when static).
   */
  update(now: number, dt: number, reducedMotion: boolean): { moving: boolean; nextAt: number } {
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
      let remaining = this.speed * dt;
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
      this.phase += ((this.speed * dt) / STRIDE) * Math.PI * 2;
    }

    // Turning.
    let dy = this.yawGoal - this.root.rotation.y;
    dy = Math.atan2(Math.sin(dy), Math.cos(dy));
    if (Math.abs(dy) > 0.004) {
      const k = 1 - Math.exp(-dt * (this.walking ? 14 : 10));
      this.root.rotation.y += dy * k;
      moving = true;
    } else this.root.rotation.y = this.yawGoal;

    // Pose targets (stepped loops tick on the shared clock).
    const period = this.walking ? 0 : loopPeriod(this.pose) * (reducedMotion ? 4 : 1);
    if (period) {
      this.step = Math.floor(now / period);
      nextAt = (this.step + 1) * period;
    }
    if (this.walking) {
      targets(this.goal, 'stand', false, 0);
      const s = Math.sin(this.phase);
      this.goal[11] = s * 0.6;
      this.goal[12] = -s * 0.6;
      this.goal[5] = -s * 0.5;
      this.goal[8] = s * 0.5;
      this.goal[7] = this.goal[10] = -0.35;
      this.goal[1] = 0.06;
      this.goal[0] = Math.abs(Math.cos(this.phase)) * 0.03;
      if (this.prop) {
        // Carry in front.
        this.goal[8] = -0.55;
        this.goal[10] = -1.1;
        this.goal[9] = -0.1;
      }
    } else targets(this.goal, this.pose, this.seated, this.step);
    if (this.jumpUntil > now && !reducedMotion) {
      this.goal[0] += Math.abs(Math.sin((now / 1000) * 8.5)) * 0.14;
      moving = true;
    }

    // Blend into a new pose, then follow the goal exactly (no per-frame drift when still).
    const bt = (now - this.blendAt) / BLEND_MS;
    if (bt < 1 && !this.walking) {
      const k = ease(Math.max(0, bt));
      for (let i = 0; i < J; i++) this.cur[i] = this.from[i] + (this.goal[i] - this.from[i]) * k;
      moving = true;
    } else if (this.walking) {
      const k = 1 - Math.exp(-dt * 18);
      for (let i = 0; i < J; i++) this.cur[i] += (this.goal[i] - this.cur[i]) * k;
    } else this.cur.set(this.goal);
    this.apply();

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
      const y = NECK + HIP + HEAD_Y + HEAD_R + (this.seated ? this.seatY / this.look.scale + 0.04 - HIP : 0) + 0.26 + this.cur[0];
      this.emoteSprite.position.set(this.seated ? 0 : 0, y + (this.pose === 'raise' ? 0.12 : 0), this.seated ? 0.1 : 0);
    }
    if (this.cloud) {
      const t = now - this.cloudAt;
      const y = NECK + HIP + HEAD_Y + HEAD_R + (this.seated ? this.seatY / this.look.scale + 0.04 - HIP : 0) + 0.3;
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

  /** The joints → the scene graph. */
  private apply() {
    const c = this.cur;
    const sitLift = this.seated ? this.seatY / this.look.scale + 0.04 - HIP : 0;
    this.body.position.y = HIP + sitLift + c[0];
    this.body.rotation.set(c[1], c[13], 0);
    this.head.rotation.set(c[2], c[3], c[4]);
    this.armL.rotation.set(c[5], 0, c[6]);
    this.foreL.rotation.set(c[7], 0, 0);
    this.armR.rotation.set(c[8], 0, -c[9]);
    this.foreR.rotation.set(c[10], 0, 0);
    this.legL.position.y = HIP + sitLift + c[0];
    this.legR.position.y = HIP + sitLift + c[0];
    this.legL.rotation.x = c[11];
    this.legR.rotation.x = c[12];
    if (this.propMesh.visible) {
      // Hold flat things up towards the face when reading; level otherwise.
      const reading = !this.walking && (this.pose === 'read' || this.pose === 'review');
      this.propMesh.rotation.set(reading ? 0.9 : this.prop === 'phone' ? 0 : this.prop === 'cue' ? 1.45 : 1.2, 0, 0);
    }
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
