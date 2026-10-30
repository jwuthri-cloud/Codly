// Build/Buy furniture: CC0 models (public/furniture, Kenney and KayKit) loaded once, scaled to
// the office, and the "ghost" of the piece being carried. Each model is normalized so its
// footprint is centered on the origin, it stands on y = 0 and faces +z like our furniture.

import { Box3, Color, DirectionalLight, Group, HemisphereLight, Mesh, MeshBasicMaterial, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene, Vector3, WebGLRenderer, type Object3D, type Raycaster } from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import type { Placed } from '../../lib/gen/Placed';
import { BY_ID, type Dims, type Furniture, type Piece } from '../buy';

/** Per pack: scale to office units (a desk top at ~0.56) and the turn that makes it face +z. */
const PACKS: Record<string, { scale: number; yaw: number }> = {
  kenney: { scale: 1.5, yaw: 0 },
  kaykit: { scale: 0.5, yaw: 0 },
};
const BASE = `${import.meta.env?.BASE_URL ?? '/'}furniture/`;

interface Model {
  root: Group;
  dims: Dims;
}

const loader = new GLTFLoader();
const models = new Map<string, Promise<Model>>();

export function model(f: Furniture): Promise<Model> {
  let m = models.get(f.id);
  if (!m) {
    m = loader.loadAsync(BASE + f.src).then((gltf) => {
      const pack = PACKS[f.src.split('/')[0]] ?? { scale: 1, yaw: 0 };
      const inner = gltf.scene;
      inner.scale.setScalar(pack.scale);
      inner.rotation.y = pack.yaw;
      const root = new Group();
      root.add(inner);
      const box = new Box3().setFromObject(root);
      const c = box.getCenter(new Vector3());
      inner.position.set(-c.x, -box.min.y + (f.rug ? 0.004 : 0), -c.z);
      const size = box.getSize(new Vector3());
      root.traverse((o) => {
        if (o instanceof Mesh) {
          o.castShadow = !f.rug;
          o.receiveShadow = true;
        }
      });
      return { root, dims: { w: size.x, d: size.z, h: size.y } };
    });
    m.catch(() => models.delete(f.id));
    models.set(f.id, m);
  }
  return m;
}

/** A piece to show: `scale` stretches the model (the built-in furniture is fitted to its spot),
 *  `tint` recolors its fabric (Kenney's `carpet` materials). */
export type Shown = Placed & { scale?: [number, number, number]; tint?: string };

const FABRIC = new Set(['carpet', 'carpetDarker']);
const tinted = new Map<string, MeshStandardMaterial>();
/** The model with its fabric in `tint` (materials shared per color). */
function recolor(root: Object3D, tint: string) {
  root.traverse((o) => {
    if (!(o instanceof Mesh)) return;
    const m = o.material as MeshStandardMaterial;
    if (!FABRIC.has(m.name)) return;
    const key = `${m.name}:${tint}`;
    let t = tinted.get(key);
    if (!t) {
      t = m.clone();
      t.color.set(tint);
      if (m.name === 'carpetDarker') t.color.multiplyScalar(0.7);
      tinted.set(key, t);
    }
    o.material = t;
  });
}

/** The pieces in the office, as 3D objects (picked by `uid`). */
export class DecorLayer {
  readonly group = new Group();
  private objs = new Map<string, { item: string; obj: Object3D }>();
  private ghost: { item: string; obj: Object3D; mats: MeshStandardMaterial[]; pad: Mesh<PlaneGeometry, MeshBasicMaterial> } | null = null;
  private seq = 0;

  /** Show `placed` (loading what's new); resolves with each piece's size, or null if outdated. */
  async set(placed: Shown[]): Promise<Piece[] | null> {
    const seq = ++this.seq;
    const loaded = await Promise.all(
      placed.map(async (p) => {
        const f = BY_ID.get(p.item);
        return f ? { p, m: await model(f).catch(() => null) } : null;
      }),
    );
    if (seq !== this.seq) return null;
    const out: Piece[] = [];
    const keep = new Set<string>();
    for (const l of loaded) {
      if (!l?.m) continue;
      const { p, m } = l;
      let o = this.objs.get(p.uid);
      if (o && o.item !== p.item) {
        o.obj.removeFromParent();
        o = undefined;
      }
      if (!o) {
        o = { item: p.item, obj: m.root.clone(true) };
        if (p.tint) recolor(o.obj, p.tint);
        o.obj.userData.uid = p.uid;
        this.group.add(o.obj);
        this.objs.set(p.uid, o);
      }
      const [sx, sy, sz] = p.scale ?? [1, 1, 1];
      o.obj.position.set(p.x, p.y, p.z);
      o.obj.rotation.y = p.rot;
      o.obj.scale.set(sx, sy, sz);
      keep.add(p.uid);
      out.push({ uid: p.uid, item: p.item, x: p.x, y: p.y, z: p.z, rot: p.rot, dims: { w: m.dims.w * sx, d: m.dims.d * sz, h: m.dims.h * sy } });
    }
    for (const [uid, o] of this.objs)
      if (!keep.has(uid)) {
        o.obj.removeFromParent();
        this.objs.delete(uid);
      }
    return out;
  }

  /** The piece under a ray, if any. */
  pick(ray: Raycaster): string | null {
    for (const hit of ray.intersectObject(this.group, true)) {
      let o: Object3D | null = hit.object;
      while (o && o.userData.uid === undefined) o = o.parent;
      if (o?.visible) return o.userData.uid as string;
    }
    return null;
  }

  /** Hide a piece while it is carried around. */
  hide(uid: string | null) {
    for (const [id, o] of this.objs) o.obj.visible = id !== uid;
  }

  /** The see-through piece following the cursor: green where it can go, red where it can't. */
  async showGhost(item: string, x: number, y: number, z: number, rot: number, ok: boolean) {
    const f = BY_ID.get(item);
    if (!f) return;
    if (this.ghost?.item !== item) {
      const m = await model(f);
      this.hideGhost();
      const obj = m.root.clone(true);
      const mats: MeshStandardMaterial[] = [];
      obj.traverse((o) => {
        if (!(o instanceof Mesh)) return;
        const mat = (o.material as MeshStandardMaterial).clone();
        mat.transparent = true;
        mat.opacity = 0.7;
        mat.depthWrite = false;
        o.material = mat;
        o.castShadow = false;
        mats.push(mat);
      });
      // Its footprint on the floor, green or red.
      const pad = new Mesh(new PlaneGeometry(m.dims.w + 0.1, m.dims.d + 0.1), new MeshBasicMaterial({ transparent: true, opacity: 0.45, depthWrite: false }));
      pad.rotation.x = -Math.PI / 2;
      pad.position.y = 0.012;
      obj.add(pad);
      this.ghost = { item, obj, mats, pad };
      this.group.add(obj);
    }
    const g = this.ghost!;
    g.obj.position.set(x, y + 0.01, z);
    g.obj.rotation.y = rot;
    for (const m of g.mats) m.emissive.set(ok ? '#1f7a3c' : '#9a2020');
    g.pad.material.color.set(ok ? '#5fe08f' : '#ff5a4f');
    g.pad.position.y = 0.012 - y; // stays on the floor under a piece set on a table
  }

  hideGhost() {
    if (!this.ghost) return;
    this.ghost.obj.removeFromParent();
    for (const m of this.ghost.mats) m.dispose();
    this.ghost.pad.geometry.dispose();
    this.ghost.pad.material.dispose();
    this.ghost = null;
  }

  dispose() {
    this.seq++;
    this.hideGhost();
    this.group.clear();
    this.objs.clear();
  }
}

// ── Catalog thumbnails: one small offscreen renderer, each model drawn once. ──

let thumbs: { r: WebGLRenderer; scene: Scene; cam: PerspectiveCamera } | null = null;
const shots = new Map<string, Promise<string>>();

export function thumbnail(item: string): Promise<string> {
  let s = shots.get(item);
  if (!s) {
    const f = BY_ID.get(item);
    s = f ? model(f).then((m) => shoot(m)) : Promise.resolve('');
    shots.set(item, s);
  }
  return s;
}

function shoot(m: Model): string {
  if (!thumbs) {
    const r = new WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true });
    r.setSize(96, 96, false);
    const scene = new Scene();
    const sun = new DirectionalLight('#fff4e6', 2.2);
    sun.position.set(2, 4, 3);
    scene.add(new HemisphereLight('#ffffff', new Color('#8a7a6a'), 1.6), sun);
    thumbs = { r, scene, cam: new PerspectiveCamera(30, 1, 0.01, 50) };
  }
  const { r, scene, cam } = thumbs;
  const obj = m.root.clone(true);
  obj.rotation.y = -0.5;
  scene.add(obj);
  const rad = Math.max(0.2, Math.hypot(m.dims.w, m.dims.d, m.dims.h) / 2);
  const dist = rad / Math.sin((cam.fov * Math.PI) / 360);
  cam.position.set(0, m.dims.h / 2 + dist * 0.55, dist * 0.84);
  cam.lookAt(0, m.dims.h / 2, 0);
  r.render(scene, cam);
  scene.remove(obj);
  return r.domElement.toDataURL('image/png');
}
