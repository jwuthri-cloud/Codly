// A tiny mesh builder: boxes, cylinders, spheres and blobs with per-vertex colors, merged into
// one BufferGeometry. Furniture, walls and characters are built from these so a whole piece is
// one draw call (and repeated pieces are instanced). Low-poly by design.

import {
  BoxGeometry,
  BufferAttribute,
  BufferGeometry,
  CylinderGeometry,
  IcosahedronGeometry,
  Matrix3,
  Matrix4,
  Quaternion,
  SphereGeometry,
  Vector3,
  type Euler,
} from 'three';
import type { RGB } from '../palette';

const prims = new Map<string, BufferGeometry>();
function prim(key: string, make: () => BufferGeometry): BufferGeometry {
  let g = prims.get(key);
  if (!g) {
    g = make();
    g = g.index ? g.toNonIndexed() : g;
    prims.set(key, g);
  }
  return g;
}

const BOX = () => prim('box', () => new BoxGeometry(1, 1, 1));
const CYL = (seg: number) => prim('cyl' + seg, () => new CylinderGeometry(1, 1, 1, seg, 1));
const CONE = (seg: number, top: number) => prim(`cone${seg}:${top}`, () => new CylinderGeometry(top, 1, 1, seg, 1));
const SPH = (w: number, h: number) => prim(`sph${w}x${h}`, () => new SphereGeometry(1, w, h));
const ICO = (detail: number) => prim('ico' + detail, () => new IcosahedronGeometry(1, detail));

const m4 = new Matrix4();
const m3 = new Matrix3();
const v = new Vector3();
const q = new Quaternion();
const s = new Vector3();
const p = new Vector3();
const UP = new Vector3(0, 1, 0);

export class Builder {
  private pos: number[] = [];
  private nor: number[] = [];
  private col: number[] = [];
  /** Base transform applied to everything (an item's placement). */
  base = new Matrix4();

  /** Place subsequent parts in an item frame: at (x, y, z), turned `rot` around Y. */
  frame(x = 0, y = 0, z = 0, rot = 0): this {
    this.base.makeRotationY(rot).setPosition(x, y, z);
    return this;
  }

  /** Add a (non-indexed) geometry under `local` (then the base frame), in one color (times its own vertex colors, if any). */
  add(geo: BufferGeometry, color: RGB, local: Matrix4, flat = false, shade = 1): this {
    m4.multiplyMatrices(this.base, local);
    m3.getNormalMatrix(m4);
    const P = geo.getAttribute('position');
    const Nn = geo.getAttribute('normal');
    const C = geo.getAttribute('color');
    const n = P.count;
    const start = this.pos.length;
    for (let i = 0; i < n; i++) {
      v.fromBufferAttribute(P, i).applyMatrix4(m4);
      this.pos.push(v.x, v.y, v.z);
      if (!flat && Nn) {
        v.fromBufferAttribute(Nn, i).applyMatrix3(m3).normalize();
        this.nor.push(v.x, v.y, v.z);
      }
      const k = C ? C.getX(i) * shade : shade;
      this.col.push(color[0] * k, color[1] * k, color[2] * k);
    }
    if (flat || !Nn) {
      const a = new Vector3();
      const b = new Vector3();
      const c = new Vector3();
      for (let i = start; i < this.pos.length; i += 9) {
        a.fromArray(this.pos, i);
        b.fromArray(this.pos, i + 3);
        c.fromArray(this.pos, i + 6);
        c.sub(b);
        a.sub(b);
        c.cross(a).normalize();
        this.nor.push(c.x, c.y, c.z, c.x, c.y, c.z, c.x, c.y, c.z);
      }
    }
    return this;
  }

  /** Box centered at (x, y, z) with size (w, h, d), optionally turned around Y. */
  box(x: number, y: number, z: number, w: number, h: number, d: number, color: RGB, rotY = 0): this {
    q.setFromAxisAngle(UP, rotY);
    return this.add(BOX(), color, m4.clone().compose(p.set(x, y, z), q, s.set(w, h, d)), false);
  }

  /** Box from its min corner to max corner (axis-aligned in the frame). */
  span(x0: number, y0: number, z0: number, x1: number, y1: number, z1: number, color: RGB): this {
    return this.box((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2, Math.abs(x1 - x0), Math.abs(y1 - y0), Math.abs(z1 - z0), color);
  }

  /** Cylinder standing on (x, y, z) — base at y — radius r (rTop for a taper), height h. */
  cyl(x: number, y: number, z: number, r: number, h: number, color: RGB, seg = 12, rTop?: number): this {
    const geo = rTop == null ? CYL(seg) : CONE(seg, Math.round((rTop / r) * 100) / 100);
    q.identity();
    return this.add(geo, color, m4.clone().compose(p.set(x, y + h / 2, z), q, s.set(r, h, r)), seg <= 6);
  }

  /** Cylinder along an arbitrary axis between two points. */
  rod(a: Vector3, b: Vector3, r: number, color: RGB, seg = 8): this {
    const dir = b.clone().sub(a);
    const len = dir.length();
    q.setFromUnitVectors(UP, dir.normalize());
    const mid = a.clone().add(b).multiplyScalar(0.5);
    return this.add(CYL(seg), color, m4.clone().compose(mid, q, s.set(r, len, r)));
  }

  /** Ellipsoid centered at (x, y, z) with radii (rx, ry, rz). */
  ball(x: number, y: number, z: number, rx: number, ry: number, rz: number, color: RGB, w = 14, h = 10, rot?: Euler): this {
    if (rot) q.setFromEuler(rot);
    else q.identity();
    return this.add(SPH(w, h), color, m4.clone().compose(p.set(x, y, z), q, s.set(rx, ry, rz)));
  }

  /** Faceted blob (foliage, clouds). */
  blob(x: number, y: number, z: number, rx: number, ry: number, rz: number, color: RGB, detail = 0, rotY = 0): this {
    q.setFromAxisAngle(UP, rotY);
    return this.add(ICO(detail), color, m4.clone().compose(p.set(x, y, z), q, s.set(rx, ry, rz)), true);
  }

  /** A flat polygon (badge glyphs) facing +z at depth z, from 2D points around (cx, cy). */
  poly(points: [number, number][], z: number, color: RGB): this {
    const g = new BufferGeometry();
    const arr: number[] = [];
    let cx = 0;
    let cy = 0;
    for (const [x, y] of points) {
      cx += x;
      cy += y;
    }
    cx /= points.length;
    cy /= points.length;
    for (let i = 0; i < points.length; i++) {
      const [x0, y0] = points[i];
      const [x1, y1] = points[(i + 1) % points.length];
      arr.push(cx, cy, z, x0, y0, z, x1, y1, z);
    }
    g.setAttribute('position', new BufferAttribute(new Float32Array(arr), 3));
    const nrm = new Float32Array(arr.length);
    for (let i = 2; i < nrm.length; i += 3) nrm[i] = 1;
    g.setAttribute('normal', new BufferAttribute(nrm, 3));
    this.add(g, color, new Matrix4());
    g.dispose();
    return this;
  }

  get empty() {
    return this.pos.length === 0;
  }

  build(): BufferGeometry {
    const g = new BufferGeometry();
    g.setAttribute('position', new BufferAttribute(new Float32Array(this.pos), 3));
    g.setAttribute('normal', new BufferAttribute(new Float32Array(this.nor), 3));
    g.setAttribute('color', new BufferAttribute(new Float32Array(this.col), 3));
    g.computeBoundingSphere();
    g.computeBoundingBox();
    return g;
  }
}
