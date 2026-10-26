// DOM overlay: name tags (identity dot, name, an activity icon), the agent's one-line sentence
// (when selected, hovered, when it needs you, or briefly after an outcome), room names and
// small tags (test results). Positioned from the camera on rendered frames; crisp (integer
// pixels); sanitized (textContent only, icons are our own static SVG); decluttered: labels
// are placed by priority, and a label that would overlap a more important one collapses to
// its name, stacks above, or hides (agents never hide unless under the app's UI).

import { Vector3, type PerspectiveCamera } from 'three';
import type { Tone } from './model';

export interface LabelSpec {
  key: string;
  kind: 'agent' | 'room' | 'tag';
  world: Vector3;
  /** Pixel offset of the label's anchor (bottom-center) from the projected point. */
  dy: number;
  priority: number;
  title: string;
  pre?: string;
  code?: string | null;
  post?: string;
  tone?: Tone;
  color?: string;
  badge?: string | null;
  icon?: string | null;
  selected?: boolean;
  hovered?: boolean;
  /** Prefer the compact form (name + icon). */
  compact?: boolean;
  /** Keep visible even under the app's top UI (clamped below it). */
  urgent?: boolean;
  agentId?: string;
  taskId?: string;
  dim?: boolean;
}

interface El {
  root: HTMLDivElement;
  title: HTMLSpanElement;
  dot: HTMLSpanElement | null;
  icon: HTMLSpanElement | null;
  badge: HTMLSpanElement | null;
  line: HTMLDivElement | null;
  pre: Text | null;
  code: HTMLElement | null;
  post: Text | null;
  sig: string;
  iconKey: string;
  w: number;
  wc: number;
  h: number;
  hc: number;
  measured: boolean;
  shown: boolean;
  compact: boolean;
  x: number;
  y: number;
  used: boolean;
}

const v = new Vector3();
const SVG = 'http://www.w3.org/2000/svg';

/** Activity icons: 16×16, stroke paths (ours, static). */
const ICONS: Record<string, string> = {
  read: 'M2 3.5h4.5a1.5 1.5 0 0 1 1.5 1.5v8a1.5 1.5 0 0 0-1.5-1.5H2zM14 3.5H9.5A1.5 1.5 0 0 0 8 5v8a1.5 1.5 0 0 1 1.5-1.5H14z',
  search: 'M7 2.5a4.5 4.5 0 1 1 0 9a4.5 4.5 0 0 1 0-9zM10.4 10.4L14 14',
  edit: 'M10.5 2.5l3 3L6 13H3v-3zM9 4l3 3',
  create: 'M8 2.5v11M2.5 8h11',
  delete: 'M3 4.5h10M6 4.5V3h4v1.5M4.5 4.5l.7 9h5.6l.7-9',
  execute: 'M2.5 3.5h11v9h-11zM4.5 6.5l2 1.5-2 1.5M8 10h3',
  test: 'M6 2.5h4M6.8 2.5v4L3.5 12.5a1 1 0 0 0 .9 1.5h7.2a1 1 0 0 0 .9-1.5L9.2 6.5v-4',
  'tests-pass': 'M3 8.5l3 3 7-7',
  'tests-fail': 'M4 4l8 8M12 4l-8 8',
  fetch: 'M8 2.5a5.5 5.5 0 1 1 0 11a5.5 5.5 0 0 1 0-11zM2.5 8h11M8 2.5c2 2.2 2 8.8 0 11M8 2.5c-2 2.2-2 8.8 0 11',
  mcp: 'M4.5 2.5h2l1 3-1.5 1a7 7 0 0 0 3.5 3.5l1-1.5 3 1v2a1.5 1.5 0 0 1-1.5 1.5A10 10 0 0 1 3 4a1.5 1.5 0 0 1 1.5-1.5z',
  plan: 'M5 3.5h6M4 2.5h8v11H4zM6 7h4M6 9.5h4',
  note: 'M3 3h10v6.5L9.5 13H3zM9.5 13V9.5H13',
  memory: 'M2.5 4h11v3h-11zM3.5 7v6h9V7M6.5 9.5h3',
  message: 'M2.5 8l11-5-4 11-2-4.5zM7.5 9.5L13.5 3',
  review: 'M1.5 8s2.5-4.5 6.5-4.5S14.5 8 14.5 8s-2.5 4.5-6.5 4.5S1.5 8 1.5 8zM8 6a2 2 0 1 1 0 4a2 2 0 0 1 0-4z',
  commit: 'M2.5 4h11v8h-11zM2.5 4l5.5 4.5L13.5 4',
  merge: 'M5 2.5v11M5 6c0 3 6 2 6 5.5M11 9.5v4',
  card: 'M3 3.5h10v9H3zM5 6h6M5 8.5h4',
  delegate: 'M5.5 7a2 2 0 1 0 0-4a2 2 0 0 0 0 4zM2 13c0-2 1.5-3.5 3.5-3.5S9 11 9 13M11.5 6.5v4M9.5 8.5h4',
  approval: 'M8 2.5v7M8 12.5v.5',
  question: 'M6 5.5a2 2 0 1 1 2.8 1.8c-.5.3-.8.7-.8 1.2v.8M8 12v.5',
  blocked: 'M4 10.5a3 3 0 0 1 .5-6 3.5 3.5 0 0 1 6.8.5A2.5 2.5 0 0 1 11.5 10.5zM6 12.5l-.5 1.5M9 12.5l-.5 1.5',
  rate: 'M2.5 5h9v6h-9zM11.5 7h1.5v2h-1.5',
  sleep: 'M4 4h4l-4 4h4M9 8h3l-3 3h3',
  thinking: 'M4 8h.5M8 8h.5M12 8h.5',
  replying: 'M2.5 3.5h11v7H7l-3 2.5v-2.5H2.5z',
  retry: 'M12.5 8a4.5 4.5 0 1 1-1.3-3.2M12.5 3v2.5H10',
  compacting: 'M3 4h10M4.5 8h7M6 12h4',
  done: 'M3 8.5l3 3 7-7',
  working: 'M3 12.5h10M4 10l2-5 2 3 2-4 2 6',
};

function iconEl(key: string): SVGSVGElement | null {
  const d = ICONS[key];
  if (!d) return null;
  const s = document.createElementNS(SVG, 'svg');
  s.setAttribute('viewBox', '0 0 16 16');
  s.setAttribute('width', '13');
  s.setAttribute('height', '13');
  s.setAttribute('aria-hidden', 'true');
  const p = document.createElementNS(SVG, 'path');
  p.setAttribute('d', d);
  p.setAttribute('fill', 'none');
  p.setAttribute('stroke', 'currentColor');
  p.setAttribute('stroke-width', key === 'approval' || key === 'question' || key === 'thinking' ? '2.4' : '1.5');
  p.setAttribute('stroke-linecap', 'round');
  p.setAttribute('stroke-linejoin', 'round');
  s.appendChild(p);
  return s;
}

export class Overlay {
  readonly root: HTMLDivElement;
  private els = new Map<string, El>();

  constructor(parent: HTMLElement) {
    this.root = document.createElement('div');
    this.root.className = 'of-overlay';
    this.root.setAttribute('aria-hidden', 'true');
    parent.appendChild(this.root);
  }

  private make(spec: LabelSpec): El {
    const root = document.createElement('div');
    root.className = `of-label of-${spec.kind}`;
    const head = document.createElement('div');
    head.className = 'of-head';
    let dot: HTMLSpanElement | null = null;
    let icon: HTMLSpanElement | null = null;
    if (spec.kind === 'agent') {
      dot = document.createElement('span');
      dot.className = 'of-dot';
      head.appendChild(dot);
    }
    const title = document.createElement('span');
    title.className = 'of-title';
    head.appendChild(title);
    let badge: HTMLSpanElement | null = null;
    if (spec.kind === 'agent') {
      badge = document.createElement('span');
      badge.className = 'of-badge';
      head.appendChild(badge);
      icon = document.createElement('span');
      icon.className = 'of-icon';
      head.appendChild(icon);
    }
    root.appendChild(head);
    let line: HTMLDivElement | null = null;
    let pre: Text | null = null;
    let code: HTMLElement | null = null;
    let post: Text | null = null;
    if (spec.kind === 'agent') {
      line = document.createElement('div');
      line.className = 'of-line';
      pre = document.createTextNode('');
      code = document.createElement('code');
      post = document.createTextNode('');
      line.append(pre, code, post);
      root.appendChild(line);
    }
    if (spec.agentId) root.dataset.agent = spec.agentId;
    if (spec.taskId) root.dataset.task = spec.taskId;
    this.root.appendChild(root);
    return { root, title, dot, icon, badge, line, pre, code, post, sig: '', iconKey: '', w: 0, wc: 0, h: 0, hc: 0, measured: false, shown: false, compact: false, x: -1e6, y: -1e6, used: true };
  }

  private fill(e: El, s: LabelSpec) {
    const sig = [s.title, s.pre, s.code, s.post, s.tone, s.color, s.badge, s.selected, s.hovered, s.icon, s.dim].join('\u0001');
    if (sig === e.sig) return;
    e.sig = sig;
    e.title.textContent = s.title;
    if (e.dot && s.color) e.dot.style.background = s.color;
    if (e.badge) {
      e.badge.textContent = s.badge ?? '';
      e.badge.style.display = s.badge ? '' : 'none';
    }
    if (e.icon && (s.icon ?? '') !== e.iconKey) {
      e.iconKey = s.icon ?? '';
      e.icon.replaceChildren();
      const svg = s.icon ? iconEl(s.icon) : null;
      if (svg) e.icon.appendChild(svg);
      e.icon.style.display = svg ? '' : 'none';
      e.icon.dataset.icon = e.iconKey;
    }
    if (e.pre && e.code && e.post) {
      const pre = s.pre ?? '';
      e.pre.textContent = s.code ? pre + ' ' : pre;
      e.code.textContent = s.code ?? '';
      e.code.style.display = s.code ? '' : 'none';
      e.post.textContent = s.post ?? '';
    }
    e.root.dataset.tone = s.tone ?? 'normal';
    e.root.classList.toggle('of-selected', !!s.selected);
    e.root.classList.toggle('of-hovered', !!s.hovered);
    e.root.classList.toggle('of-dim', !!s.dim);
    if (s.color) e.root.style.setProperty('--of-id', s.color);
    e.measured = false;
  }

  /** Lay out all labels for the current camera. */
  update(specs: LabelSpec[], camera: PerspectiveCamera, W: number, H: number, insets = { top: 0, right: 0, bottom: 0, left: 0 }) {
    for (const e of this.els.values()) e.used = false;
    const items: { s: LabelSpec; e: El; x: number; y: number; depth: number }[] = [];
    for (const s of specs) {
      let e = this.els.get(s.key);
      if (!e) {
        e = this.make(s);
        this.els.set(s.key, e);
      }
      e.used = true;
      this.fill(e, s);
      v.copy(s.world).project(camera);
      const visible = v.z < 1 && v.z > -1 && Math.abs(v.x) < 1.25 && Math.abs(v.y) < 1.25;
      if (!visible) {
        this.hide(e);
        continue;
      }
      items.push({ s, e, x: (v.x * 0.5 + 0.5) * W, y: (-v.y * 0.5 + 0.5) * H + s.dy, depth: v.z });
    }
    // Measure (batched reads after writes).
    for (const it of items) {
      const e = it.e;
      if (e.measured) continue;
      e.root.classList.remove('of-compact');
      e.root.style.display = '';
      e.w = e.root.offsetWidth;
      e.h = e.root.offsetHeight;
      if (e.line) {
        e.root.classList.add('of-compact');
        e.wc = e.root.offsetWidth;
        e.hc = e.root.offsetHeight;
        e.root.classList.toggle('of-compact', e.compact);
      } else {
        e.wc = e.w;
        e.hc = e.h;
      }
      e.measured = true;
      if (!e.shown) e.root.style.display = 'none';
    }
    items.sort((a, b) => b.s.priority - a.s.priority || a.depth - b.depth);
    const placed: [number, number, number, number][] = [];
    if (insets.top > 0) placed.push([-1e5, -1e5, 1e5, insets.top - 4]);
    if (insets.bottom > 0) placed.push([-1e5, H - insets.bottom + 4, 1e5, 1e5]);
    if (insets.left > 0) placed.push([-1e5, -1e5, insets.left - 4, 1e5]);
    if (insets.right > 0) placed.push([W - insets.right + 4, -1e5, 1e5, 1e5]);
    const pad = 3;
    const free = (x0: number, y0: number, x1: number, y1: number) => {
      for (const [a, b, c, d] of placed) if (x0 < c + pad && x1 > a - pad && y0 < d + pad && y1 > b - pad) return false;
      return true;
    };
    for (const it of items) {
      const e = it.e;
      const full = it.s.selected || it.s.hovered;
      let compact = !!it.s.compact && !full && !!e.line;
      let w = compact ? e.wc : e.w;
      let h = compact ? e.hc : e.h;
      let x0 = it.x - w / 2;
      let y0 = it.y - h;
      let ok = x0 > -w && x0 < W && y0 > -h && y0 < H && free(x0, y0, x0 + w, y0 + h);
      if (!ok && e.line && !full) {
        w = e.wc;
        h = e.hc;
        x0 = it.x - w / 2;
        y0 = it.y - h;
        compact = true;
        ok = x0 > -w && x0 < W && free(x0, y0, x0 + w, y0 + h);
      }
      if (!ok && it.s.kind !== 'agent') {
        this.hide(e);
        continue;
      }
      if (!ok) {
        // Agents never disappear: stack the compact tag above whatever is in the way.
        if (!full) {
          compact = true;
          w = e.wc;
          h = e.hc;
          x0 = it.x - w / 2;
          y0 = it.y - h;
        }
        for (let k = 1; k <= 4 && !ok; k++) {
          const y = y0 - k * (h + 4);
          if (y >= insets.top && free(x0, y, x0 + w, y + h)) {
            y0 = y;
            ok = true;
          }
        }
        if (!ok && y0 < insets.top) {
          if (!it.s.urgent) {
            this.hide(e);
            continue;
          }
          y0 = insets.top;
        }
        e.root.classList.toggle('of-crowded', !ok);
      } else e.root.classList.remove('of-crowded');
      placed.push([x0, y0, x0 + w, y0 + h]);
      if (compact !== e.compact) {
        e.compact = compact;
        e.root.classList.toggle('of-compact', compact);
      }
      const rx = Math.round(x0);
      const ry = Math.round(y0);
      if (!e.shown) {
        e.root.style.display = '';
        e.shown = true;
      }
      if (rx !== e.x || ry !== e.y) {
        e.x = rx;
        e.y = ry;
        e.root.style.transform = `translate3d(${rx}px, ${ry}px, 0)`;
      }
      e.root.style.zIndex = String(Math.round(it.s.priority));
    }
    for (const [k, e] of this.els) {
      if (!e.used) {
        e.root.remove();
        this.els.delete(k);
      }
    }
  }

  private hide(e: El) {
    if (e.shown) {
      e.root.style.display = 'none';
      e.shown = false;
    }
  }

  dispose() {
    this.root.remove();
    this.els.clear();
  }
}
