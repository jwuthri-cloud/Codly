// The games corner, played for real: foosball with a ball and rods that slide and kick, pool with
// rolling balls, cushions, pockets and turns, and a football match on the TV. Each game is a small
// seeded simulation (pure, tested) that also tells its players what to do: where to look, how
// tense it is, when to shoot, cheer or groan (motion.ts PlayCue).
//
// Only idle agents play (behavior.ts), so this is leisure: the scores are made up and nothing
// here stands for work. Games advance only while the office is live; otherwise they hold still.

import { CanvasTexture, Group, Mesh, MeshBasicMaterial, MeshStandardMaterial, PlaneGeometry, SphereGeometry, type BufferGeometry, Vector3 } from 'three';
import type { Item, OfficeLayout } from '../layout';
import { rng } from '../text';
import { Builder } from './builder';
import type { Character } from './character';
import { FOOSBALL_MEN, FOOSBALL_RODS, foosballRod, POOL_BALLS, POOL_POCKETS } from './furniture';
import { noise, type PlayCue } from './motion';
import { poolSkill, traitsOf } from '../traits';

type React = PlayCue['react'];
/** A player at a two-player table whose partner is not there (yet). */
const WAITING: PlayCue = { intensity: 0.1, lean: 0, phase: 'wait', t: 0, react: null };
const clamp = (x: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, x));

// ───────────── foosball ─────────────

/** Play area (table-local): the ball stays within ±X, ±Z; goals are |x| < GOAL_W at z = ±Z. */
const FX = 0.42;
const FZ = 0.265;
const GOAL_W = 0.11;

/** Team 0 (red, rods 0 and 2) attacks +z; team 1 (blue, rods 1 and 3) attacks −z. Player slot = team. */
/** A sound a game makes: a sample name (sound.ts) or a made-up one (crowd, whistle). */
export interface GameSound {
  name: string;
  gain: number;
}

export class FoosballSim {
  ball = { x: 0, z: 0, vx: 0, vz: 0 };
  slide = [0, 0, 0, 0];
  twist = [0, 0, 0, 0];
  score = [0, 0];
  lastGoal: { team: number; at: number } | null = null;
  lastShot: { team: number; at: number } | null = null;
  /** The ball is out (a goal): it comes back at this time. */
  outUntil = 0;
  /** Sounds since the last frame (the games corner plays them at the table). */
  events: GameSound[] = [];
  private kickAt = [-1e9, -1e9, -1e9, -1e9];
  private r: () => number;

  constructor(seed: number) {
    this.r = rng(seed);
    this.serve(0);
  }

  private serve(team: number) {
    const a = (this.r() - 0.5) * 1.2;
    const s = 0.45 + this.r() * 0.2;
    this.ball = { x: (this.r() - 0.5) * 0.2, z: 0, vx: Math.sin(a) * s, vz: Math.cos(a) * s * (team === 0 ? 1 : -1) };
  }

  /** Advance; `present[team]` = someone plays that side (an empty side still plays, more slowly). */
  step(now: number, dt: number, present: [boolean, boolean]) {
    for (let i = 0; i < 4; i++) {
      const team = i % 2;
      // Rods: a kick swings the men through, then back; otherwise they twitch in hand.
      const age = now - this.kickAt[i];
      this.twist[i] =
        age < 110 ? (-1.5 * age) / 110 : age < 420 ? -1.5 * (1 - (age - 110) / 310) : present[team] ? 0.18 * noise(now / 260 + i * 7, 31 + i) : 0;
    }
    if (now < this.outUntil) return;
    if (this.outUntil) {
      this.outUntil = 0;
      this.serve(this.lastGoal ? 1 - this.lastGoal.team : 0);
    }
    const b = this.ball;
    b.x += b.vx * dt;
    b.z += b.vz * dt;
    const f = Math.exp(-0.3 * dt);
    b.vx *= f;
    b.vz *= f;
    if (Math.abs(b.x) > FX) {
      b.x = Math.sign(b.x) * FX;
      b.vx = -b.vx * 0.8;
    }
    if (Math.abs(b.z) > FZ) {
      if (Math.abs(b.x) < GOAL_W) {
        const team = b.z > 0 ? 0 : 1;
        this.score[team]++;
        this.lastGoal = { team, at: now };
        this.outUntil = now + 1500;
        this.events.push({ name: 'pool-pocket', gain: 0.8 });
        return;
      }
      b.z = Math.sign(b.z) * FZ;
      b.vz = -b.vz * 0.8;
    }
    if (Math.hypot(b.vx, b.vz) < 0.12) {
      // A dead ball gets nudged, as anyone would.
      const a = this.r() * Math.PI * 2;
      b.vx = Math.cos(a) * 0.55;
      b.vz = Math.sin(a) * 0.55;
    }
    for (let i = 0; i < 4; i++) {
      const team = i % 2;
      // The men slide to line up with the ball, slower when nobody holds the handles.
      const near = FOOSBALL_MEN.reduce((best, m) => (Math.abs(b.x - m) < Math.abs(b.x - best) ? m : best), FOOSBALL_MEN[0]);
      const want = clamp(b.x - near, -0.09, 0.09);
      this.slide[i] += (want - this.slide[i]) * (1 - Math.exp(-dt * (present[team] ? 9 : 5)));
      const rz = FOOSBALL_RODS[i];
      if (Math.abs(b.z - rz) > 0.022 || now - this.kickAt[i] < 380) continue;
      const hit = FOOSBALL_MEN.some((m) => Math.abs(m + this.slide[i] - b.x) < 0.05);
      if (!hit) continue;
      const goalZ = team === 0 ? FZ + 0.05 : -FZ - 0.05;
      const aimX = (this.r() - 0.5) * 0.32;
      const dx = aimX - b.x;
      const dz = goalZ - b.z;
      const d = Math.hypot(dx, dz) || 1;
      const s = 0.9 + this.r() * 0.9;
      b.vx = (dx / d) * s;
      b.vz = (dz / d) * s;
      b.z = rz + Math.sign(dz) * 0.024;
      this.kickAt[i] = now;
      this.lastShot = { team, at: now };
      this.events.push({ name: 'foos-kick', gain: 0.35 + s * 0.2 });
    }
  }

  /** The cue for the player of `team`; `leftSign` maps table z to its left (−1 for the −x side). */
  cue(team: number, leftSign: number): PlayCue {
    const b = this.ball;
    const rods = FOOSBALL_RODS.filter((_, i) => i % 2 === team);
    const d = Math.min(...rods.map((z) => Math.abs(b.z - z)));
    const speed = Math.min(1, Math.hypot(b.vx, b.vz) / 1.6);
    let react: React = null;
    const g = this.lastGoal;
    const s = this.lastShot;
    if (g) react = { kind: g.team === team ? 'cheer' : 'groan', at: g.at };
    if (s && s.team === team && (!g || s.at > g.at)) react = { kind: 'shot', at: s.at };
    return { intensity: clamp(1 - d / 0.3, 0, 1) * 0.7 + speed * 0.3, lean: clamp((leftSign * b.z) / FZ, -1, 1), phase: null, t: 0, react };
  }
}

// ───────────── ping-pong ─────────────

/** Table-local: the ends are at x = ±PP_X (slot 0 plays the +x end); the top is at PP_TOP. */
const PP_X = 0.9;
export const PP_TOP = 0.76;
const PP_R = 0.02;

/** A rally: the ball flies end to end, bounces once on the far half, and is hit back, until
 *  someone misses (about one shot in seven). */
export class PingPongSim {
  ball = { x: PP_X, y: PP_TOP + 0.2, z: 0 };
  score = [0, 0];
  lastPoint: { team: number; at: number } | null = null;
  lastHit: { team: number; at: number } | null = null;
  outUntil = 0;
  events: GameSound[] = [];
  /** Who hit the ball in flight (0: from the +x end), when, how long it flies and where it goes. */
  private from = 0;
  private at = 0;
  private dur = 0.9;
  private z0 = 0;
  private z1 = 0;
  private miss = false;
  private bounced = false;
  private r: () => number;

  constructor(seed: number) {
    this.r = rng(seed);
  }

  private hit(team: number, now: number) {
    this.from = team;
    this.at = now;
    this.dur = 0.8 + this.r() * 0.35;
    this.z0 = this.z1;
    this.z1 = (this.r() - 0.5) * 0.7;
    this.miss = this.r() < 0.14;
    this.bounced = false;
    this.lastHit = { team, at: now };
    this.events.push({ name: 'pool-ball', gain: 0.3 });
  }

  step(now: number) {
    if (!this.lastHit) this.hit(0, now);
    if (now < this.outUntil) return;
    if (this.outUntil) {
      this.outUntil = 0;
      this.z1 = 0;
      this.hit(this.lastPoint?.team ?? 0, now); // the winner serves
    }
    const p = (now - this.at) / (this.dur * 1000);
    const dir = this.from === 0 ? -1 : 1;
    const b = this.ball;
    b.x = -dir * (PP_X + 0.1) + dir * 2 * (PP_X + 0.1) * Math.min(p, 1.35);
    b.z = this.z0 + (this.z1 - this.z0) * Math.min(p, 1.35);
    const top = PP_TOP + PP_R;
    const B = 0.72; // where it bounces, 0 … 1 along the flight
    if (p < B) {
      const k = p / B;
      b.y = top + (0.2 - 0.2 * k) + 0.3 * 4 * k * (1 - k);
    } else if (p < 1 || !this.miss) {
      const k = (p - B) / (1 - B);
      b.y = top + 0.2 * k + 0.18 * 4 * k * (1 - k);
      if (!this.bounced) {
        this.bounced = true;
        this.events.push({ name: 'pool-ball', gain: 0.18 });
      }
    } else b.y = Math.max(PP_R, top + 0.2 - 2.4 * (p - 1)); // whiffed: past the paddle, down to the floor
    if (p >= 1 && !this.miss) this.hit(1 - this.from, now);
    else if (p >= 1.35 && this.miss) {
      this.score[this.from]++;
      this.lastPoint = { team: this.from, at: now };
      this.outUntil = now + 1500;
    }
  }

  /** The cue for the player of `team` (0 plays the +x end facing −x: its left is +z). */
  cue(team: number): PlayCue {
    const b = this.ball;
    const end = team === 0 ? PP_X : -PP_X;
    let react: React = null;
    const pt = this.lastPoint;
    const h = this.lastHit;
    if (pt) react = { kind: pt.team === team ? 'cheer' : 'groan', at: pt.at };
    if (h && h.team === team && (!pt || h.at > pt.at)) react = { kind: 'shot', at: h.at };
    return { intensity: clamp(1 - Math.abs(b.x - end) / (2 * PP_X), 0, 1), lean: clamp(((team === 0 ? 1 : -1) * b.z) / 0.45, -1, 1), phase: null, t: 0, react };
  }
}

// ───────────── pool ─────────────

const BALL_R = 0.03;
const PX = 0.87;
const PZ = 0.42;
/** The table's outer half extents, and how far from them a shooter stands. */
const TABLE_X = 1.0;
const TABLE_Z = 0.55;
const STAND = 0.42;

export interface PoolBall {
  x: number;
  z: number;
  vx: number;
  vz: number;
  in: boolean;
}

type PoolPhase = 'place' | 'walk' | 'aim' | 'strike' | 'roll' | 'after';

/** A shot to play: where to stand (table-local) and the cue ball's velocity. */
export interface PoolShot {
  spot: { x: number; z: number; face: number };
  vx: number;
  vz: number;
  target: number;
}

export class PoolSim {
  balls: PoolBall[] = [];
  phase: PoolPhase = 'place';
  phaseAt = 0;
  phaseLen = 0;
  /** Index into the players list given to `step` (whose turn it is). */
  turn = 0;
  shot: PoolShot | null = null;
  result: { player: number; kind: 'cheer' | 'groan'; at: number } | null = null;
  /** Sounds since the last frame (the games corner plays them at the table). */
  events: GameSound[] = [];
  private potted = false;
  private rackAt = 0;
  private r: () => number;

  constructor(seed: number) {
    this.r = rng(seed);
    this.rack();
  }

  rack() {
    this.balls = POOL_BALLS.map(([x, z]) => ({ x, z, vx: 0, vz: 0, in: false }));
  }

  get moving() {
    return this.balls.some((b) => !b.in && (b.vx || b.vz));
  }

  private set(phase: PoolPhase, now: number, len = 0) {
    this.phase = phase;
    this.phaseAt = now;
    this.phaseLen = len;
  }

  /** Choose the next shot: an object ball and the pocket it goes in, aimed at the ghost ball. */
  /** Chance (0 … 1) the next shooter's shot goes where it aims (traits.ts poolSkill). */
  skill = 0.55;

  plan(): PoolShot {
    const cue = this.balls[0];
    let best: { score: number; t: number; px: number; pz: number } | null = null;
    this.balls.forEach((b, i) => {
      if (i === 0 || b.in) return;
      for (const [px, pz] of POOL_POCKETS) {
        const tx = px - b.x;
        const tz = pz - b.z;
        const td = Math.hypot(tx, tz);
        const cx = b.x - cue.x;
        const cz = b.z - cue.z;
        const cd = Math.hypot(cx, cz);
        const cut = (tx * cx + tz * cz) / (td * cd || 1); // 1 = straight in
        if (cut < 0.2) continue;
        const score = cut * 2 - td - cd * 0.5;
        if (!best || score > best.score) best = { score, t: i, px, pz };
      }
    });
    const b = best as { t: number; px: number; pz: number } | null;
    const target = b ? this.balls[b.t] : (this.balls.find((x, i) => i > 0 && !x.in) ?? this.balls[0]);
    const pocket = b ? [b.px, b.pz] : [0, 0];
    const ux = pocket[0] - target.x;
    const uz = pocket[1] - target.z;
    const ud = Math.hypot(ux, uz) || 1;
    const gx = target.x - (ux / ud) * BALL_R * 2;
    const gz = target.z - (uz / ud) * BALL_R * 2;
    let dx = gx - cue.x;
    let dz = gz - cue.z;
    const dd = Math.hypot(dx, dz) || 1;
    dx /= dd;
    dz /= dd;
    // Agents are not that good: whether it goes where it aims depends on the shooter (traits).
    const err = this.r() < this.skill ? (this.r() - 0.5) * 0.02 : (this.r() < 0.5 ? -1 : 1) * (0.05 + this.r() * 0.1);
    const c = Math.cos(err);
    const s = Math.sin(err);
    const vx = dx * c - dz * s;
    const vz = dx * s + dz * c;
    const speed = 1.3 + this.r() * 0.9;
    // Stand behind the cue ball, outside the table, facing the shot.
    let t = 0;
    while (Math.abs(cue.x - vx * t) < TABLE_X && Math.abs(cue.z - vz * t) < TABLE_Z) t += 0.02;
    t += STAND;
    return { spot: { x: cue.x - vx * t, z: cue.z - vz * t, face: Math.atan2(vx, vz) }, vx: vx * speed, vz: vz * speed, target: b ? b.t : 0 };
  }

  /** Advance the turn. `players` = how many play; `arrived` = the shooter stands at its spot. */
  step(now: number, dt: number, players: number, arrived: boolean) {
    this.roll(dt);
    if (!players) return;
    this.turn %= players;
    const age = (now - this.phaseAt) / 1000;
    switch (this.phase) {
      case 'place':
        if (this.rackAt && now < this.rackAt) return;
        if (this.rackAt) {
          this.rackAt = 0;
          this.rack();
        }
        if (this.moving) return;
        this.shot = this.plan();
        this.set('walk', now);
        break;
      case 'walk':
        if (arrived) this.set('aim', now, 1.8 + this.r() * 1.4);
        break;
      case 'aim':
        if (age >= this.phaseLen) this.set('strike', now, 0.18);
        break;
      case 'strike':
        if (age >= this.phaseLen && this.shot) {
          this.balls[0].vx = this.shot.vx;
          this.balls[0].vz = this.shot.vz;
          this.events.push({ name: 'pool-cue', gain: 0.7 });
          this.potted = false;
          this.set('roll', now, 4);
        }
        break;
      case 'roll':
        if (!this.moving) {
          const kind = this.potted ? 'cheer' : 'groan';
          this.result = { player: this.turn, kind, at: now };
          if (this.balls[0].in) Object.assign(this.balls[0], { x: -0.5, z: 0, vx: 0, vz: 0, in: false }); // scratch
          if (!this.potted) this.turn = (this.turn + 1) % players;
          if (this.balls.every((b, i) => i === 0 || b.in)) this.rackAt = now + 2200;
          this.set('after', now, 0.9);
        }
        break;
      case 'after':
        if (age >= this.phaseLen) this.set('place', now);
        break;
    }
  }

  /** Rolling, cushions, pockets and collisions (equal masses, elastic). */
  private roll(dt: number) {
    const live = this.balls.filter((b) => !b.in);
    for (const b of live) {
      if (!b.vx && !b.vz) continue;
      b.x += b.vx * dt;
      b.z += b.vz * dt;
      const sp = Math.hypot(b.vx, b.vz);
      const ns = Math.max(0, sp - 0.85 * dt);
      if (ns < 0.02) {
        b.vx = b.vz = 0;
      } else {
        b.vx *= ns / sp;
        b.vz *= ns / sp;
      }
      for (const [px, pz] of POOL_POCKETS) {
        if (Math.hypot(b.x - px, b.z - pz) < 0.06) {
          this.events.push({ name: 'pool-pocket', gain: 0.6 });
          b.in = true;
          b.vx = b.vz = 0;
          if (b !== this.balls[0]) this.potted = true;
          break;
        }
      }
      if (b.in) continue;
      if (Math.abs(b.x) > PX) {
        b.x = Math.sign(b.x) * PX;
        b.vx = -b.vx * 0.8;
      }
      if (Math.abs(b.z) > PZ) {
        b.z = Math.sign(b.z) * PZ;
        b.vz = -b.vz * 0.8;
      }
    }
    for (let i = 0; i < live.length; i++) {
      for (let j = i + 1; j < live.length; j++) {
        const a = live[i];
        const b = live[j];
        if (a.in || b.in) continue;
        const dx = b.x - a.x;
        const dz = b.z - a.z;
        const d = Math.hypot(dx, dz);
        if (d >= BALL_R * 2 || d === 0) continue;
        const nx = dx / d;
        const nz = dz / d;
        const rel = (a.vx - b.vx) * nx + (a.vz - b.vz) * nz;
        const push = (BALL_R * 2 - d) / 2;
        a.x -= nx * push;
        a.z -= nz * push;
        b.x += nx * push;
        b.z += nz * push;
        if (rel <= 0) continue;
        // The clack: louder the harder they meet.
        if (rel > 0.05) this.events.push({ name: 'pool-ball', gain: Math.min(1, 0.25 + rel * 0.7) });
        a.vx -= rel * nx;
        a.vz -= rel * nz;
        b.vx += rel * nx;
        b.vz += rel * nz;
      }
    }
  }

  /** The cue for player `i` (its index in the players list). */
  cue(i: number, now: number): PlayCue {
    let react: React = null;
    if (this.result && this.result.player === i) react = { kind: this.result.kind, at: this.result.at };
    if (i !== this.turn) return { intensity: this.phase === 'roll' ? 0.7 : 0.2, lean: 0, phase: 'wait', t: 0, react };
    const age = (now - this.phaseAt) / 1000;
    const t = this.phaseLen ? clamp(age / this.phaseLen, 0, 1) : 0;
    const phase = this.phase === 'aim' ? 'aim' : this.phase === 'strike' ? 'strike' : this.phase === 'roll' || this.phase === 'after' ? 'watch' : 'wait';
    return { intensity: this.phase === 'aim' ? 0.8 : 0.5, lean: 0, phase, t, react };
  }

  /** What everyone looks at: the ball that moves most, else the shot's target. */
  focus(): PoolBall {
    let best: PoolBall | null = null;
    let bs = 0;
    for (const b of this.balls) {
      const s = Math.hypot(b.vx, b.vz);
      if (!b.in && s > bs) {
        bs = s;
        best = b;
      }
    }
    return best ?? this.balls[this.shot?.target ?? 0] ?? this.balls[0];
  }
}

/** Walk around a rectangle (half extents hx, hz) from a to b, never across it: the corners to pass,
 *  the shorter way round. */
export function aroundTable(a: [number, number], b: [number, number], hx: number, hz: number): [number, number][] {
  // Perimeter position s, counter-clockwise from the (hx, hz) corner (towards −x first).
  const per = 4 * (hx + hz);
  const corners: { s: number; p: [number, number] }[] = [
    { s: 0, p: [hx, hz] },
    { s: 2 * hx, p: [-hx, hz] },
    { s: 2 * hx + 2 * hz, p: [-hx, -hz] },
    { s: 4 * hx + 2 * hz, p: [hx, -hz] },
  ];
  const param = ([x, z]: [number, number]) => {
    const px = clamp(x, -hx, hx);
    const pz = clamp(z, -hz, hz);
    const top = hz - pz;
    const left = px + hx;
    const bottom = pz + hz;
    const m = Math.min(top, left, bottom, hx - px);
    if (m === top) return hx - px;
    if (m === left) return 2 * hx + (hz - pz);
    if (m === bottom) return 2 * hx + 2 * hz + (px + hx);
    return 4 * hx + 2 * hz + (pz + hz);
  };
  const sa = param(a);
  const fwd = (param(b) - sa + per) % per;
  const ccw = fwd <= per / 2;
  const span = ccw ? fwd : per - fwd;
  return corners
    .map((c) => ({ d: ccw ? (c.s - sa + per) % per : (sa - c.s + per) % per, p: c.p }))
    .filter((c) => c.d > 1e-6 && c.d < span)
    .sort((x, y) => x.d - y.d)
    .map((c) => c.p);
}

// ───────────── console ─────────────

/** The two sides on the TV: seat 0 is the home side. Kits are the teams' home colours. */
const TV_TEAMS = [
  { code: 'BRA', shirt: '#FFDF00', shorts: '#002776', ink: '#0b1b4a' },
  { code: 'ARG', shirt: '#75AADB', shorts: '#1c1c1c', ink: '#0b1b2e' },
];

/** A football match on the TV: about 75 s from kick-off to full time, chances every few seconds. */
export class ConsoleSim {
  score = [0, 0];
  /** 0 … 1 through the match. */
  clock = 0;
  /** The side attacking right now (seat index) and how far into the chance (0 … 1), or null. */
  chance: { side: number; at: number } | null = null;
  lastGoal: { side: number; at: number } | null = null;
  lastMiss: { side: number; at: number } | null = null;
  fullTime: number | null = null;
  /** Sounds since the last frame: the crowd and the referee, from the TV. */
  events: GameSound[] = [];
  ball = { x: 0, y: 0 };
  private nextChance = 0;
  private r: () => number;

  constructor(private seed: number) {
    this.r = rng(seed);
  }

  step(now: number, dt: number) {
    if (this.fullTime) {
      if (now - this.fullTime > 4500) {
        this.score = [0, 0];
        this.clock = 0;
        this.fullTime = null;
        this.events.push({ name: 'whistle', gain: 0.6 }); // kick-off
      }
      return;
    }
    this.clock += dt / 75;
    if (this.clock >= 1) {
      this.fullTime = now;
      this.chance = null;
      this.events.push({ name: 'whistle-end', gain: 0.6 });
      return;
    }
    if (!this.nextChance) this.nextChance = now + 3000 + this.r() * 6000;
    if (!this.chance && now >= this.nextChance) {
      this.chance = { side: this.r() < 0.5 ? 0 : 1, at: now };
      this.events.push({ name: 'crowd', gain: 0.5 });
    }
    if (this.chance && now - this.chance.at > 1700) {
      const side = this.chance.side;
      if (this.r() < 0.36) {
        this.score[side]++;
        this.lastGoal = { side, at: now };
        this.events.push({ name: 'crowd-goal', gain: 0.7 });
      } else {
        this.lastMiss = { side, at: now };
        this.events.push({ name: 'crowd-ooh', gain: 0.5 });
      }
      this.chance = null;
      this.nextChance = now + 4500 + this.r() * 8000;
    }
    // The ball drifts with the play; during a chance it runs at a goal.
    const t = now / 1000;
    const drift = noise(t * 0.4, this.seed);
    const target = this.chance ? (this.chance.side === 0 ? 0.85 : -0.85) : drift * 0.5;
    this.ball.x += (target - this.ball.x) * (1 - Math.exp(-dt * (this.chance ? 2.2 : 0.9)));
    this.ball.y = noise(t * 0.7, this.seed + 5) * 0.6;
  }

  cue(seat: number, now: number): PlayCue {
    const t = now / 1000;
    const tense = this.chance ? Math.min(1, (now - this.chance.at) / 1200) : 0;
    const late = this.clock > 0.85 ? 0.25 : 0;
    let react: React = null;
    if (this.lastGoal) react = { kind: this.lastGoal.side === seat ? 'cheer' : 'groan', at: this.lastGoal.at };
    if (this.lastMiss && this.lastMiss.side === seat && (!this.lastGoal || this.lastMiss.at > this.lastGoal.at)) react = { kind: 'groan', at: this.lastMiss.at };
    if (this.fullTime && this.score[0] !== this.score[1]) {
      const won = this.score[seat] > this.score[1 - seat];
      react = { kind: won ? 'cheer' : 'groan', at: this.fullTime };
    }
    return { intensity: clamp(0.3 + 0.15 * noise(t * 0.5, this.seed + seat) + tense * 0.6 + late, 0, 1), lean: clamp(noise(t * 1.3, this.seed + 9 + seat) * (0.5 + tense), -1, 1), phase: null, t: 0, react };
  }
}

// ───────────── the corner (meshes, players) ─────────────

export interface GamePlayer {
  /** Slot index at its station ("games.foosball:1" → 1). */
  slot: number;
  c: Character;
}

export interface GamePlayers {
  foosball: GamePlayer[];
  pingpong: GamePlayer[];
  pool: GamePlayer[];
  console: GamePlayer[];
}

/** An item's frame: table-local (x, z) → layout coordinates. */
function frameOf(it: Item) {
  const c = Math.cos(it.rot);
  const s = Math.sin(it.rot);
  return {
    toLayout: (x: number, z: number): [number, number] => [it.x + x * c + z * s, it.z - x * s + z * c],
    toLocal: (x: number, z: number): [number, number] => {
      const dx = x - it.x;
      const dz = z - it.z;
      return [dx * c - dz * s, dx * s + dz * c];
    },
  };
}

export class GamesCorner {
  readonly group = new Group();
  private mat = new MeshStandardMaterial({ vertexColors: true, roughness: 0.55, metalness: 0.1 });
  private geos: BufferGeometry[] = [];
  private foos: { it: Item; sim: FoosballSim; rods: Group[]; ball: Mesh } | null = null;
  private pool: { it: Item; sim: PoolSim; balls: Mesh[]; walking: Character | null } | null = null;
  private pong: { it: Item; sim: PingPongSim; ball: Mesh } | null = null;
  private tv: { it: Item; sim: ConsoleSim; screen: Mesh; tex: CanvasTexture; g: CanvasRenderingContext2D; drawnAt: number } | null = null;
  private v = new Vector3();

  /** `origin` = where layout (0, 0) is in the world (characters live in world coordinates). */
  constructor(
    L: OfficeLayout,
    private origin: { x: number; z: number },
    canvas?: (w: number, h: number) => [HTMLCanvasElement, CanvasRenderingContext2D],
    texture?: (c: HTMLCanvasElement) => CanvasTexture,
  ) {
    const find = (type: string) => L.items.find((i) => i.type === type);
    const frame = (it: Item) => {
      const g = new Group();
      g.position.set(it.x, 0, it.z);
      g.rotation.y = it.rot;
      this.group.add(g);
      return g;
    };
    const fi = find('foosball');
    if (fi) {
      const g = frame(fi);
      const geo = [0, 1].map((team) => {
        const b = new Builder();
        foosballRod(b, team);
        return b.build();
      });
      this.geos.push(...geo);
      const rods = FOOSBALL_RODS.map((z, i) => {
        const rod = new Group();
        rod.position.set(0, 0.84, z);
        const m = new Mesh(geo[i % 2], this.mat);
        m.castShadow = true;
        rod.add(m);
        g.add(rod);
        return rod;
      });
      const ball = new Mesh(this.sphere(0.025), new MeshStandardMaterial({ color: 0xf4efe3, roughness: 0.4 }));
      ball.position.y = 0.795;
      g.add(ball);
      this.foos = { it: fi, sim: new FoosballSim(1), rods, ball };
    }
    const ppi = find('pingpong');
    if (ppi) {
      const g = frame(ppi);
      const ball = new Mesh(this.sphere(PP_R), new MeshStandardMaterial({ color: 0xff9a3c, roughness: 0.4 }));
      ball.castShadow = true;
      g.add(ball);
      this.pong = { it: ppi, sim: new PingPongSim(4), ball };
    }
    const pi = find('poolTable');
    if (pi) {
      const g = frame(pi);
      const balls = POOL_BALLS.map(([x, z, color]) => {
        const m = new Mesh(this.sphere(BALL_R), new MeshStandardMaterial({ color, roughness: 0.25 }));
        m.position.set(x, 0.735, z);
        m.castShadow = true;
        g.add(m);
        return m;
      });
      this.pool = { it: pi, sim: new PoolSim(2), balls, walking: null };
    }
    const ti = find('tvStand');
    if (ti && canvas && texture) {
      const g = frame(ti);
      const [cv, ctx] = canvas(512, 288);
      const t = texture(cv);
      const screen = new Mesh(new PlaneGeometry(1.22, 0.66), new MeshBasicMaterial({ map: t, toneMapped: false }));
      screen.position.set(0, 1.0, -0.043);
      screen.visible = false;
      g.add(screen);
      this.tv = { it: ti, sim: new ConsoleSim(3), screen, tex: t, g: ctx, drawnAt: 0 };
    }
  }

  private sphere(r: number) {
    const geo = new SphereGeometry(r, 12, 8);
    this.geos.push(geo);
    return geo;
  }

  /** Table-local → world (characters' frame) at height y. */
  private world(it: Item, x: number, z: number, y: number, out = this.v) {
    const [lx, lz] = frameOf(it).toLayout(x, z);
    return out.set(lx + this.origin.x, y, lz + this.origin.z);
  }

  /** Where each game's sounds come from (the engine plays them; floor plan x, z). */
  onSound: ((s: GameSound, at: [number, number]) => void) | null = null;

  /** Advance the games and steer their players (cue, gaze; the pool shooter walks). */
  update(now: number, dt: number, p: GamePlayers): boolean {
    let moving = false;
    if (this.foos) moving = this.updateFoosball(now, dt, p.foosball) || moving;
    if (this.pool) moving = this.updatePool(now, dt, p.pool) || moving;
    if (this.pong) moving = this.updatePingPong(now, p.pingpong) || moving;
    if (this.tv) moving = this.updateTv(now, dt, p.console) || moving;
    for (const g of [this.foos, this.pool, this.pong, this.tv]) {
      if (!g) continue;
      for (const e of g.sim.events.splice(0)) this.onSound?.(e, [g.it.x, g.it.z]);
    }
    // Pad clicks while someone plays on the PS5.
    if (this.tv && p.console.length && Math.random() < dt * 1.6 * p.console.length) this.onSound?.({ name: 'pad', gain: 0.25 }, [this.tv.it.x + 2.6, this.tv.it.z]);
    return moving;
  }

  private updateFoosball(now: number, dt: number, players: GamePlayer[]) {
    const f = this.foos!;
    if (!players.length) return false;
    if (players.length < 2) {
      // Waiting for a partner (still walking over, or gone back to work): no match alone.
      const centre = this.world(f.it, 0, 0, 0.8);
      for (const pl of players) {
        pl.c.cue = WAITING;
        pl.c.gaze = (pl.c.gaze ?? new Vector3()).copy(centre);
      }
      return true;
    }
    const present: [boolean, boolean] = [true, true];
    f.sim.step(now, dt, present);
    f.rods.forEach((rod, i) => {
      rod.rotation.x = (i % 2 ? -1 : 1) * f.sim.twist[i];
      rod.position.x = f.sim.slide[i];
    });
    const b = f.sim.ball;
    f.ball.visible = now >= f.sim.outUntil;
    f.ball.position.set(b.x, 0.795, b.z);
    for (const pl of players) {
      const team = pl.slot === 0 ? 0 : 1;
      // Slot 0 stands on the −x side facing +x: its left is −z. Slot 1 the opposite.
      pl.c.cue = f.sim.cue(team, team === 0 ? -1 : 1);
      pl.c.gaze = (pl.c.gaze ?? new Vector3()).copy(this.world(f.it, b.x, b.z, 0.8));
    }
    return true;
  }

  private updatePingPong(now: number, players: GamePlayer[]) {
    const P = this.pong!;
    P.ball.visible = players.length >= 2 && now >= P.sim.outUntil;
    if (!players.length) return false;
    if (players.length < 2) {
      const centre = this.world(P.it, 0, 0, PP_TOP);
      for (const pl of players) {
        pl.c.cue = WAITING;
        pl.c.gaze = (pl.c.gaze ?? new Vector3()).copy(centre);
      }
      return true;
    }
    P.sim.step(now);
    const b = P.sim.ball;
    P.ball.position.set(b.x, b.y, b.z);
    for (const pl of players) {
      pl.c.cue = P.sim.cue(pl.slot === 0 ? 0 : 1);
      pl.c.gaze = (pl.c.gaze ?? new Vector3()).copy(this.world(P.it, b.x, b.z, b.y));
    }
    return true;
  }

  private updatePool(now: number, dt: number, players: GamePlayer[]) {
    const P = this.pool!;
    const sim = P.sim;
    const list = [...players].sort((a, b) => a.slot - b.slot);
    if (list.length < 2) {
      // Pool takes two: whoever is here waits for its partner, leaning on the cue.
      sim.step(now, dt, 0, false); // balls already rolling come to rest
      P.walking = null;
      const centre = this.world(P.it, 0, 0, 0.74);
      for (const pl of list) {
        pl.c.cue = WAITING;
        pl.c.gaze = (pl.c.gaze ?? new Vector3()).copy(centre);
      }
      return sim.moving || list.length > 0;
    }
    const shooter = list[sim.turn % list.length];
    sim.skill = poolSkill(traitsOf(shooter.c.id));
    // The shooter walks round the table to stand behind the cue ball.
    let arrived = false;
    if (shooter && sim.phase === 'walk' && sim.shot) {
      const fr = frameOf(P.it);
      const pos = shooter.c.position;
      const [cx, cz] = fr.toLocal(pos.x - this.origin.x, pos.z - this.origin.z);
      const spot = sim.shot.spot;
      const close = Math.hypot(cx - spot.x, cz - spot.z) < 0.08;
      if (close && !shooter.c.walking) arrived = true;
      else if (P.walking !== shooter.c || !shooter.c.walking) {
        const hx = TABLE_X + STAND;
        const hz = TABLE_Z + STAND;
        const via = aroundTable([cx, cz], [spot.x, spot.z], hx, hz);
        const toWorld = ([x, z]: [number, number]): [number, number] => {
          const [lx, lz] = fr.toLayout(x, z);
          return [lx + this.origin.x, lz + this.origin.z];
        };
        const [fx, fz] = toWorld([spot.x, spot.z]);
        shooter.c.walk(via.map(toWorld), { x: fx, z: fz, face: spot.face + P.it.rot }, shooter.c.destKey ?? 'st:games.pool', now);
        shooter.c.speed = 1.6;
        P.walking = shooter.c;
      }
    }
    if (arrived) P.walking = null;
    const wasMoving = sim.moving;
    sim.step(now, dt, list.length, arrived);
    sim.balls.forEach((b, i) => {
      const m = P.balls[i];
      m.visible = !b.in;
      m.position.set(b.x, 0.735, b.z);
    });
    const focus = sim.focus();
    list.forEach((pl, i) => {
      pl.c.cue = sim.cue(i, now);
      pl.c.gaze = (pl.c.gaze ?? new Vector3()).copy(this.world(P.it, focus.x, focus.z, 0.74));
    });
    return wasMoving || sim.moving || list.length > 0;
  }

  private updateTv(now: number, dt: number, players: GamePlayer[]) {
    const T = this.tv!;
    // The TV is always on: the match runs whether or not anyone is on the PS5.
    T.screen.visible = true;
    T.sim.step(now, dt);
    const tv = this.world(T.it, 0, 0, 1.0, new Vector3());
    for (const pl of players) {
      pl.c.cue = T.sim.cue(pl.slot === 0 ? 0 : 1, now);
      pl.c.gaze = (pl.c.gaze ?? new Vector3()).copy(tv);
    }
    if (now - T.drawnAt > 80) {
      this.drawTv(T.g, T.sim, now);
      T.tex.needsUpdate = true;
      T.drawnAt = now;
    }
    return true;
  }

  /** The match on the TV, broadcast style: striped pitch, numbered players, a score bar, a goal banner. */
  private drawTv(g: CanvasRenderingContext2D, sim: ConsoleSim, now: number) {
    const W = 512;
    const H = 288;
    const TOP = 36;
    const BOT = 26;
    const PH = H - TOP - BOT;
    // Pitch: mown stripes, then the lines.
    for (let i = 0; i < 12; i++) {
      g.fillStyle = i % 2 ? '#2b8649' : '#33975a';
      g.fillRect((i * W) / 12, TOP, W / 12, PH);
    }
    g.strokeStyle = 'rgba(255,255,255,0.8)';
    g.lineWidth = 2;
    g.strokeRect(14, TOP + 8, W - 28, PH - 16);
    g.beginPath();
    g.moveTo(W / 2, TOP + 8);
    g.lineTo(W / 2, H - BOT - 8);
    g.stroke();
    g.beginPath();
    g.arc(W / 2, TOP + PH / 2, 34, 0, Math.PI * 2);
    g.stroke();
    for (const side of [-1, 1]) {
      const x0 = side < 0 ? 14 : W - 14 - 70;
      g.strokeRect(x0, TOP + PH / 2 - 50, 70, 100);
    }
    const cx = (x: number) => W / 2 + x * (W / 2 - 26);
    const cy = (y: number) => TOP + PH / 2 + y * (PH / 2 - 14);
    const t = performance.now() / 1000;
    // Players: a shadow, the kit, and a shirt number.
    for (let side = 0; side < 2; side++) {
      for (let k = 0; k < 5; k++) {
        const home = (side ? 1 : -1) * (0.15 + (k % 3) * 0.25);
        const x = home * 0.9 + sim.ball.x * 0.35 + noise(t * 0.6 + k, side * 10 + k) * 0.08;
        const y = (k - 2) * 0.35 + sim.ball.y * 0.3 + noise(t * 0.5 + k, side * 20 + k) * 0.1;
        const px = cx(x);
        const py = cy(y);
        g.fillStyle = 'rgba(0,0,0,0.25)';
        g.beginPath();
        g.ellipse(px, py + 6, 8, 3, 0, 0, Math.PI * 2);
        g.fill();
        const team = TV_TEAMS[side];
        g.fillStyle = team.shorts;
        g.fillRect(px - 5, py + 2, 10, 6);
        g.fillStyle = team.shirt;
        g.beginPath();
        g.arc(px, py, 8, 0, Math.PI * 2);
        g.fill();
        g.strokeStyle = '#fff';
        g.lineWidth = 1.5;
        g.stroke();
        g.fillStyle = team.ink;
        g.font = '700 9px Inter, system-ui, sans-serif';
        g.textAlign = 'center';
        g.textBaseline = 'middle';
        g.fillText(String(side * 5 + k + 1), px, py + 0.5);
      }
    }
    // Ball with a soft shadow.
    const bx = cx(sim.ball.x);
    const by = cy(sim.ball.y);
    g.fillStyle = 'rgba(0,0,0,0.3)';
    g.beginPath();
    g.ellipse(bx, by + 4, 5, 2, 0, 0, Math.PI * 2);
    g.fill();
    g.fillStyle = '#fff';
    g.beginPath();
    g.arc(bx, by, 4.5, 0, Math.PI * 2);
    g.fill();
    // Score bar: team colours, score, clock.
    g.fillStyle = 'rgba(10,13,20,0.9)';
    g.fillRect(0, 0, W, TOP);
    g.fillStyle = TV_TEAMS[0].shirt;
    g.fillRect(0, 0, 6, TOP);
    g.fillStyle = TV_TEAMS[1].shirt;
    g.fillRect(W - 6, 0, 6, TOP);
    g.fillStyle = '#fff';
    g.textBaseline = 'middle';
    g.font = '700 18px Inter, system-ui, sans-serif';
    g.textAlign = 'right';
    g.fillText(TV_TEAMS[0].code, W / 2 - 60, TOP / 2 + 1);
    g.textAlign = 'left';
    g.fillText(TV_TEAMS[1].code, W / 2 + 60, TOP / 2 + 1);
    g.font = '800 22px Inter, system-ui, sans-serif';
    g.textAlign = 'center';
    g.fillText(`${sim.score[0]} – ${sim.score[1]}`, W / 2, TOP / 2 + 1);
    // Lower third: the clock, and a banner after a goal.
    g.fillStyle = 'rgba(10,13,20,0.9)';
    g.fillRect(0, H - BOT, W, BOT);
    g.fillStyle = '#fff';
    g.font = '600 13px Inter, system-ui, sans-serif';
    g.textAlign = 'left';
    g.fillText('MUNDIAL', 10, H - BOT / 2 + 1);
    g.textAlign = 'right';
    g.fillText(sim.fullTime ? 'FULL TIME' : `${Math.min(90, Math.floor(sim.clock * 90))}'`, W - 10, H - BOT / 2 + 1);
    if (sim.lastGoal && now - sim.lastGoal.at < 2500) {
      const scorer = TV_TEAMS[sim.lastGoal.side];
      g.fillStyle = scorer.shirt;
      g.fillRect(W / 2 - 110, H / 2 - 24, 220, 48);
      g.fillStyle = scorer.ink;
      g.font = '900 30px Inter, system-ui, sans-serif';
      g.textAlign = 'center';
      g.fillText('GOAL!', W / 2, H / 2 + 1);
    }
  }

  dispose() {
    for (const g of this.geos) g.dispose();
    this.mat.dispose();
    this.tv?.tex.dispose();
    this.group.traverse((o) => {
      const m = (o as Mesh).material as MeshStandardMaterial | MeshBasicMaterial | undefined;
      if (m && m !== this.mat) m.dispose();
    });
  }
}
