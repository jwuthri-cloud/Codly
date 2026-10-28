// Characters: charming low-poly chibi people built procedurally from the agent's seed — skin,
// hair style and color, clothes in the agent's identity color, a small provider badge on the
// chest — with role silhouettes: the Lead wears a blazer and stands a bit taller, a reviewer
// wears a cardigan and round glasses, workers wear a tee with a hood or headphones, a helper
// ("intern", a subagent) is small and wears a cap in its parent's color.
//
// Animation is procedural: a few joints (hips, torso lean, head, shoulders, elbows, legs), a walk
// cycle while moving, pose blends on change, and each pose's own motion as continuous curves
// (gfx/motion.ts). While the office is live (on screen, window focused, motion allowed) every
// character also breathes, blinks, shifts its weight and glances around (or at what the engine
// points it to: a ball, the TV, someone walking by). Otherwise poses tick in steps on a shared
// clock and a character standing or sitting still costs nothing.

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
import { addLife, gazeWeight, J, type Joints, loopPeriod, type PlayCue, REACT_S, reactPose, type ReactKind, SIT_POSES, targets } from './motion';
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
/** Leg segments: thigh (hip → knee) and shin (knee → ankle); the shoe hangs at the ankle. */
const THIGH = 0.17;
const SHIN = HIP - THIGH - 0.03;

/** Joint vector (gfx/motion.ts): y, lean, headPitch, headYaw, headRoll, armLx, armLz, elbowL, armRx, armRz, elbowR, legL, legR, twist */

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
  // The hand hangs at the wrist, so it can bend (a pad, a cue, a rod handle).
  const hand = new Builder();
  hand.ball(0, -0.005, 0.005, 0.05, 0.052, 0.045, L.skin, 10, 8);

  // Legs: thigh and shin (the knee bends) and the shoe at the ankle.
  const thigh = new Builder();
  thigh.cyl(0, -THIGH, 0, 0.062, THIGH + 0.01, L.pants, 10, 0.07);
  const shin = new Builder();
  shin.cyl(0, -SHIN, 0, 0.056, SHIN + 0.01, L.pants, 10, 0.062);
  shin.ball(0, 0, 0, 0.062, 0.05, 0.062, L.pants, 10, 6);
  const shoe = new Builder();
  shoe.ball(0, 0.005, 0.03, 0.065, 0.04, 0.095, L.shoes, 10, 6);
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

// ───────────── the character ─────────────

const ease = (t: number) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2);
const clamp = (x: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, x));
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
  /** Where to look (world), set by the engine; null = its own glances. */
  gaze: Vector3 | null = null;
  /** Its game's cue (gfx/games.ts); null when not playing. */
  cue: PlayCue | null = null;
  /** A personal reaction playing over its pose (traits.ts), started at `at`. */
  private reacting: { kind: ReactKind; at: number } | null = null;

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
    this.mat = new MeshStandardMaterial({ vertexColors: true, roughness: 0.78, metalness: 0, transparent: false });
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
    return { t: (tMs / 1000) * this.tempo + this.offset, now, seed: this.seed, cue: this.cue, live, lazy: this.lazy };
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
      if (this.prop) {
        // Carry in front.
        this.goal[8] = -0.55;
        this.goal[10] = -1.1;
        this.goal[9] = -0.1;
      }
    } else {
      targets(this.goal, this.pose, this.seated, this.ctx(tMs, now, live));
      if (live) {
        addLife(this.goal, this.pose, this.seated, now / 1000, this.seed, this.breathPeriod);
        this.look_(now, dt);
      }
      if (this.reacting) {
        const age = (now - this.reacting.at) / 1000;
        const dur = REACT_S[this.reacting.kind];
        if (age >= dur) this.reacting = null;
        else {
          reactPose(this.goal, this.reacting.kind, Math.min(1, age / 0.15) * Math.min(1, (dur - age) / 0.35), now / 1000);
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
      let r = reading ? 0.9 : this.prop === 'phone' ? 0 : this.prop === 'cue' ? 1.45 : 1.2;
      if (this.prop === 'cue' && !this.walking && this.pose === 'pool') {
        const aiming = this.cue?.phase === 'aim' || this.cue?.phase === 'strike';
        r = (aiming ? 1.67 : 0.08) - (c[1] + c[8] + c[10] + c[19]);
      }
      this.propMesh.rotation.set(r, 0, 0);
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
