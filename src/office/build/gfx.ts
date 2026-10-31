// Build mode's overlay on the office floor (floor-plan coordinates, the engine's `buildLayer`):
// the 0.5 m grid, and each tool's preview: walls going up (green, or red where they can't),
// tiles to paint, a wall or opening picked, a room's push/pull arrows, a room being carried.

import { BoxGeometry, BufferGeometry, ConeGeometry, Float32BufferAttribute, Group, LineBasicMaterial, LineSegments, Mesh, MeshBasicMaterial, PlaneGeometry, type Material } from 'three';
import type { OfficeLayout } from '../layout';

export type Shape =
  /** A wall-like box from (x0, z0) to (x1, z1), `h` tall. */
  | { kind: 'wall'; x0: number; z0: number; x1: number; z1: number; h: number; tone: Tone }
  /** Floor rectangles. */
  | { kind: 'floor'; x: number; z: number; w: number; d: number; tone: Tone }
  /** A push/pull arrow at (x, z) pointing (nx, nz). */
  | { kind: 'arrow'; x: number; z: number; nx: number; nz: number; tone: Tone };

export type Tone = 'ok' | 'bad' | 'pick' | 'hot';

const COLOR: Record<Tone, number> = { ok: 0x7ee0a1, bad: 0xff8f80, pick: 0xffffff, hot: 0xffd27a };

export class BuildLayer {
  readonly group = new Group();
  private grid: LineSegments[] = [];
  private shapes = new Group();
  private mats = new Map<string, Material>();
  private box = new BoxGeometry(1, 1, 1);
  private plane = new PlaneGeometry(1, 1).rotateX(-Math.PI / 2);
  private cone = new ConeGeometry(0.22, 0.42, 3).rotateZ(-Math.PI / 2);

  constructor() {
    this.group.add(this.shapes);
  }

  /** The grid over the lot: a faint line every 0.5 m, a stronger one every metre. */
  setLot(L: OfficeLayout) {
    for (const g of this.grid) {
      g.geometry.dispose();
      (g.material as Material).dispose();
      this.group.remove(g);
    }
    this.grid = [true, false].map((whole) => {
      const pos: number[] = [];
      for (let x = 0; x <= L.w + 1e-6; x += 0.5) if ((Math.round(x) === x) === whole) pos.push(x, 0.012, 0, x, 0.012, L.d);
      for (let z = 0; z <= L.d + 1e-6; z += 0.5) if ((Math.round(z) === z) === whole) pos.push(0, 0.012, z, L.w, 0.012, z);
      const geo = new BufferGeometry();
      geo.setAttribute('position', new Float32BufferAttribute(pos, 3));
      const g = new LineSegments(geo, new LineBasicMaterial({ color: 0x1d1610, transparent: true, opacity: whole ? 0.42 : 0.2, depthWrite: false, toneMapped: false }));
      g.renderOrder = 5;
      this.group.add(g);
      return g;
    });
  }

  private mat(tone: Tone, solid: boolean): Material {
    const key = `${tone}:${solid}`;
    let m = this.mats.get(key);
    if (!m) this.mats.set(key, (m = new MeshBasicMaterial({ color: COLOR[tone], transparent: true, opacity: solid ? 0.55 : 0.28, depthWrite: false, depthTest: !solid || tone !== 'hot', toneMapped: false })));
    return m;
  }

  /** What the tool shows now (replaces the last). */
  show(shapes: Shape[]) {
    this.shapes.clear();
    for (const s of shapes) {
      if (s.kind === 'wall') {
        const m = new Mesh(this.box, this.mat(s.tone, true));
        const T = 0.2;
        m.scale.set(Math.max(T, s.x1 - s.x0 + (s.z0 === s.z1 ? 0 : T)), s.h, Math.max(T, s.z1 - s.z0 + (s.x0 === s.x1 ? 0 : T)));
        m.position.set((s.x0 + s.x1) / 2, s.h / 2, (s.z0 + s.z1) / 2);
        m.renderOrder = 6;
        this.shapes.add(m);
      } else if (s.kind === 'floor') {
        const m = new Mesh(this.plane, this.mat(s.tone, false));
        m.scale.set(s.w, 1, s.d);
        m.position.set(s.x + s.w / 2, 0.02, s.z + s.d / 2);
        m.renderOrder = 6;
        this.shapes.add(m);
      } else {
        const m = new Mesh(this.cone, this.mat(s.tone, true));
        m.position.set(s.x, 0.3, s.z);
        m.rotation.y = Math.atan2(-s.nz, s.nx);
        m.renderOrder = 7;
        this.shapes.add(m);
      }
    }
  }

  setVisible(on: boolean) {
    this.group.visible = on;
  }

  dispose() {
    this.shapes.clear();
    for (const g of this.grid) {
      g.geometry.dispose();
      (g.material as Material).dispose();
    }
    for (const m of this.mats.values()) m.dispose();
    this.box.dispose();
    this.plane.dispose();
    this.cone.dispose();
    this.group.clear();
  }
}
