// Kenney "Mini Characters" (CC0, public/characters): 12 rigged chibi people. Loaded once; each
// agent gets a clone of one, its shirt recolored to the agent's identity color, its 7 bones
// (root, torso, head, arms, legs) driven by our procedural motion (gfx/character.ts).

import { CanvasTexture, MeshStandardMaterial, SRGBColorSpace, Vector3, type Bone, type Group, type Mesh, type SkinnedMesh, type Texture } from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { clone } from 'three/examples/jsm/utils/SkeletonUtils.js';
import type { RGB } from '../palette';

const NAMES = ['male-a', 'male-b', 'male-c', 'male-d', 'male-e', 'male-f', 'female-a', 'female-b', 'female-c', 'female-d', 'female-e', 'female-f'];
export const KENNEY_COUNT = NAMES.length;
const BASE = `${import.meta.env?.BASE_URL ?? '/'}characters/`;
const SWATCH_W = 32;
const SWATCH_H = 128;

interface Template {
  scene: Group;
  /** The colormap texel the shirt is painted with (most torso-bound vertices sample it). */
  shirt: [number, number];
}

let templates: Template[] | null = null;
let glasses: Group | null = null;
let pixels: ImageData | null = null;
let loading: Promise<void> | null = null;

export function kenneyReady() {
  return templates != null;
}

/** Load the pack (once); resolves when characters can be built from it. */
export function loadKenney(): Promise<void> {
  if (typeof document === 'undefined') return Promise.resolve();
  loading ??= (async () => {
    const loader = new GLTFLoader();
    const [chars, specs] = await Promise.all([Promise.all(NAMES.map((n) => loader.loadAsync(`${BASE}character-${n}.glb`))), loader.loadAsync(`${BASE}aid-glasses.glb`)]);
    const map = ((chars[0].scene.getObjectByName('body-mesh') as Mesh).material as MeshStandardMaterial).map!;
    const img = map.image as CanvasImageSource & { width: number; height: number };
    const cv = document.createElement('canvas');
    cv.width = img.width;
    cv.height = img.height;
    const g = cv.getContext('2d', { willReadFrequently: true })!;
    g.drawImage(img, 0, 0);
    pixels = g.getImageData(0, 0, cv.width, cv.height);
    glasses = specs.scene;
    templates = chars.map((gl) => ({ scene: gl.scene, shirt: shirtTexel(gl.scene) }));
  })();
  loading.catch(() => (loading = null));
  return loading;
}

/** A texel in the swatch covering most of the torso's front, skipping the head's swatches
 *  (skin, hair) so a bare neck or a tank top's shoulders never count as the shirt. */
function shirtTexel(scene: Group): [number, number] {
  const px = pixels!;
  const cell = (u: number, v: number): [number, number, number] => {
    const x = Math.min(px.width - 1, Math.floor(u * px.width));
    const y = Math.min(px.height - 1, Math.floor(v * px.height));
    return [Math.floor(y / SWATCH_H) * 64 + Math.floor(x / SWATCH_W), x, y];
  };
  const headUv = (scene.getObjectByName('head-mesh') as Mesh).geometry.getAttribute('uv');
  const head = new Set<number>();
  for (let k = 0; k < headUv.count; k++) head.add(cell(headUv.getX(k), headUv.getY(k))[0]);
  const m = scene.getObjectByName('body-mesh') as SkinnedMesh;
  const pos = m.geometry.getAttribute('position');
  const uv = m.geometry.getAttribute('uv');
  const j = m.geometry.getAttribute('skinIndex');
  const w = m.geometry.getAttribute('skinWeight');
  const index = m.geometry.index!;
  const torso = m.skeleton.bones.findIndex((b) => b.name === 'torso');
  const [a, b, c] = [new Vector3(), new Vector3(), new Vector3()];
  const votes = new Map<number, { area: number; at: [number, number] }>();
  for (let t = 0; t < index.count; t += 3) {
    const v = [index.getX(t), index.getX(t + 1), index.getX(t + 2)];
    if (v.some((k) => j.getX(k) !== torso || w.getX(k) < 0.99)) continue;
    a.fromBufferAttribute(pos, v[0]);
    b.fromBufferAttribute(pos, v[1]).sub(a);
    c.fromBufferAttribute(pos, v[2]).sub(a);
    b.cross(c);
    const area = b.length();
    if (b.z < 0.5 * area) continue; // front-facing only
    const [key, x, y] = cell((uv.getX(v[0]) + uv.getX(v[1]) + uv.getX(v[2])) / 3, (uv.getY(v[0]) + uv.getY(v[1]) + uv.getY(v[2])) / 3);
    if (head.has(key)) continue;
    const vote = votes.get(key) ?? { area: 0, at: [x, y] as [number, number] };
    vote.area += area;
    votes.set(key, vote);
  }
  let best: [number, number] = [0, 0];
  let most = -1;
  for (const v of votes.values()) if (v.area > most) [most, best] = [v.area, v.at];
  return best;
}

const maps = new Map<string, Texture>();
/** The colormap with template `i`'s shirt color swapped for `shirt` (linear RGB). */
function tinted(i: number, shirt: RGB, base: Texture): Texture {
  const key = `${i}:${shirt.join(',')}`;
  let t = maps.get(key);
  if (t) return t;
  const px = pixels!;
  // The colormap is a grid of 32×128 swatches (flat or a gradient): repaint the shirt's whole
  // swatch in `shirt`, keeping its shading relative to the swatch's middle.
  const [sx, sy] = templates![i].shirt;
  const x0 = sx - (sx % SWATCH_W);
  const y0 = sy - (sy % SWATCH_H);
  const lum = (k: number) => 0.3 * px.data[k] + 0.59 * px.data[k + 1] + 0.11 * px.data[k + 2] + 1;
  const mid = lum(((y0 + SWATCH_H / 2) * px.width + x0 + SWATCH_W / 2) * 4);
  const to = shirt.map((c) => 255 * (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055));
  const out = new ImageData(new Uint8ClampedArray(px.data), px.width, px.height);
  const d = out.data;
  for (let y = y0; y < y0 + SWATCH_H; y++)
    for (let x = x0; x < x0 + SWATCH_W; x++) {
      const k = (y * px.width + x) * 4;
      const f = lum(k) / mid;
      d[k] = to[0] * f;
      d[k + 1] = to[1] * f;
      d[k + 2] = to[2] * f;
    }
  const cv = document.createElement('canvas');
  cv.width = px.width;
  cv.height = px.height;
  cv.getContext('2d')!.putImageData(out, 0, 0);
  t = new CanvasTexture(cv);
  t.colorSpace = SRGBColorSpace;
  t.flipY = base.flipY;
  t.magFilter = base.magFilter;
  t.minFilter = base.minFilter;
  maps.set(key, t);
  return t;
}

export interface KenneyBody {
  scene: Group;
  mat: MeshStandardMaterial;
  bones: Record<'root' | 'torso' | 'head' | 'arm-left' | 'arm-right' | 'leg-left' | 'leg-right', Bone>;
}

/** A posable clone of model `i` (mod 12) in `shirt`, optionally with glasses on. */
export function kenneyBody(i: number, shirt: RGB, specs: boolean): KenneyBody {
  i = ((i % KENNEY_COUNT) + KENNEY_COUNT) % KENNEY_COUNT;
  const scene = clone(templates![i].scene) as Group;
  const src = (scene.getObjectByName('body-mesh') as Mesh).material as MeshStandardMaterial;
  const mat = src.clone();
  mat.map = tinted(i, shirt, src.map!);
  scene.traverse((o) => {
    if ((o as Mesh).isMesh) {
      (o as Mesh).material = mat;
      o.castShadow = true;
    }
  });
  const bone = (n: string) => scene.getObjectByName(n) as Bone;
  const bones = { root: bone('root'), torso: bone('torso'), head: bone('head'), 'arm-left': bone('arm-left'), 'arm-right': bone('arm-right'), 'leg-left': bone('leg-left'), 'leg-right': bone('leg-right') };
  if (specs && glasses) {
    const g = glasses.clone();
    g.traverse((o) => {
      if ((o as Mesh).isMesh) (o as Mesh).material = mat;
    });
    // The pack places them on a standing character; carry them on the head bone instead.
    g.position.set(0, 0.105, 0.09);
    bones.head.add(g);
  }
  return { scene, mat, bones };
}
