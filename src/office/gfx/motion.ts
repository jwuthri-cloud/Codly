// Motion: what each joint does over time, as continuous curves (no stepped flips between two
// angles). Pure functions of a character's own clock and seed, so two characters never move in
// sync and the same moment always looks the same (tests, replays).
//
// Joints, shared with character.ts (radians unless noted):
//   0 body bob (m)          1 torso pitch (+ forward)     2 head pitch (+ down)
//   3 head yaw (+ its left)  4 head roll                   5 left arm pitch (− forward, up)
//   6 left arm out           7 left forearm (− bends)      8 right arm pitch
//   9 right arm out         10 right forearm              11 left leg   12 right leg (− forward)
//  13 torso yaw (+ its left)
//  14 left knee 15 right knee (+ bends)   16 left ankle 17 right ankle (+ toes down)
//  18 left wrist 19 right wrist (− flexes up)   20 pelvis sway (roll)
//  21 left leg out 22 right leg out   23 seated (0 standing → 1 hips on the seat)

import type { Pose } from '../behavior';
import CLIPS from './clips.json';

export const J = 24;
export type Joints = Float32Array;

/** What a game tells its player each frame (gfx/games.ts). */
export interface PlayCue {
  /** 0 calm … 1 the ball is at its rods, the last minute of the match. */
  intensity: number;
  /** −1 … 1: lean or steer towards its right … its left. */
  lean: number;
  /** Pool: where the shooter is in its turn (null for other games). */
  phase: 'aim' | 'strike' | 'watch' | 'wait' | null;
  /** 0 … 1 through `phase`. */
  t: number;
  /** The latest reaction and when it started (ms, performance clock). */
  react: { kind: 'cheer' | 'groan' | 'shot'; at: number } | null;
}

export interface MotionCtx {
  /** Seconds on this character's own clock (its tempo and offset applied). */
  t: number;
  /** The real clock (ms, performance.now), for reactions. */
  now: number;
  seed: number;
  cue: PlayCue | null;
  /** Continuous mode: fidgets play (never in stepped mode, where they would freeze mid-way). */
  live: boolean;
  /** Lazy (traits.ts): stretches and yawns about twice as often. */
  lazy?: boolean;
}

const TAU = Math.PI * 2;
export const osc = (t: number, hz: number, ph = 0) => Math.sin(TAU * (hz * t + ph));

function h01(i: number, seed: number): number {
  let h = Math.imul(i | 0, 374761393) ^ Math.imul(seed | 0, 668265263);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

/** Smooth value noise in −1 … 1 (about one change per unit of x). */
export function noise(x: number, seed: number): number {
  const i = Math.floor(x);
  const f = x - i;
  const u = f * f * (3 - 2 * f);
  const a = h01(i, seed);
  return (a + (h01(i + 1, seed) - a) * u) * 2 - 1;
}

/** 0 → 1 → 0 bump for something that happens about every `every` s and lasts `len` s. */
export function pulse(t: number, every: number, len: number, seed: number): number {
  const k = Math.floor(t / every);
  const start = k * every + h01(k, seed) * Math.max(0, every - len);
  const p = (t - start) / len;
  return p > 0 && p < 1 ? Math.sin(Math.PI * p) ** 2 : 0;
}

const clamp01 = (x: number) => Math.min(1, Math.max(0, x));
const easeOut = (x: number) => 1 - (1 - clamp01(x)) ** 3;
/** Move joint i towards v by weight k (0 keeps it, 1 replaces it). */
const towards = (out: Joints, i: number, v: number, k: number) => {
  out[i] += (v - out[i]) * k;
};

/** A reaction's weight now: in fast, holds, eases out over `dur` seconds. */
export function reaction(cue: PlayCue | null, kind: 'cheer' | 'groan' | 'shot', now: number, dur: number): number {
  const r = cue?.react;
  if (!r || r.kind !== kind) return 0;
  const age = (now - r.at) / 1000;
  if (age < 0 || age > dur) return 0;
  return Math.min(1, age / 0.15) * Math.min(1, (dur - age) / 0.45);
}

export const SIT_POSES = new Set<Pose>(['sit', 'type', 'read', 'think', 'phone', 'sleep', 'review', 'gamepad']);

/** How much the head follows glances in a pose (a reader keeps its eyes on the page). */
export function gazeWeight(pose: Pose): number {
  switch (pose) {
    case 'sleep':
    case 'drop':
      return 0;
    case 'type':
      return 0.35;
    case 'read':
    case 'review':
    case 'write':
    case 'browse':
    case 'phone':
    case 'think':
      return 0.25;
    default:
      return 1;
  }
}

const TICK = 150;
/** Stepped-loop period (ms) when the office is not live (reduced motion); 0 = still. */
export function loopPeriod(pose: Pose): number {
  switch (pose) {
    case 'type':
      return TICK;
    case 'write':
    case 'gamepad':
      return TICK * 2;
    case 'browse':
    case 'raise':
    case 'talk':
    case 'foosball':
    case 'pool':
    case 'wave':
    case 'celebrate':
    case 'dance':
      return TICK * 3;
    case 'read':
    case 'review':
    case 'phone':
    case 'drink':
      return TICK * 4;
    default:
      return 0;
  }
}

/** The pose at time x.t: base angles plus its own continuous motion. */
export function targets(out: Joints, pose: Pose, seated: boolean, x: MotionCtx) {
  out.fill(0);
  // Legs: seated = thighs forward, knees bent, feet flat; standing = soft knees.
  out[11] = out[12] = seated ? -1.5 : 0;
  out[14] = out[15] = seated ? 1.45 : 0.04;
  out[16] = out[17] = seated ? 0.05 : -0.04;
  out[21] = out[22] = seated ? 0.07 : 0.02;
  out[23] = seated ? 1 : 0;
  // Relaxed arms by default.
  out[5] = 0.02;
  out[6] = 0.08;
  out[7] = -0.12;
  out[8] = 0.02;
  out[9] = 0.08;
  out[10] = -0.12;
  const { t, seed } = x;
  const n = (k: number, hz: number) => noise(t * hz, seed + k * 101);
  switch (pose) {
    case 'sit':
      out[1] = 0.02;
      out[5] = out[8] = -0.35;
      out[7] = out[10] = -0.85;
      out[6] = out[9] = 0.05;
      out[2] = 0.05;
      fidget(out, x, true);
      break;
    case 'stand':
      fidget(out, x, false);
      break;
    case 'type': {
      // Typing comes in bursts with pauses; now and then a hand goes to the mouse.
      const burst = clamp01(0.55 + 0.7 * n(1, 0.32));
      const k = 0.075 * burst;
      out[1] = 0.12;
      out[5] = out[8] = -0.5;
      out[6] = out[9] = -0.06;
      out[18] = out[19] = -0.22; // hands on the keys
      out[7] = -1.05 + k * osc(t, 5.3);
      out[10] = -1.05 + k * osc(t, 5.3, 0.5) + 0.02 * n(2, 3);
      out[2] = 0.2 + 0.035 * n(3, 0.45);
      out[3] = 0.07 * n(4, 0.18);
      const mouse = pulse(t, 7.5, 1.8, seed + 5);
      out[9] -= 0.28 * mouse;
      out[10] += 0.3 * mouse;
      break;
    }
    case 'read':
    case 'review': {
      // Eyes run along the lines; a page turns every few seconds.
      out[1] = 0.06;
      out[5] = out[8] = -0.62;
      out[6] = out[9] = -0.2;
      out[7] = out[10] = -1.15;
      out[18] = out[19] = -0.35;
      out[2] = 0.32 + 0.025 * n(1, 0.3);
      out[3] = 0.05 * osc(t, 0.17);
      const flip = pulse(t, 5.5, 0.7, seed + 7);
      out[10] -= 0.32 * flip;
      out[9] += 0.28 * flip;
      out[4] = 0.04 * flip;
      break;
    }
    case 'think':
      out[1] = -0.03;
      out[8] = -0.95;
      out[9] = -0.32;
      out[10] = -2.25 + 0.04 * osc(t, 1.2); // tapping the chin
      out[5] = -0.55;
      out[6] = -0.25;
      out[7] = -1.25;
      out[2] = -0.1 + 0.06 * n(1, 0.15);
      out[4] = 0.12 + 0.05 * n(2, 0.2);
      break;
    case 'phone':
      out[8] = -0.3;
      out[9] = 0.22;
      out[10] = -2.6;
      out[5] = -0.4 + 0.18 * n(1, 0.55); // the free hand talks
      out[7] = -0.9 + 0.2 * n(2, 0.7);
      out[2] = 0.03 + 0.05 * osc(t, 0.7);
      out[3] = 0.1 * n(3, 0.15);
      out[4] = 0.16;
      break;
    case 'sleep':
      out[1] = 0.6 + 0.03 * osc(t, 0.2);
      out[5] = out[8] = -1.05;
      out[6] = out[9] = -0.3;
      out[7] = out[10] = -1.25;
      out[2] = 0.5 + 0.02 * osc(t, 0.2, 0.15);
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
    case 'browse':
      // Reaching along a shelf, eyes on the spines.
      out[8] = -2.15 + 0.22 * n(1, 0.4);
      out[9] = 0.08 + 0.06 * n(2, 0.5);
      out[10] = -0.25;
      out[5] = -0.55;
      out[6] = -0.12;
      out[7] = -1.2;
      out[2] = -0.18 + 0.08 * n(3, 0.3);
      out[3] = 0.16 * n(4, 0.25);
      break;
    case 'write': {
      const stroke = osc(t, 1.7);
      out[8] = -2.0 + 0.08 * osc(t, 0.9);
      out[9] = 0.16 + 0.1 * stroke;
      out[10] = -0.5 + 0.08 * osc(t, 1.7, 0.25);
      out[5] = -0.15;
      out[2] = -0.12;
      out[3] = 0.05 * stroke;
      break;
    }
    case 'watch':
      out[5] = out[8] = -0.45;
      out[6] = out[9] = -0.38;
      out[7] = out[10] = -1.75;
      out[2] = -0.14;
      break;
    case 'raise':
      out[8] = -2.95;
      out[9] = 0.21 + 0.11 * osc(t, 1.3);
      out[10] = -0.15;
      out[2] = -0.08;
      out[5] = -0.05;
      break;
    case 'wave':
      clip(out, 'wave', t, 1, seated);
      break;
    case 'celebrate':
      clip(out, 'cheer', t, 1, seated);
      break;
    case 'dance':
      clip(out, 'dance', t, 1, seated);
      break;
    case 'drop':
      out[1] = 0.14;
      out[8] = -1.2;
      out[10] = -0.25;
      out[2] = 0.18;
      break;
    case 'talk':
      out[8] = -0.82 + 0.2 * n(1, 0.9);
      out[9] = 0.2 + 0.07 * n(2, 0.7);
      out[10] = -0.95 + 0.22 * n(3, 1.1);
      out[5] = -0.2 + 0.14 * n(4, 0.6);
      out[7] = -0.5 + 0.16 * n(5, 0.8);
      out[2] = 0.04 * osc(t, 1.1);
      out[3] = 0.09 * n(6, 0.4);
      break;
    case 'drink': {
      // A mug at the chest; a sip every few seconds.
      const sip = pulse(t, 6.5, 1.4, seed + 13);
      out[8] = -0.6 - 0.45 * sip;
      out[9] = 0.05;
      out[10] = -1.6 - 0.75 * sip;
      out[2] = -0.02 - 0.18 * sip;
      out[5] = -0.1;
      out[7] = -0.3;
      fidget(out, x, false);
      break;
    }
    case 'gamepad':
      gamepad(out, x);
      break;
    case 'foosball':
      foosball(out, x);
      break;
    case 'pool':
      pool(out, x);
      break;
  }
}

/** Little things people do when nothing happens: stretch, scratch their head, roll their neck. */
function fidget(out: Joints, x: MotionCtx, seated: boolean) {
  if (!x.live) return;
  const stretch = pulse(x.t, x.lazy ? 11 : 26, 2.6, x.seed + 21);
  const scratch = pulse(x.t + 9, 19, 1.6, x.seed + 22);
  const neck = pulse(x.t + 4, 31, 1.8, x.seed + 23);
  // Blended, not exclusive: when two overlap one fades into the other instead of snapping.
  if (stretch > 0) {
    towards(out, 5, -2.9, stretch);
    towards(out, 8, -2.9, stretch);
    towards(out, 6, 0.35, stretch);
    towards(out, 9, 0.35, stretch);
    towards(out, 7, -0.25, stretch);
    towards(out, 10, -0.25, stretch);
    towards(out, 1, seated ? -0.12 : -0.06, stretch);
    towards(out, 2, -0.2, stretch);
  }
  if (scratch > 0) {
    towards(out, 8, -2.5, scratch);
    towards(out, 9, 0.5, scratch);
    towards(out, 10, -2.1 + 0.12 * osc(x.t, 4), scratch);
    towards(out, 4, -0.12, scratch);
  }
  if (neck > 0) {
    out[4] += 0.18 * Math.sin(((x.t + 4) / 1.8) * Math.PI * 2) * neck;
    out[2] += 0.1 * Math.cos(((x.t + 4) / 1.8) * Math.PI * 2) * neck;
  }
}

/** Couch gaming: calm and leaning back, forward when it gets tense, the body steers. */
function gamepad(out: Joints, x: MotionCtx) {
  const cue = x.cue;
  const I = cue?.intensity ?? 0.35;
  const lean = cue?.lean ?? 0;
  const { t } = x;
  out[1] = -0.08 + 0.3 * easeOut(I);
  out[5] = out[8] = -0.78 - 0.12 * I;
  out[6] = out[9] = -0.3;
  const thumbs = 0.03 + 0.05 * I;
  out[7] = -1.0 + thumbs * osc(t, 7.3);
  out[10] = -1.0 + thumbs * osc(t, 6.1, 0.3);
  out[18] = -0.4 + thumbs * 0.6 * osc(t, 7.3, 0.2); // both hands round the pad
  out[19] = -0.4 + thumbs * 0.6 * osc(t, 6.1, 0.5);
  out[13] = 0.16 * lean;
  out[4] = -0.12 * lean;
  out[2] = 0.03;
  const cheer = reaction(cue, 'cheer', x.now, 1.7);
  if (cheer) {
    for (const i of [5, 8]) towards(out, i, -2.6 + 0.2 * osc(t, 3), cheer);
    for (const i of [7, 10]) towards(out, i, -0.35, cheer);
    towards(out, 1, -0.15, cheer);
    towards(out, 2, -0.15, cheer);
  }
  const groan = reaction(cue, 'groan', x.now, 1.9);
  if (groan) {
    towards(out, 1, -0.32, groan); // falls back into the sofa
    towards(out, 2, -0.38, groan);
    for (const i of [5, 8]) towards(out, i, -0.25, groan);
    for (const i of [7, 10]) towards(out, i, -0.55, groan);
  }
}

/** Foosball: bent over the handles, wrists working the rods, the body follows the ball. */
function foosball(out: Joints, x: MotionCtx) {
  const cue = x.cue;
  const I = cue?.intensity ?? 0.4;
  const lean = cue?.lean ?? 0;
  const { t } = x;
  out[0] = -0.025;
  out[1] = 0.3 + 0.08 * I;
  // A ready stance: knees bent, weight forward; wrists turn the rods.
  out[11] = out[12] = -0.18;
  out[14] = out[15] = 0.32;
  out[16] = out[17] = -0.14;
  out[18] = 0.45 * I * osc(t, 4.6, 0.25);
  out[19] = 0.45 * I * osc(t, 4.6, 0.75);
  out[5] = out[8] = -1.25;
  out[6] = out[9] = -0.12;
  out[7] = out[10] = -0.3;
  out[5] += 0.1 * I * osc(t, 4.6);
  out[8] += 0.1 * I * osc(t, 4.6, 0.5);
  out[13] = 0.22 * lean;
  out[3] = 0.1 * lean;
  const shot = reaction(cue, 'shot', x.now, 0.4);
  if (shot) {
    out[8] -= 0.45 * shot;
    out[5] += 0.15 * shot;
    out[13] -= 0.2 * shot;
    out[1] += 0.06 * shot;
  }
  const cheer = reaction(cue, 'cheer', x.now, 1.9);
  if (cheer) {
    for (const i of [5, 8]) towards(out, i, -2.85 + 0.18 * osc(t, 2.6), cheer);
    for (const i of [6, 9]) towards(out, i, 0.38, cheer);
    for (const i of [7, 10]) towards(out, i, -0.12, cheer);
    towards(out, 1, -0.08, cheer);
    out[0] += 0.07 * Math.abs(osc(t, 2.2)) * cheer;
  }
  const groan = reaction(cue, 'groan', x.now, 2.0);
  if (groan) {
    // Hands on the head.
    for (const i of [5, 8]) towards(out, i, -2.7, groan);
    for (const i of [6, 9]) towards(out, i, 0.65, groan);
    for (const i of [7, 10]) towards(out, i, -2.0, groan);
    towards(out, 1, -0.06, groan);
    towards(out, 2, -0.28, groan);
  }
}

/** Pool: bend and line up with practice strokes, strike, rise and watch; or wait on the cue. */
function pool(out: Joints, x: MotionCtx) {
  const cue = x.cue;
  const phase = cue?.phase ?? 'wait';
  const p = cue?.t ?? 0;
  const { t } = x;
  if (phase === 'aim' || phase === 'strike') {
    out[0] = -0.04;
    out[1] = 0.62;
    // A lunge: the front knee bends, the back leg stays long.
    out[11] = -0.35;
    out[14] = 0.4;
    out[16] = -0.05;
    out[12] = 0.12;
    out[15] = 0.06;
    out[18] = 0.3; // the bridge hand flat on the felt
    out[5] = -1.45; // the bridge hand on the felt
    out[6] = -0.05;
    out[7] = -0.15;
    out[8] = -0.15; // the cue hand back by the hip
    out[9] = 0.2;
    out[10] = -1.75 + (phase === 'aim' ? 0.22 * osc(t, 0.9) : 1.0 * easeOut(p));
    if (phase === 'strike') out[8] -= 0.25 * easeOut(p);
    out[2] = -0.45; // looking along the cue
  } else if (phase === 'watch') {
    const up = easeOut(p * 2.5);
    out[1] = 0.62 * (1 - up) + 0.08;
    out[5] = -1.45 * (1 - up) + 0.02 * up;
    out[7] = -0.15 * (1 - up) - 0.12 * up;
    out[8] = -0.45;
    out[9] = 0.15;
    out[10] = -1.2;
  } else {
    // Waiting: leaning on the cue, the other hand on the hip.
    out[8] = -0.5;
    out[9] = 0.15;
    out[10] = -1.45;
    out[6] = 0.35;
    out[7] = -1.6;
    out[5] = 0.05;
  }
  const cheer = reaction(cue, 'cheer', x.now, 1.5);
  if (cheer) {
    towards(out, 5, -2.4 + 0.25 * osc(t, 3), cheer); // fist pump
    towards(out, 7, -1.6, cheer);
    towards(out, 6, 0.2, cheer);
    towards(out, 1, 0.02, cheer);
  }
  const groan = reaction(cue, 'groan', x.now, 1.7);
  if (groan) {
    out[3] += 0.22 * osc(t, 2.2) * groan; // shakes its head
    towards(out, 2, 0.25, groan);
    towards(out, 1, 0.05, groan);
  }
}

/** Breathing and weight shifts, on top of any still pose (never while walking). */
export function addLife(out: Joints, pose: Pose, seated: boolean, tSec: number, seed: number, breathPeriod: number) {
  const br = Math.sin((tSec / breathPeriod) * Math.PI * 2);
  const k = pose === 'sleep' ? 0.4 : seated ? 0.7 : 1;
  out[1] += 0.014 * br * k;
  out[2] -= 0.008 * br * k;
  out[6] += 0.012 * br * k;
  out[9] += 0.012 * br * k;
  out[0] += (seated ? 0.002 : 0.004) * (br * 0.5 + 0.5);
  if (!seated && pose !== 'foosball' && pose !== 'pool') {
    const ws = noise(tSec * 0.09, seed + 11);
    out[13] += 0.05 * ws;
    out[4] += 0.035 * ws;
    out[11] += 0.035 * ws;
    out[12] -= 0.035 * ws;
    // Weight on one leg: the other knee softens.
    out[14] += 0.08 * Math.max(0, ws);
    out[15] += 0.08 * Math.max(0, -ws);
    out[20] += 0.025 * ws;
  }
}

export type ReactKind = 'thump' | 'facepalm' | 'startle' | 'fistpump' | 'wave' | 'dance';
/** How long each personal reaction plays (s). */
export const REACT_S: Record<ReactKind, number> = { thump: 1.4, facepalm: 2.0, startle: 1.1, fistpump: 1.3, wave: 1.55, dance: 3.3 };

export type ClipName = keyof typeof CLIPS;

/**
 * A clip from KayKit Character Animations (CC0, Kay Lousberg; converted by
 * scripts/kaykit-clips.py) at weight w. A pose loops its middle (`t` its own clock); `once`
 * plays it all from the start (`t` seconds in), raise to lower. Seated: arms and head only.
 */
export function clip(out: Joints, name: ClipName, t: number, w: number, seated = false, once = false) {
  const c = CLIPS[name];
  const n = c.j['0'].length - 1;
  const [a, b] = c.loop;
  const f = once ? Math.min(n, Math.max(0, t * c.fps)) : a + ((((t * c.fps) % (b - a)) + (b - a)) % (b - a));
  const i = Math.floor(f);
  const k = f - i;
  for (const [key, vs] of Object.entries(c.j)) {
    const j = +key;
    if (seated && (j === 0 || j === 1 || j === 13 || j === 20)) continue;
    const v = vs[i] + (vs[Math.min(i + 1, n)] - vs[i]) * k;
    if (j === 0) out[0] += v * w;
    else towards(out, j, j === 6 || j === 9 ? v + 0.08 : v, w);
  }
}

export function reactPose(out: Joints, kind: ReactKind, w: number, t: number) {
  if (w <= 0) return;
  switch (kind) {
    case 'thump': // both fists come down on the desk
      for (const i of [5, 8]) towards(out, i, -0.9 + 0.35 * Math.max(0, osc(t, 2.2)), w);
      for (const i of [7, 10]) towards(out, i, -1.3, w);
      towards(out, 1, 0.22, w);
      towards(out, 2, 0.15, w);
      break;
    case 'facepalm': // a hand over the eyes, head down
      towards(out, 8, -1.65, w);
      towards(out, 9, -0.35, w);
      towards(out, 10, -2.35, w);
      towards(out, 2, 0.3, w);
      towards(out, 4, -0.08, w);
      break;
    case 'startle': // a jolt back, hands up
      towards(out, 1, -0.18, w);
      for (const i of [5, 8]) towards(out, i, -1.6, w);
      for (const i of [6, 9]) towards(out, i, 0.45, w);
      for (const i of [7, 10]) towards(out, i, -1.4, w);
      towards(out, 2, -0.2, w);
      break;
    case 'fistpump':
      towards(out, 8, -2.5 + 0.3 * osc(t, 3), w);
      towards(out, 10, -1.7, w);
      towards(out, 9, 0.15, w);
      break;
    case 'wave':
      clip(out, 'wave', t, w, false, true);
      break;
    case 'dance': // a promotion: the loop twice, then down
      clip(out, 'dance', t, w);
      break;
  }
}
