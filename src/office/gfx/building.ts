// The static studio: a floating floor slab, the floor atlas, cutaway walls (full-height back
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
  MeshStandardMaterial,
  PlaneGeometry,
  PointLight,
  Quaternion,
  Vector3,
  type Texture,
} from 'three';
import { D, DESK_H, GLASS_H, INNER_H, LOW_H, W, WALL_H, type Item, type ItemType, type OfficeLayout, type Wall } from '../layout';
import { color, lin, MAT, mix, scale, type OfficeTheme, type RGB } from '../palette';
import { hash } from '../text';
import { Builder } from './builder';
import { modeled } from './builtins';
import * as F from './furniture';
import { c } from './furniture';
import { floorTexture, glowTexture, skyTexture } from './textures';

const WT = 0.16;
const SILL = 0.78;
const LINTEL = 1.88;

type Proto = { key: string; geo: BufferGeometry; items: Item[] };

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
  private skies: Partial<Record<'light' | 'dark', Texture>> = {};
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
  private wallsUp = true;
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
    // A floating slab with a little porch at the entrance.
    b.span(-0.3, -0.55, -0.3, W + 0.3, -0.005, D + 0.3, slab);
    b.span(-0.36, -0.6, -0.36, W + 0.36, -0.5, D + 0.36, mix(slab, [0, 0, 0], 0.25));
    const dx = this.L.door.x;
    b.span(dx - 1.3, -0.3, D + 0.3, dx + 1.3, -0.02, D + 1.1, c('#d9cfbf'));
    b.span(dx - 1.1, -0.55, D + 1.1, dx + 1.1, -0.32, D + 1.5, c('#cfc4b2'));
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
      if (w.kind === 'glass') {
        for (const [a, z] of spans) this.glassSpan(b, glass, w, a, z);
        // Door posts.
        for (const [a, z] of gaps) {
          seg(a - 0.04, a + 0.04, 0, GLASS_H + 0.06, c('#5c4a3d'), 0.1);
          seg(z - 0.04, z + 0.04, 0, GLASS_H + 0.06, c('#5c4a3d'), 0.1);
          seg(a, z, GLASS_H - 0.02, GLASS_H + 0.06, c('#5c4a3d'), 0.1);
        }
        continue;
      }
      const H = w.kind === 'exterior' ? WALL_H : w.kind === 'interior' ? INNER_H : LOW_H;
      const windows = w.windows ?? [];
      for (const [a, z] of spans) {
        // Split around windows.
        let cur = a;
        for (const [wa, wz] of windows) {
          if (wz <= cur || wa >= z) continue;
          seg(cur, wa, 0, H, plaster);
          seg(wa, wz, 0, SILL, plaster);
          seg(wa, wz, LINTEL, H, plaster);
          this.window(b, w, wa, wz, frame);
          cur = wz;
        }
        seg(cur, z, 0, H, plaster);
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
      if (side) {
        const mid = horiz ? [0, w.z0 < D / 2 ? -1 : 1] : [w.x0 < W / 2 ? -1 : 1, 0];
        this.sides.push({ mesh: this.mesh(b.build(), this.matte, true, true), nx: mid[0], nz: mid[1] });
      }
    }
    const b = walls;
    // The entrance threshold and frame.
    b.frame(this.L.door.x, 0, D, 0);
    F.doorFrame(b);
    b.frame();
    const wm = this.mesh(b.build(), this.matte, true, true);
    const gm = this.mesh(glass.build(), this.glassMat, false, false);
    gm.renderOrder = 2;
    this.wallMeshes = [wm, gm];
  }

  /** Walls up, or down to knee-high stubs (The Sims' "walls down") to see into every room. */
  setWalls(up: boolean) {
    this.wallsUp = up;
    this.applyWalls();
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

  private lowered(s: { nx: number; nz: number }) {
    return s.nx * this.camDir[0] + s.nz * this.camDir[1] > 0.25;
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
      this.piece(merged, screens, it);
    }
    merged.frame();
    this.mesh(merged.build(), this.matte, true, true);
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
      case 'pigeonholes':
        F.pigeonholes(merged, this.L.pigeonholes.cols, this.L.pigeonholes.rows);
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
    // Wall sconces between the Leads' windows and in the lounge; a pendant over the meeting table.
    for (const [x, y, z] of [
      [20.0, 1.55, 0.14],
      [23.25, 1.55, 0.14],
      [0.14, 1.55, 17.6],
    ]) {
      fb.frame(x, y, z, 0);
      fb.ball(0, 0, 0, 0.11, 0.13, 0.11, [1, 1, 1], 12, 8);
    }
    fb.frame();
    this.mesh(fb.build(), this.shadeMat, false, false);
    // The sconces' warm spill on the Leads' floor.
    for (const [x, z, s] of [
      [20.0, 1.2, 3.4],
      [23.25, 1.2, 3.4],
    ] as const)
      this.pool(x, z, s);
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
    const key = t.name;
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
      for (const [x, y, z, hex, inten, dist] of [
        [21.5, 1.7, 3.2, '#ffc58a', 5, 9],
        [1.6, 1.6, 4.8, '#ffc07a', 3.5, 7],
        [4.2, 1.7, 20.2, '#ffc58a', 3.5, 7.5],
        [7.4, 1.9, 11.75, '#ffcf96', 3.5, 8],
        [14.2, 1.9, 11.75, '#ffcf96', 3.5, 8],
        [24.8, 1.5, 11.6, '#8fb8ff', 2.6, 6.5],
        [16.5, 2.4, 20.0, '#ffc58a', 1.8, 7],
        [9.3, 2.1, 8.6, '#ffd7a0', 2.4, 5.5],
        [11, 1.8, 3.6, '#ffd7a0', 3, 7],
        [24.7, 1.6, 20.4, '#ffc58a', 2.4, 6],
      ] as const) {
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
