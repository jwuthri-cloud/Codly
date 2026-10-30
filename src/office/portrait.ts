// CONTRACT (owned by the 3D office engine): a small portrait of an agent's character —
// head and shoulders, exactly as it looks in the office (the same Kenney character from its seed,
// in its identity color) — for the crew strip, chat and sheets. Synchronous: rendered with
// a tiny shared WebGL renderer (released a few seconds after the last call, so it never holds
// a GL context for long), composited on a soft identity-colored disc, cached.

import { AmbientLight, DirectionalLight, HemisphereLight, NeutralToneMapping, PerspectiveCamera, Scene, Texture, WebGLRenderer } from 'three';
import type { Agent } from '../lib/gen/Agent';
import { Character, lookFor, PropKit } from './gfx/character';
import { kenneyReady, loadKenney } from './gfx/kenney';
import { identity, type ThemeName } from './palette';

const cache = new Map<string, string>();
let renderer: WebGLRenderer | null = null;
let failed = false;
let props: PropKit | null = null;
let releaseTimer: ReturnType<typeof setTimeout> | undefined;
const blank = new Texture();

function currentTheme(): ThemeName {
  if (typeof document === 'undefined') return 'dark';
  const t = document.documentElement.dataset.theme;
  if (t === 'light' || t === 'dark') return t;
  return typeof matchMedia !== 'undefined' && matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

function release() {
  renderer?.dispose();
  renderer?.forceContextLoss();
  renderer = null;
  props?.dispose();
  props = null;
}

function gl(size: number): WebGLRenderer | null {
  if (failed) return null;
  if (!renderer) {
    try {
      renderer = new WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true, powerPreference: 'low-power' });
      renderer.toneMapping = NeutralToneMapping;
      renderer.setClearColor(0x000000, 0);
    } catch {
      failed = true;
      return null;
    }
  }
  renderer.setPixelRatio(1);
  renderer.setSize(size, size, false);
  clearTimeout(releaseTimer);
  releaseTimer = setTimeout(release, 4000);
  return renderer;
}

/** PNG data URL of the agent's character at `px` CSS pixels (rendered at device pixel ratio);
 *  '' until the Kenney characters have loaded (`loadKenney()` resolves). */
export function agentPortrait(agent: Pick<Agent, 'id' | 'color' | 'look_seed' | 'provider' | 'role'>, px: number): string {
  if (typeof document === 'undefined') return '';
  if (!kenneyReady()) {
    void loadKenney();
    return '';
  }
  const dpr = Math.min(3, Math.max(1, typeof devicePixelRatio === 'number' ? devicePixelRatio : 1));
  const theme = currentTheme();
  const size = Math.max(16, Math.round(px * dpr));
  const key = `${agent.look_seed || agent.id}|${agent.color}|${agent.provider}|${agent.role}|${size}|${theme}`;
  const hit = cache.get(key);
  if (hit != null) return hit;
  const r = gl(size * 2);
  if (!r) return '';
  props ??= new PropKit();
  const look = lookFor(agent.look_seed || agent.id, agent.color, agent.role, agent.provider);
  const c = new Character(agent.id, look, { emotes: () => blank, props });
  c.root.rotation.y = -0.32;
  c.update(0, 0, true);
  const scene = new Scene();
  scene.add(new HemisphereLight(0xfff6ea, 0x9a8a78, 2.1), new AmbientLight(0xffffff, 0.35));
  const key1 = new DirectionalLight(0xfff0dc, 2.6);
  key1.position.set(-1.2, 2.2, 3);
  const rim = new DirectionalLight(0xffe2c0, 1.2);
  rim.position.set(2, 1.5, -1.5);
  scene.add(key1, rim, c.root);
  // Head and shoulders: aim between the chin and the crown.
  const aim = 0.66 * look.scale;
  const cam = new PerspectiveCamera(26, 1, 0.1, 20);
  cam.position.set(0.1, aim + 0.28, 2.3 * (look.scale / 1.2));
  cam.lookAt(0, aim + 0.02, 0);
  r.render(scene, cam);
  const out = document.createElement('canvas');
  out.width = out.height = size;
  const g = out.getContext('2d');
  if (!g) {
    c.dispose();
    return '';
  }
  // A soft disc in the agent's identity color behind the head.
  const id = identity(agent.color, theme);
  const grad = g.createRadialGradient(size / 2, size * 0.42, size * 0.05, size / 2, size / 2, size / 2);
  grad.addColorStop(0, id.soft + 'cc');
  grad.addColorStop(1, id.soft + '55');
  g.fillStyle = grad;
  g.beginPath();
  g.arc(size / 2, size / 2, size / 2, 0, Math.PI * 2);
  g.fill();
  g.save();
  g.beginPath();
  g.arc(size / 2, size / 2, size / 2, 0, Math.PI * 2);
  g.clip();
  g.drawImage(r.domElement, 0, 0, size, size);
  g.restore();
  const url = out.toDataURL('image/png');
  c.dispose();
  if (cache.size > 300) cache.clear();
  cache.set(key, url);
  return url;
}
