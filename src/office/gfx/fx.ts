// One-shot effects, each tied to a real event: confetti (tests pass, a merge), a paper plane
// (a question or message to a Lead), sparkles (a file created), a puff (a file deleted), a
// soft glow (a shelf answering a retrieval, a drawer taking a memory). They run for a second
// or two and then stop asking for frames.

import {
  AdditiveBlending,
  BufferAttribute,
  BufferGeometry,
  Color,
  Group,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  Points,
  PointsMaterial,
  QuadraticBezierCurve3,
  Sprite,
  SpriteMaterial,
  Vector3,
  type Texture,
} from 'three';
import { lin, type RGB } from '../palette';
import { rng } from '../text';

export interface Effect {
  readonly object: Group | Mesh | Points | Sprite;
  /** Advance; false when finished. */
  update(now: number): boolean;
  dispose(): void;
}

export class FxSystem {
  readonly group = new Group();
  private list: Effect[] = [];
  add(e: Effect) {
    this.list.push(e);
    this.group.add(e.object);
  }
  update(now: number): boolean {
    let alive = false;
    for (let i = this.list.length - 1; i >= 0; i--) {
      const e = this.list[i];
      if (e.update(now)) alive = true;
      else {
        this.group.remove(e.object);
        e.dispose();
        this.list.splice(i, 1);
      }
    }
    return alive;
  }
  clear() {
    for (const e of this.list) {
      this.group.remove(e.object);
      e.dispose();
    }
    this.list = [];
  }
  get size() {
    return this.list.length;
  }
}

/** Confetti: a burst of colored flecks with gravity. */
export class Confetti implements Effect {
  readonly object: Points;
  private geo = new BufferGeometry();
  private mat: PointsMaterial;
  private vel: Float32Array;
  private base: Float32Array;
  constructor(
    at: Vector3,
    colors: RGB[],
    private t0: number,
    seed: number,
    count = 70,
    private ms = 1700,
    spark: Texture,
  ) {
    const r = rng(seed);
    const pos = new Float32Array(count * 3);
    const col = new Float32Array(count * 3);
    this.vel = new Float32Array(count * 3);
    for (let i = 0; i < count; i++) {
      pos[i * 3] = at.x;
      pos[i * 3 + 1] = at.y;
      pos[i * 3 + 2] = at.z;
      const a = r() * Math.PI * 2;
      const sp = 0.8 + r() * 1.6;
      this.vel[i * 3] = Math.cos(a) * sp * 0.6;
      this.vel[i * 3 + 1] = 2.2 + r() * 2.2;
      this.vel[i * 3 + 2] = Math.sin(a) * sp * 0.6;
      const c = colors[i % colors.length];
      col[i * 3] = c[0];
      col[i * 3 + 1] = c[1];
      col[i * 3 + 2] = c[2];
    }
    this.base = pos.slice();
    this.geo.setAttribute('position', new BufferAttribute(pos, 3));
    this.geo.setAttribute('color', new BufferAttribute(col, 3));
    void spark;
    this.mat = new PointsMaterial({ size: 0.13, vertexColors: true, transparent: true, depthWrite: false });
    this.object = new Points(this.geo, this.mat);
    this.object.frustumCulled = false;
  }
  update(now: number): boolean {
    const t = (now - this.t0) / 1000;
    if (t < 0) return true;
    const P = this.geo.getAttribute('position') as BufferAttribute;
    const a = P.array as Float32Array;
    for (let i = 0; i < a.length; i += 3) {
      a[i] = this.base[i] + this.vel[i] * t;
      a[i + 1] = Math.max(0.02, this.base[i + 1] + this.vel[i + 1] * t - 4.9 * t * t);
      a[i + 2] = this.base[i + 2] + this.vel[i + 2] * t;
    }
    P.needsUpdate = true;
    this.mat.opacity = Math.max(0, 1 - Math.max(0, t * 1000 - this.ms * 0.6) / (this.ms * 0.4));
    return t * 1000 < this.ms;
  }
  dispose() {
    this.geo.dispose();
    this.mat.dispose();
  }
}

/** A folded paper plane on an arc, A → B. */
export class PaperPlane implements Effect {
  readonly object = new Group();
  private curve: QuadraticBezierCurve3;
  private geo: BufferGeometry;
  private mat = new MeshStandardMaterial({ color: 0xfbf8f1, roughness: 0.7, side: 2 });
  private ms: number;
  private tmp = new Vector3();
  constructor(
    from: Vector3,
    to: Vector3,
    private t0: number,
    private onLand?: () => void,
  ) {
    const mid = from.clone().add(to).multiplyScalar(0.5);
    mid.y = Math.max(from.y, to.y) + 1.2 + from.distanceTo(to) * 0.08;
    this.curve = new QuadraticBezierCurve3(from, mid, to);
    this.ms = Math.min(2000, Math.max(900, from.distanceTo(to) * 120));
    // A simple dart: two wings and a keel.
    const v = new Float32Array([0, 0, 0.16, -0.09, 0.01, -0.08, 0, 0, -0.06, 0, 0, 0.16, 0, 0, -0.06, 0.09, 0.01, -0.08, 0, 0, 0.16, 0, -0.04, -0.06, 0, 0, -0.06].map((x) => x * 1.8));
    this.geo = new BufferGeometry();
    this.geo.setAttribute('position', new BufferAttribute(v, 3));
    this.geo.computeVertexNormals();
    const m = new Mesh(this.geo, this.mat);
    m.castShadow = true;
    this.object.add(m);
    this.object.position.copy(from);
  }
  update(now: number): boolean {
    const t = Math.min(1, (now - this.t0) / this.ms);
    const k = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
    this.curve.getPoint(k, this.object.position);
    this.curve.getTangent(Math.min(0.999, k + 0.001), this.tmp);
    this.object.lookAt(this.tmp.add(this.object.position));
    this.object.rotation.z += Math.sin(t * Math.PI * 3) * 0.2;
    if (t >= 1) {
      this.onLand?.();
      this.onLand = undefined;
      return false;
    }
    return true;
  }
  dispose() {
    this.geo.dispose();
    this.mat.dispose();
  }
}

/** A few twinkling sparkles (a file created). */
export class Sparkles implements Effect {
  readonly object = new Group();
  private sprites: Sprite[] = [];
  private mat: SpriteMaterial;
  constructor(
    at: Vector3,
    color: RGB,
    private t0: number,
    spark: Texture,
    seed: number,
  ) {
    this.mat = new SpriteMaterial({ map: spark, color: new Color().setRGB(...color), transparent: true, depthWrite: false, blending: AdditiveBlending });
    const r = rng(seed);
    for (let i = 0; i < 4; i++) {
      const s = new Sprite(this.mat);
      s.position.set(at.x + (r() - 0.5) * 0.4, at.y + r() * 0.3, at.z + (r() - 0.5) * 0.4);
      s.userData.d = i * 120;
      this.sprites.push(s);
      this.object.add(s);
    }
  }
  update(now: number): boolean {
    const t = now - this.t0;
    for (const s of this.sprites) {
      const k = Math.max(0, Math.min(1, (t - s.userData.d) / 600));
      const size = Math.sin(k * Math.PI) * 0.22;
      s.scale.set(size, size, 1);
    }
    return t < 1100;
  }
  dispose() {
    this.mat.dispose();
  }
}

/** A soft puff (a file deleted, an envelope dropped). */
export class Puff implements Effect {
  readonly object = new Group();
  private mat: SpriteMaterial;
  private sprites: Sprite[] = [];
  constructor(
    at: Vector3,
    color: RGB,
    private t0: number,
    soft: Texture,
  ) {
    this.mat = new SpriteMaterial({ map: soft, color: new Color().setRGB(...color), transparent: true, depthWrite: false, opacity: 0.7 });
    for (let i = 0; i < 5; i++) {
      const s = new Sprite(this.mat);
      const a = (i / 5) * Math.PI * 2;
      s.userData.dir = new Vector3(Math.cos(a), 0.6, Math.sin(a));
      s.position.copy(at);
      this.sprites.push(s);
      this.object.add(s);
    }
    this.object.userData.at = at.clone();
  }
  update(now: number): boolean {
    const t = Math.min(1, (now - this.t0) / 700);
    const at = this.object.userData.at as Vector3;
    for (const s of this.sprites) {
      s.position.copy(at).addScaledVector(s.userData.dir as Vector3, t * 0.25);
      const size = 0.12 + t * 0.2;
      s.scale.set(size, size, 1);
    }
    this.mat.opacity = 0.7 * (1 - t);
    return t < 1;
  }
  dispose() {
    this.mat.dispose();
  }
}

/** A soft glow that swells and fades in place. */
export class Glow implements Effect {
  readonly object: Sprite;
  private mat: SpriteMaterial;
  constructor(
    at: Vector3,
    color: RGB,
    private size: number,
    private t0: number,
    glow: Texture,
    private ms = 1400,
  ) {
    this.mat = new SpriteMaterial({ map: glow, color: new Color().setRGB(...color), transparent: true, depthWrite: false, blending: AdditiveBlending });
    this.object = new Sprite(this.mat);
    this.object.position.copy(at);
  }
  update(now: number): boolean {
    const t = Math.min(1, (now - this.t0) / this.ms);
    const s = this.size * (0.6 + 0.4 * Math.sin(Math.min(1, t * 2) * Math.PI * 0.5));
    this.object.scale.set(s, s, 1);
    this.mat.opacity = t < 0.3 ? t / 0.3 : 1 - (t - 0.3) / 0.7;
    return t < 1;
  }
  dispose() {
    this.mat.dispose();
  }
}

/** A ring that expands on the floor (someone arrives / leaves). */
export class FloorRing implements Effect {
  readonly object: Mesh;
  private mat: MeshBasicMaterial;
  constructor(
    at: Vector3,
    color: RGB,
    private t0: number,
    ring: Texture,
  ) {
    this.mat = new MeshBasicMaterial({ map: ring, color: new Color().setRGB(...color), transparent: true, depthWrite: false, blending: AdditiveBlending });
    const g = new BufferGeometry();
    const s = 0.5;
    g.setAttribute('position', new BufferAttribute(new Float32Array([-s, 0, -s, s, 0, -s, s, 0, s, -s, 0, -s, s, 0, s, -s, 0, s]), 3));
    g.setAttribute('uv', new BufferAttribute(new Float32Array([0, 0, 1, 0, 1, 1, 0, 0, 1, 1, 0, 1]), 2));
    this.object = new Mesh(g, this.mat);
    this.object.position.set(at.x, 0.02, at.z);
  }
  update(now: number): boolean {
    const t = Math.min(1, (now - this.t0) / 900);
    const s = 0.4 + t * 1.6;
    this.object.scale.set(s, 1, s);
    this.mat.opacity = 1 - t;
    return t < 1;
  }
  dispose() {
    this.object.geometry.dispose();
    this.mat.dispose();
  }
}

export const CONFETTI: RGB[] = [lin('#ffb547'), lin('#5ee08f'), lin('#6ab8ff'), lin('#ff7a8a'), lin('#c6a7ff'), lin('#fff1c4')];
