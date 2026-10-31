// Character lab (character-lab.html): a lineup of agents for working on how people look.
//   ?view=close|office|side|back   camera (office = the game's high angle)
//   ?pose=stand|walk|sit|type|…    everyone in one pose     ?emote=check|zz|rain|…
import { AmbientLight, CanvasTexture, Color, CircleGeometry, DirectionalLight, HemisphereLight, Mesh, MeshStandardMaterial, NeutralToneMapping, PCFShadowMap, PerspectiveCamera, Scene, WebGLRenderer } from 'three';
import type { Emote, Pose } from '../behavior';
import { Character, lookFor, PropKit } from '../gfx/character';
import type { Provider } from '../../lib/gen/Provider';
import type { Role } from '../../lib/gen/Role';

const params = new URLSearchParams(location.search);
const view = params.get('view') ?? 'close';
const pose = (params.get('pose') ?? 'stand') as Pose | 'walk';

const ALL: [string, number, Role, Provider, boolean?][] = [
  ['ada', 0, 'lead', 'claude'],
  ['bo', 1, 'reviewer', 'codex'],
  ['cy', 2, 'worker', 'gemini'],
  ['dee', 3, 'worker', 'claude'],
  ['eli', 4, 'worker', 'cursor'],
  ['fay', 5, 'solo', 'opencode'],
  ['gus', 6, 'worker', 'claude'],
  ['hal', 7, 'worker', 'codex'],
  ['ivy', 2, 'worker', 'claude', true],
];
// ?only=0,3 picks from the cast; ?style=N gives everyone hairstyle N (a pick, like the look editor).
const only = params.get('only')?.split(',').map(Number);
// ?styles=a-b: one person per hairstyle a … b.
const styles = params.get('styles')?.split('-').map(Number);
const CAST = styles ? Array.from({ length: styles[1] - styles[0] + 1 }, (_, i) => ALL[i % 4 === 3 ? 6 : 3 + (i % 3)].slice() as (typeof ALL)[number]) : only ? ALL.filter((_, i) => only.includes(i)) : ALL;
const style = params.get('style');

const renderer = new WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
renderer.setPixelRatio(Math.min(2, devicePixelRatio));
renderer.toneMapping = NeutralToneMapping;
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = PCFShadowMap;
document.getElementById('lab')!.appendChild(renderer.domElement);

const scene = new Scene();
scene.background = new Color(0xe9e1d4);
scene.add(new HemisphereLight(0xfff6ea, 0x9a8a78, 2.1), new AmbientLight(0xffffff, 0.35));
const key = new DirectionalLight(0xfff0dc, 2.6);
key.position.set(-3, 6, 5);
key.castShadow = true;
key.shadow.mapSize.set(2048, 2048);
Object.assign(key.shadow.camera, { left: -8, right: 8, top: 4, bottom: -4 });
const rim = new DirectionalLight(0xffe2c0, 1.2);
rim.position.set(4, 3, -3);
scene.add(key, rim);
const floor = new Mesh(new CircleGeometry(30, 48), new MeshStandardMaterial({ color: 0xd8cbb6, roughness: 0.9 }));
floor.rotation.x = -Math.PI / 2;
floor.receiveShadow = true;
scene.add(floor);

const blank = new CanvasTexture(document.createElement('canvas'));
const props = new PropKit();
const gap = 1.05;
const people = CAST.map(([id, color, role, provider, intern], i) => {
  const st = styles ? String(styles[0] + i) : style;
  const c = new Character(id + i, lookFor(st ? `${id}#style=${st}&acc=none` : id, color, role, provider, intern), { emotes: () => blank, props });
  c.teleport({ x: (i - (CAST.length - 1) / 2) * gap, z: 0, face: view === 'side' ? Math.PI / 2 : view === 'back' ? Math.PI : 0 }, 'k', 0, false);
  scene.add(c.root);
  return c;
});

const cam = new PerspectiveCamera(view === 'office' ? 30 : 24, 1, 0.1, 200);
function frame() {
  const w = innerWidth;
  const h = innerHeight;
  renderer.setSize(w, h);
  cam.aspect = w / h;
  // Fit the lineup's width.
  const half = (CAST.length * gap) / 2 + 0.2;
  const dist = half / Math.tan(((cam.fov / 2) * Math.PI) / 180) / cam.aspect;
  if (view === 'office') cam.position.set(0, dist * 0.62, dist * 0.78);
  else cam.position.set(0, 1.0, Math.max(dist, CAST.length === 1 ? 2.3 : 2.6));
  cam.lookAt(0, view === 'office' ? 0.5 : CAST.length === 1 ? 0.75 : CAST.length < 4 ? 0.95 : 0.85, 0);
  cam.updateProjectionMatrix();
}
frame();
addEventListener('resize', frame);

const emote = params.get('emote') as Emote;
let last = performance.now();
const start = last;
function tick(now: number) {
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;
  for (const c of people) {
    if (pose === 'walk') {
      if (!c.walking) c.walk([[c.root.position.x, 40]], { x: c.root.position.x, z: 41, face: 0 }, 'w', now);
      c.root.rotation.y = 0;
      c.root.position.z = 0;
    } else c.setPose(pose, now, pose === 'sit' || pose === 'type' ? 0.45 : undefined);
    if (emote && now - start > 50) c.setEmote(emote, now);
    c.update(now, dt, false, true);
  }
  renderer.render(scene, cam);
  requestAnimationFrame(tick);
}
requestAnimationFrame(tick);

const ui = document.getElementById('ui')!;
for (const v of ['close', 'office', 'side', 'back'])
  for (const p of v === 'close' ? ['stand', 'walk', 'type', 'think', 'celebrate'] : ['stand']) {
    const a = document.createElement('a');
    a.href = `?view=${v}&pose=${p}`;
    a.textContent = `${v}${p === 'stand' ? '' : ' · ' + p}`;
    ui.appendChild(a);
  }
