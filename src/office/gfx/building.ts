// The static office (from its lot): a floating floor slab, the floor atlas, cutaway walls (full-height back
// walls with windows, low interior walls with dark caps, glass partitions), all furniture,
// lamps and the evening lights. Repeated furniture (desks, chairs, monitors, lamps, racks,
// cabinets) is instanced; one-off pieces are merged into a single vertex-colored mesh.

import {
  AdditiveBlending,
  BufferAttribute,
  BufferGeometry,
  Color,
  DoubleSide,
  Group,
  InstancedMesh,
  Matrix4,
  Mesh,
  MeshBasicMaterial,
  type Object3D,
  MeshStandardMaterial,
  PlaneGeometry,
  PointLight,
  Quaternion,
  Vector3,
  type Texture,
} from 'three';
import { DESK_H, GLASS_H, INNER_H, LOW_H, WALL_H, type Item, type ItemType, type OfficeLayout, type Wall } from '../layout';
import { color, lin, MAT, mix, scale, type OfficeTheme, type RGB, type SkyKind } from '../palette';
import { hash } from '../text';
import { paperOf } from '../build/paints';
import type { DoorKind } from '../../lib/gen/DoorKind';
import { Builder } from './builder';
import { modeled } from './builtins';
import * as F from './furniture';
import { c } from './furniture';
import { floorTexture, glowTexture, skyTexture } from './textures';

const WT = 0.16;
const SILL = 0.78;
const LINTEL = 1.88;

type Proto = { key: string; geo: BufferGeometry; items: Item[] };

/** Hung on a wall rather than standing on the floor. */
const HUNG = new Set<ItemType>(['whiteboard', 'poster']);

const INSTANCED: Partial<Record<ItemType, (b: Builder, variant: number) => void>> = {
  desk: (b) => F.desk(b),
  leadDesk: (b) => F.leadDesk(b),
  chair: (b, v) => F.chair(b, v),
  monitor: (b, v) => F.monitor(b, v),
  lamp: (b) => F.lampBody(b),
  divider: (b) => F.divider(b),
  rack: (b) => F.rack(b),
  cabinet: (b) => F.cabinet(b),
};

export class BuildingGfx {
  readonly group = new Group();
  private matte = new MeshStandardMaterial({ vertexColors: true, roughness: 0.86, metalness: 0 });
  private floorMat: MeshStandardMaterial;
  private glassMat = new MeshStandardMaterial({ color: 0xd8ecf2, transparent: true, opacity: 0.16, roughness: 0.08, metalness: 0, depthWrite: false, side: DoubleSide });
  private skyMat = new MeshBasicMaterial({ toneMapped: false });
  private shadeMat = new MeshStandardMaterial({ color: color(lin(MAT.shade)), roughness: 0.6, emissive: new Color(0, 0, 0) });
  private deskShadeMat = new MeshBasicMaterial({ color: 0xffffff });
  private screenMat = new MeshBasicMaterial({ vertexColors: true });
  private poolMat: MeshBasicMaterial;
  private textures: Texture[] = [];
  private geos: BufferGeometry[] = [];
  private skies: Partial<Record<SkyKind, Texture>> = {};
  private deskShades: InstancedMesh | null = null;
  private deskPools: InstancedMesh | null = null;
  private lampState: ('off' | 'on' | 'amber')[] = [];
  private staticPools: Mesh[] = [];
  private lights: PointLight[] = [];
  private theme: OfficeTheme | null = null;
  /** Desk index → instance id in the lamp meshes. */
  private lampIndex = new Map<number, number>();
  private wallMeshes: Mesh[] = [];
  /** The full-height exterior walls, each with its outward normal: lowered when it faces the camera. */
  private sides: { mesh: Mesh; nx: number; nz: number }[] = [];
  /** What hangs on an exterior wall (whiteboard, posters): hidden while its wall is down. */
  private hung: { obj: Object3D; nx: number; nz: number }[] = [];
  private wallsUp = true;
  /** Cutaway: the outer walls facing the camera drop to the low walls' height. */
  private cutaway = true;
  private camDir: [number, number] = [0, 1];

  constructor(private L: OfficeLayout) {
    const floorTex = floorTexture(L);
    this.textures.push(floorTex);
    this.floorMat = new MeshStandardMaterial({ map: floorTex, roughness: 0.92, metalness: 0 });
    const glow = glowTexture();
    this.textures.push(glow);
    this.poolMat = new MeshBasicMaterial({ map: glow, transparent: true, blending: AdditiveBlending, depthWrite: false, toneMapped: false });
    this.buildSlab();
    this.buildWalls();
    this.buildFurniture();
    this.buildLamps();
    this.applyWalls();
  }

  private mesh(geo: BufferGeometry, mat: MeshStandardMaterial | MeshBasicMaterial, cast = true, receive = true): Mesh {
    this.geos.push(geo);
    const m = new Mesh(geo, mat);
    m.castShadow = cast;
    m.receiveShadow = receive;
    m.matrixAutoUpdate = false;
    m.updateMatrix();
    this.group.add(m);
    return m;
  }

  private buildSlab() {
    const b = new Builder();
    const slab = c(MAT.slab);
    const { w: W, d: D, door } = this.L;
    // A floating slab with a little porch at the entrance (in the door's frame: x along the
    // wall, z out of the lot).
    b.span(-0.3, -0.55, -0.3, W + 0.3, -0.005, D + 0.3, slab);
    b.span(-0.36, -0.6, -0.36, W + 0.36, -0.5, D + 0.36, mix(slab, [0, 0, 0], 0.25));
    const pw = Math.max(1.3, door.width / 2 + 0.5);
    b.frame(door.x, 0, door.z, door.rot);
    b.span(-pw, -0.3, 0.3, pw, -0.02, 1.1, c('#d9cfbf'));
    b.span(-pw + 0.2, -0.55, 1.1, pw - 0.2, -0.32, 1.5, c('#cfc4b2'));
    b.frame();
    this.mesh(b.build(), this.matte, false, true);
    // Floor.
    const fg = new PlaneGeometry(W, D).rotateX(-Math.PI / 2).translate(W / 2, 0, D / 2);
    this.mesh(fg, this.floorMat, false, true);
    // A soft shadow under the floating diorama.
    const sh = new PlaneGeometry(W * 1.35, D * 1.35).rotateX(-Math.PI / 2).translate(W / 2 + 0.8, -1.2, D / 2 + 0.8);
    const shMat = new MeshBasicMaterial({ map: this.poolMat.map, color: 0x000000, transparent: true, opacity: 0.32, depthWrite: false });
    this.extraMats.push(shMat);
    const m = this.mesh(sh, shMat, false, false);
    m.renderOrder = -1;
  }

  private buildWalls() {
    const walls = new Builder();
    const glass = new Builder();
    const plaster = c(MAT.wall);
    const cap = c(MAT.wallCap);
    const base = c(MAT.baseboard);
    const frame = c('#f4efe6');
    for (const w of this.L.walls) {
      const horiz = w.z0 === w.z1;
      const len = horiz ? w.x1 - w.x0 : w.z1 - w.z0;
      // A full-height outer wall gets its own mesh, to drop it when the camera turns to face it.
      const side = w.kind === 'exterior';
      const b = side ? new Builder() : walls;
      // Solid spans between gaps.
      const gaps = [...w.gaps].sort((a, b2) => a[0] - b2[0]);
      const spans: [number, number][] = [];
      let at = 0;
      for (const [a, z] of [...gaps, [len, len] as [number, number]]) {
        if (a > at + 1e-6) spans.push([at, a]);
        at = Math.max(at, z);
      }
      const seg = (a: number, z: number, y0: number, y1: number, col: RGB, t = WT, inset = 0) => {
        if (z - a < 1e-4 || y1 - y0 < 1e-4) return;
        if (horiz) b.span(w.x0 + a, y0, w.z0 - t / 2 + inset, w.x0 + z, y1, w.z0 + t / 2 + inset, col);
        else b.span(w.x0 - t / 2 + inset, y0, w.z0 + a, w.x0 + t / 2 + inset, y1, w.z0 + z, col);
      };
      const doors = this.L.lot.doors.filter((d) => d.wall === w.id);
      if (w.kind === 'glass') {
        for (const [a, z] of spans) this.glassSpan(b, glass, w, a, z);
        // Door posts.
        for (const [a, z] of gaps) {
          seg(a - 0.04, a + 0.04, 0, GLASS_H + 0.06, c('#5c4a3d'), 0.1);
          seg(z - 0.04, z + 0.04, 0, GLASS_H + 0.06, c('#5c4a3d'), 0.1);
          seg(a, z, GLASS_H - 0.02, GLASS_H + 0.06, c('#5c4a3d'), 0.1);
        }
        w.gaps.forEach(([a, z], i) => doors[i] && this.leaves(b, glass, w, a, z, doors[i].kind, GLASS_H));
        continue;
      }
      const H = w.kind === 'exterior' ? WALL_H : w.kind === 'solid' ? INNER_H : LOW_H;
      const windows = w.windows ?? [];
      // The faces a side's paint covers: the wall, less its doors and windows.
      const faces: [number, number, number, number][] = [];
      const face = (a: number, z: number, y0: number, y1: number) => {
        seg(a, z, y0, y1, plaster);
        faces.push([a, z, y0, y1]);
      };
      for (const [a, z] of spans) {
        // Split around windows.
        let cur = a;
        for (const [wa, wz] of windows) {
          if (wz <= cur || wa >= z) continue;
          face(cur, wa, 0, H);
          face(wa, wz, 0, SILL);
          face(wa, wz, LINTEL, H);
          this.window(b, w, wa, wz, frame);
          cur = wz;
        }
        face(cur, z, 0, H);
        // Dark cap where the wall is cut, and a baseboard on both faces.
        seg(a, z, H, H + 0.03, cap, WT + 0.02);
        seg(a, z, 0, 0.09, base, WT + 0.03);
      }
      // Rounded-off ends at door gaps: a slim jamb in the cap color, standing a hair proud of
      // the wall's end face (flush, the two faces z-fight into stripes).
      for (const [ga, gz] of gaps) {
        if (ga > 0) seg(ga - 0.03, ga + 0.005, 0, H + 0.03, cap, WT + 0.02);
        if (gz < len) seg(gz - 0.005, gz + 0.03, 0, H + 0.03, cap, WT + 0.02);
      }
      // Paint and wallpaper: a thin skin on the painted side, above the baseboard.
      for (const p of this.L.lot.paint) {
        if (p.wall !== w.id || (p.side !== 1 && p.side !== -1)) continue;
        const skin = (a: number, z: number, y0: number, y1: number, col: RGB, t = 0.008) => seg(a, z, Math.max(0.09, y0), y1, col, t, p.side * (WT / 2 + t / 2 - 0.002));
        for (const [a, z, y0, y1] of faces) this.paint(skin, p.paint, a, z, y0, y1);
      }
      w.gaps.forEach(([a, z], i) => doors[i] && this.leaves(b, glass, w, a, z, doors[i].kind, H));
      if (side) {
        const mid = horiz ? [0, w.z0 < this.L.d / 2 ? -1 : 1] : [w.x0 < this.L.w / 2 ? -1 : 1, 0];
        this.sides.push({ mesh: this.mesh(b.build(), this.matte, true, true), nx: mid[0], nz: mid[1] });
      }
    }
    const b = walls;
    // The entrance threshold and frame (a garage: the roll-up door, open).
    const door = this.L.door;
    b.frame(door.x, 0, door.z, door.rot);
    if (door.kind === 'rollup') F.rollupDoor(b, door.width);
    else F.doorFrame(b);
    b.frame();
    const wm = this.mesh(b.build(), this.matte, true, true);
    const gm = this.mesh(glass.build(), this.glassMat, false, false);
    gm.renderOrder = 2;
    this.wallMeshes = [wm, gm];
  }

  /** One painted face: a colour, or a wallpaper's pattern (stripes and checks line up along the wall). */
  private paint(skin: (a: number, z: number, y0: number, y1: number, col: RGB, t?: number) => void, id: string, a: number, z: number, y0: number, y1: number) {
    const p = paperOf(id);
    if (!p) return skin(a, z, y0, y1, c(/^#[0-9a-f]{6}$/i.test(id) ? id : MAT.wall));
    if (p.look === 'stripes') {
      for (let i = Math.floor(a / p.w); i * p.w < z; i++) skin(Math.max(a, i * p.w), Math.min(z, (i + 1) * p.w), y0, y1, c(i % 2 ? p.b : p.a));
    } else if (p.look === 'dado') {
      skin(a, z, y0, Math.min(y1, p.rail), c(p.b));
      if (y1 > p.rail) {
        skin(a, z, Math.max(y0, p.rail), Math.min(y1, p.rail + 0.05), c('#f4efe6'), 0.016);
        skin(a, z, Math.max(y0, p.rail + 0.05), y1, c(p.a));
      }
    } else {
      for (let i = Math.floor(a / p.w); i * p.w < z; i++)
        for (let j = Math.floor(y0 / p.w); j * p.w < y1; j++) skin(Math.max(a, i * p.w), Math.min(z, (i + 1) * p.w), Math.max(y0, j * p.w), Math.min(y1, (j + 1) * p.w), c((i + j) % 2 ? p.b : p.a));
    }
  }

  /** A door's leaves in the gap [a, z] along a wall, standing open towards its +z / +x side
   *  (simple shapes: a wood leaf, two for a double door, a framed pane for glass, a rolled-up
   *  drum for a roll-up door). As tall as the wall, so a cut-away wall cuts its door too. */
  private leaves(b: Builder, glass: Builder, w: Wall, a: number, z: number, kind: DoorKind, H: number) {
    if (kind === 'open') return;
    const horiz = w.z0 === w.z1;
    const at = (u: number): [number, number] => (horiz ? [w.x0 + u, w.z0] : [w.x0, w.z0 + u]);
    const h = H < 1 ? H - 0.03 : Math.min(2.02, H - 0.1);
    const wood = c('#a87c55');
    const trim = c('#8a6343');
    if (kind === 'rollup') {
      const steel = c('#7d828a');
      for (const u of [a, z]) {
        const [x, zz] = at(u);
        b.box(x, H / 2, zz, horiz ? 0.08 : 0.2, H, horiz ? 0.2 : 0.08, c('#5d6168'));
      }
      const [mx, mz] = at((a + z) / 2);
      const len = z - a;
      b.box(mx, H - 0.13, mz, horiz ? len : 0.26, 0.22, horiz ? 0.26 : len, steel);
      b.box(mx, H - 0.26, mz, horiz ? len : 0.05, 0.04, horiz ? 0.05 : len, c('#5d6168'));
      return;
    }
    const open = (70 * Math.PI) / 180;
    // One leaf hinged at `u`, `lw` wide, closing towards `dir` (+1: along the wall).
    const leaf = (u: number, lw: number, dir: 1 | -1) => {
      const [hx, hz] = at(u);
      const [dx, dz] = horiz ? [dir * Math.cos(open), Math.sin(open)] : [Math.sin(open), dir * Math.cos(open)];
      const rot = Math.atan2(-dz, dx);
      b.frame(hx, 0, hz, rot);
      if (kind === 'glass') {
        const f = c('#5c4a3d');
        b.box(lw / 2, 0.05, 0, lw, 0.1, 0.05, f).box(lw / 2, h - 0.03, 0, lw, 0.06, 0.05, f).box(0.025, h / 2, 0, 0.05, h, 0.05, f).box(lw - 0.025, h / 2, 0, 0.05, h, 0.05, f);
        glass.frame(hx, 0, hz, rot).box(lw / 2, h / 2, 0, lw - 0.08, h - 0.14, 0.015, [1, 1, 1]);
        glass.frame();
      } else {
        b.box(lw / 2, h / 2, 0, lw, h, 0.045, wood);
        if (h > 0.6) b.box(lw / 2, h / 2, 0, lw * 0.7, h * 0.8, 0.05, trim);
        b.ball(lw - 0.09, Math.min(h * 0.5, 0.95), 0.04, 0.025, 0.025, 0.025, c('#d9c79a'), 8, 6);
      }
      b.frame();
    };
    const gap = z - a - 0.04;
    if (kind === 'double') {
      leaf(a + 0.02, gap / 2, 1);
      leaf(z - 0.02, gap / 2, -1);
    } else leaf(a + 0.02, gap, 1);
  }

  /** Walls up, or down to knee-high stubs (The Sims' "walls down") to see into every room. */
  setWalls(up: boolean) {
    this.setMode(up ? 'cutaway' : 'down');
  }

  /** Walls up (all of them), cutaway (the outer walls facing the camera drop low), or down. */
  setMode(mode: 'up' | 'cutaway' | 'down') {
    this.wallsUp = mode !== 'down';
    this.cutaway = mode === 'cutaway';
    this.applyWalls();
  }

  /** How tall a wall stands right now (walls down, cut away), for picking it with the mouse. */
  shownHeight(id: string): number {
    const w = this.L.walls.find((x) => x.id === id);
    if (!w) return 0;
    const H = w.kind === 'exterior' ? WALL_H : w.kind === 'solid' ? INNER_H : w.kind === 'glass' ? GLASS_H : LOW_H;
    if (!this.wallsUp) return Math.max(0.3, H * 0.16);
    if (w.kind !== 'exterior') return H;
    const horiz = w.z0 === w.z1;
    const n = horiz ? { nx: 0, nz: w.z0 < this.L.d / 2 ? -1 : 1 } : { nx: w.x0 < this.L.w / 2 ? -1 : 1, nz: 0 };
    return this.lowered(n) ? LOW_H : H;
  }

  /** Where the camera is, seen from the floor (sin yaw, cos yaw): outer walls facing it drop
   *  to the low front walls' height. Returns whether anything changed. */
  faceCamera(yaw: number): boolean {
    const d: [number, number] = [Math.sin(yaw), Math.cos(yaw)];
    const was = this.sides.map((s) => this.lowered(s));
    this.camDir = d;
    if (this.sides.every((s, i) => this.lowered(s) === was[i])) return false;
    this.applyWalls();
    return true;
  }

  /** Hang something drawn elsewhere (the whiteboard's notes) on the exterior wall at (x, z). */
  hang(obj: Object3D, x: number, z: number) {
    const side = this.sideOf(x, z);
    if (!side) return;
    this.hung.push({ obj, ...side });
    this.applyWalls();
  }

  /** The outward normal of the exterior wall at (x, z), if (x, z) is against one. */
  private sideOf(x: number, z: number): { nx: number; nz: number } | null {
    for (const w of this.L.walls) {
      if (w.kind !== 'exterior') continue;
      if (w.z0 === w.z1 ? Math.abs(z - w.z0) < 0.5 : Math.abs(x - w.x0) < 0.5)
        return w.z0 === w.z1 ? { nx: 0, nz: w.z0 < this.L.d / 2 ? -1 : 1 } : { nx: w.x0 < this.L.w / 2 ? -1 : 1, nz: 0 };
    }
    return null;
  }

  private lowered(s: { nx: number; nz: number }) {
    return this.cutaway && s.nx * this.camDir[0] + s.nz * this.camDir[1] > 0.25;
  }

  private applyWalls() {
    for (const m of this.wallMeshes) {
      m.scale.y = this.wallsUp ? 1 : 0.16;
      m.updateMatrix();
    }
    for (const s of this.sides) {
      s.mesh.scale.y = !this.wallsUp ? 0.16 : this.lowered(s) ? LOW_H / WALL_H : 1;
      s.mesh.updateMatrix();
    }
    for (const h of this.hung) h.obj.visible = this.wallsUp && !this.lowered(h);
  }

  private window(b: Builder, w: Wall, a: number, z: number, frame: RGB) {
    const horiz = w.z0 === w.z1;
    const ft = 0.05;
    const box = (u0: number, u1: number, y0: number, y1: number, t: number, col: RGB) => {
      if (horiz) b.span(w.x0 + u0, y0, w.z0 - t / 2, w.x0 + u1, y1, w.z0 + t / 2, col);
      else b.span(w.x0 - t / 2, y0, w.z0 + u0, w.x0 + t / 2, y1, w.z0 + u1, col);
    };
    box(a - ft, a, SILL - 0.04, LINTEL + 0.04, WT + 0.04, frame);
    box(z, z + ft, SILL - 0.04, LINTEL + 0.04, WT + 0.04, frame);
    box(a, z, LINTEL, LINTEL + 0.05, WT + 0.04, frame);
    box(a - 0.06, z + 0.06, SILL - 0.05, SILL, WT + 0.14, frame);
    const mid = (a + z) / 2;
    box(mid - 0.025, mid + 0.025, SILL, LINTEL, 0.05, frame);
    box(a, z, (SILL + LINTEL) / 2 + 0.12, (SILL + LINTEL) / 2 + 0.16, 0.05, frame);
    // The sky pane (uv-mapped quad; collected for a textured mesh).
    const plane = new PlaneGeometry(z - a, LINTEL - SILL);
    const m = new Matrix4();
    if (horiz) m.makeTranslation(w.x0 + mid, (SILL + LINTEL) / 2, w.z0 + 0.01);
    else m.makeRotationY(Math.PI / 2).setPosition(w.x0 + 0.01, (SILL + LINTEL) / 2, w.z0 + mid);
    plane.applyMatrix4(m);
    this.windowPlanes.push(plane);
  }

  private windowPlanes: PlaneGeometry[] = [];

  private glassSpan(b: Builder, glass: Builder, w: Wall, a: number, z: number) {
    const horiz = w.z0 === w.z1;
    const col = c('#6e5a4a');
    const box = (u0: number, u1: number, y0: number, y1: number, t: number, target: Builder, cc: RGB) => {
      if (horiz) target.span(w.x0 + u0, y0, w.z0 - t / 2, w.x0 + u1, y1, w.z0 + t / 2, cc);
      else target.span(w.x0 - t / 2, y0, w.z0 + u0, w.x0 + t / 2, y1, w.z0 + u1, cc);
    };
    box(a, z, 0, 0.12, 0.1, b, c(MAT.wallCap));
    box(a, z, GLASS_H - 0.03, GLASS_H + 0.03, 0.08, b, col);
    const n = Math.max(1, Math.round((z - a) / 1.3));
    for (let i = 0; i <= n; i++) {
      const u = a + ((z - a) * i) / n;
      box(u - 0.025, u + 0.025, 0, GLASS_H, 0.07, b, col);
    }
    box(a, z, 0.12, GLASS_H - 0.03, 0.02, glass, [1, 1, 1]);
  }

  private buildFurniture() {
    const merged = new Builder();
    const screens = new Builder();
    const protos = new Map<string, Proto>();
    const hung = new Map<string, { b: Builder; nx: number; nz: number }>();
    for (const it of this.L.items) {
      // Drawn with a CC0 model instead (gfx/builtins.ts); the TV stand keeps its TV and console.
      if (modeled(it.type) && it.type !== 'tvStand') continue;
      const inst = INSTANCED[it.type];
      if (inst) {
        const key = `${it.type}:${it.variant ?? 0}`;
        let p = protos.get(key);
        if (!p) {
          const pb = new Builder();
          inst(pb, it.variant ?? 0);
          p = { key, geo: pb.build(), items: [] };
          protos.set(key, p);
        }
        p.items.push(it);
        continue;
      }
      const side = HUNG.has(it.type) ? this.sideOf(it.x, it.z) : null;
      if (side) {
        const key = `${side.nx},${side.nz}`;
        let hb = hung.get(key);
        if (!hb) hung.set(key, (hb = { b: new Builder(), ...side }));
        this.piece(hb.b, screens, it);
        continue;
      }
      this.piece(merged, screens, it);
    }
    merged.frame();
    this.mesh(merged.build(), this.matte, true, true);
    for (const h of hung.values()) {
      h.b.frame();
      this.hung.push({ obj: this.mesh(h.b.build(), this.matte, true, true), nx: h.nx, nz: h.nz });
    }
    // Static screens (the games TV): unlit so they read as glowing.
    this.mesh(screens.build(), this.screenMat, false, false);
    const m = new Matrix4();
    const q = new Quaternion();
    const one = new Vector3(1, 1, 1);
    const up = new Vector3(0, 1, 0);
    for (const p of protos.values()) {
      this.geos.push(p.geo);
      const im = new InstancedMesh(p.geo, this.matte, p.items.length);
      p.items.forEach((it, i) => {
        q.setFromAxisAngle(up, it.rot);
        im.setMatrixAt(i, m.compose(new Vector3(it.x, 0, it.z), q, one));
      });
      im.instanceMatrix.needsUpdate = true;
      im.castShadow = true;
      im.receiveShadow = true;
      im.computeBoundingSphere();
      this.group.add(im);
    }
  }

  /** One merged (not instanced) piece of furniture. */
  private piece(merged: Builder, screens: Builder, it: Item) {
    merged.frame(it.x, 0, it.z, it.rot);
    const seed = hash(`${it.type}:${it.x}:${it.z}`);
    switch (it.type) {
      case 'shelf':
        F.shelf(merged, it.w, seed);
        break;
      case 'armchair':
        F.armchair(merged, it.variant ?? 0);
        break;
      case 'sideTable':
        F.sideTable(merged);
        break;
      case 'floorLamp':
        F.floorLamp(merged);
        break;
      case 'readingTable':
        F.readingTable(merged);
        break;
      case 'meetingTable':
        F.meetingTable(merged);
        break;
      case 'whiteboard':
        F.whiteboardFrame(merged);
        break;
      case 'taskBoard':
        F.taskBoardFrame(merged);
        break;
      case 'testMonitor':
        F.testMonitorBody(merged);
        break;
      case 'serverMachine':
        F.serverMachine(merged);
        break;
      case 'pigeonholes':
        F.pigeonholes(merged, this.L.pigeonholes?.cols ?? 7, this.L.pigeonholes?.rows ?? 4);
        break;
      case 'mailbox':
        F.mailbox(merged);
        break;
      case 'sortingTable':
        F.sortingTable(merged);
        break;
      case 'catalog':
        F.catalog(merged);
        break;
      case 'counter':
        F.counter(merged);
        break;
      case 'fridge':
        F.fridge(merged);
        break;
      case 'coffee':
        F.coffee(merged);
        break;
      case 'roundTable':
        F.roundTable(merged);
        break;
      case 'stool':
        F.stool(merged);
        break;
      case 'sofa':
        F.sofa(merged);
        break;
      case 'couch':
        F.sofa(merged, true);
        break;
      case 'plant':
        F.plant(merged, it.variant ?? 0, seed);
        break;
      case 'tallPlant':
        F.tallPlant(merged, it.variant ?? 0, seed);
        break;
      case 'coatRack':
        F.coatRack(merged);
        break;
      case 'doormat':
        F.doormat(merged);
        break;
      case 'bench':
        F.bench(merged);
        break;
      case 'cooler':
        F.cooler(merged);
        break;
      case 'printer':
        F.printer(merged);
        break;
      case 'poster':
        F.poster(merged, it.variant ?? 0);
        break;
      case 'credenza':
        F.credenza(merged);
        break;
      case 'tvStand':
        screens.frame(it.x, 0, it.z, it.rot);
        F.tvStand(merged, screens, false);
        break;
      case 'foosball':
        F.foosball(merged);
        break;
      case 'poolTable':
        F.poolTable(merged);
        break;
      default:
        break;
    }
  }

  private buildLamps() {
    // Desk lamp shades: instanced, colored per desk (off / warm / amber).
    const lamps = this.L.items.filter((it) => it.type === 'lamp');
    const sb = new Builder();
    F.lampShade(sb);
    const shadeGeo = sb.build();
    this.geos.push(shadeGeo);
    const shades = new InstancedMesh(shadeGeo, this.deskShadeMat, lamps.length);
    const poolGeo = new PlaneGeometry(1.1, 1.1).rotateX(-Math.PI / 2);
    this.geos.push(poolGeo);
    const pools = new InstancedMesh(poolGeo, this.poolMat, lamps.length);
    const m = new Matrix4();
    lamps.forEach((it, i) => {
      // Shade hangs off the arm towards the desk center.
      const fx = Math.sin(it.rot);
      const fz = Math.cos(it.rot);
      m.makeTranslation(it.x + fx * 0.02, DESK_H + 0.3, it.z + fz * 0.02);
      shades.setMatrixAt(i, m);
      m.makeTranslation(it.x + fx * 0.1, DESK_H + 0.004, it.z + fz * 0.1);
      pools.setMatrixAt(i, m);
      if (it.desk != null) this.lampIndex.set(it.desk, i);
      this.lampState.push('off');
    });
    shades.instanceMatrix.needsUpdate = true;
    pools.instanceMatrix.needsUpdate = true;
    shades.castShadow = false;
    pools.renderOrder = 1;
    pools.frustumCulled = false;
    shades.computeBoundingSphere();
    this.group.add(shades, pools);
    this.deskShades = shades;
    this.deskPools = pools;

    // Floor lamps: glowing shades and pools of light.
    const fb = new Builder();
    for (const it of this.L.items) {
      if (it.type !== 'floorLamp') continue;
      fb.frame(it.x, 0, it.z, 0);
      fb.cyl(0, 1.3, 0, 0.24, 0.3, [1, 1, 1], 14, 0.16);
      this.pool(it.x, it.z, 3.2);
    }
    // Wall lights, each with its warm spill on the floor in front.
    for (const it of this.L.items) {
      if (it.type !== 'sconce') continue;
      fb.frame(it.x, it.y ?? 1.55, it.z, 0);
      fb.ball(0, 0, 0, 0.11, 0.13, 0.11, [1, 1, 1], 12, 8);
      this.pool(it.x + Math.sin(it.rot) * 1.06, it.z + Math.cos(it.rot) * 1.06, 3.4);
    }
    fb.frame();
    this.mesh(fb.build(), this.shadeMat, false, false);
  }

  private pool(x: number, z: number, size: number) {
    const g = new PlaneGeometry(size, size).rotateX(-Math.PI / 2).translate(x, 0.012, z);
    const mat = this.poolMat.clone();
    const mesh = this.mesh(g, mat, false, false);
    mesh.renderOrder = 1;
    this.staticPools.push(mesh);
  }

  /** Desk lamp state: off (nobody there), on (someone's working), amber (needs you). */
  setDeskLamp(desk: number, state: 'off' | 'on' | 'amber') {
    const i = this.lampIndex.get(desk);
    if (i == null || this.lampState[i] === state) return;
    this.lampState[i] = state;
    this.paintLamp(i);
  }

  private paintLamp(i: number) {
    const t = this.theme;
    if (!t || !this.deskShades || !this.deskPools) return;
    const st = this.lampState[i];
    const shadeOff = scale(lin(MAT.shade), t.name === 'dark' ? 0.35 : 0.85);
    const on: RGB = st === 'amber' ? scale(t.amber, t.name === 'dark' ? 1.4 : 1.1) : mix(lin(MAT.shade), t.lamp, 0.5).map((v) => v * (t.name === 'dark' ? 1.7 : 1.05)) as RGB;
    const sc = st === 'off' ? shadeOff : on;
    this.deskShades.setColorAt(i, new Color().setRGB(...sc));
    const k = st === 'off' ? 0 : t.name === 'dark' ? (st === 'amber' ? 0.9 : 0.6) : st === 'amber' ? 0.45 : 0;
    const pc = st === 'amber' ? t.amber : t.lamp;
    this.deskPools.setColorAt(i, new Color().setRGB(pc[0] * k, pc[1] * k, pc[2] * k));
    if (this.deskShades.instanceColor) this.deskShades.instanceColor.needsUpdate = true;
    if (this.deskPools.instanceColor) this.deskPools.instanceColor.needsUpdate = true;
  }

  applyTheme(t: OfficeTheme) {
    this.theme = t;
    // Window panes: one textured mesh per theme sky.
    const key = t.window.sky;
    if (!this.skies[key]) {
      const s = skyTexture(key);
      this.skies[key] = s;
      this.textures.push(s);
    }
    if (!this.windowMesh && this.windowPlanes.length) {
      const g = mergePlanes(this.windowPlanes);
      this.windowMesh = this.mesh(g, this.skyMat, false, false);
    }
    this.skyMat.map = this.skies[key]!;
    this.skyMat.color.setScalar(t.window.glow);
    this.skyMat.needsUpdate = true;
    this.shadeMat.emissive.setRGB(...t.lamp);
    this.shadeMat.emissiveIntensity = t.lampGlow;
    for (const p of this.staticPools) {
      const mat = p.material as MeshBasicMaterial;
      mat.color.setRGB(t.lamp[0] * t.pool, t.lamp[1] * t.pool, t.lamp[2] * t.pool);
      p.visible = t.pool > 0;
    }
    this.glassMat.opacity = t.name === 'dark' ? 0.1 : 0.16;
    for (let i = 0; i < this.lampState.length; i++) this.paintLamp(i);
    // Evening: a few warm point lights (no shadows) where the lamps are.
    if (t.points && !this.lights.length) {
      for (const [x, y, z, hex, inten, dist] of this.L.lights) {
        const l = new PointLight(new Color(hex), inten, dist, 1.4);
        l.position.set(x, y, z);
        this.lights.push(l);
        this.group.add(l);
      }
    }
    for (const l of this.lights) l.visible = !!t.points;
  }

  private windowMesh: Mesh | null = null;
  private extraMats: MeshBasicMaterial[] = [];

  dispose() {
    for (const g of this.geos) g.dispose();
    for (const t of this.textures) t.dispose();
    for (const p of this.windowPlanes) p.dispose();
    this.matte.dispose();
    this.floorMat.dispose();
    this.glassMat.dispose();
    this.skyMat.dispose();
    this.shadeMat.dispose();
    this.deskShadeMat.dispose();
    this.screenMat.dispose();
    this.poolMat.dispose();
    for (const p of this.staticPools) (p.material as MeshBasicMaterial).dispose();
    for (const m of this.extraMats) m.dispose();
    this.group.clear();
  }
}

function mergePlanes(planes: PlaneGeometry[]): BufferGeometry {
  let n = 0;
  for (const p of planes) n += p.index ? p.index.count : p.getAttribute('position').count;
  const pos = new Float32Array(n * 3);
  const uv = new Float32Array(n * 2);
  let k = 0;
  for (const p of planes) {
    const np = p.index ? p.toNonIndexed() : p;
    const P = np.getAttribute('position');
    const U = np.getAttribute('uv');
    for (let i = 0; i < P.count; i++, k++) {
      pos[k * 3] = P.getX(i);
      pos[k * 3 + 1] = P.getY(i);
      pos[k * 3 + 2] = P.getZ(i);
      uv[k * 2] = U.getX(i);
      uv[k * 2 + 1] = U.getY(i);
    }
    if (np !== p) np.dispose();
  }
  const g = new BufferGeometry();
  g.setAttribute('position', new BufferAttribute(pos, 3));
  g.setAttribute('uv', new BufferAttribute(uv, 2));
  g.computeVertexNormals();
  g.computeBoundingSphere();
  return g;
}
