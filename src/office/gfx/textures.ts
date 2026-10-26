// Procedural textures (no image assets): the floor (one atlas for the whole plan: oak planks,
// walnut herringbone, felt carpet, tiles, terrazzo, rugs and soft contact shading along the
// walls), the sky behind the windows (day / dusk with a little skyline), soft glows, and the
// emote sprites ("!", "?", zZ, thought cloud, check).

import { CanvasTexture, LinearMipmapLinearFilter, SRGBColorSpace, type Texture } from 'three';
import { D, W, wallPieces, type OfficeLayout, type Rect, type Room } from '../layout';
import { MAT, type ThemeName } from '../palette';
import { rng } from '../text';

export function canvas(w: number, h = w): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  return [c, c.getContext('2d')!];
}

export function tex(c: HTMLCanvasElement, srgb = true): CanvasTexture {
  const t = new CanvasTexture(c);
  if (srgb) t.colorSpace = SRGBColorSpace;
  t.anisotropy = 4;
  t.needsUpdate = true;
  return t;
}

const PX = 64; // floor texture pixels per unit

function shade(hex: string, k: number): string {
  const n = parseInt(hex.slice(1), 16);
  const f = (v: number) => Math.max(0, Math.min(255, Math.round(v * k)));
  return `rgb(${f((n >> 16) & 255)},${f((n >> 8) & 255)},${f(n & 255)})`;
}

function planks(g: CanvasRenderingContext2D, r: Rect, base: string, seed: number, w = 0.22) {
  const rand = rng(seed);
  const x0 = r.x * PX;
  const y0 = r.z * PX;
  const x1 = (r.x + r.w) * PX;
  const y1 = (r.z + r.d) * PX;
  const pw = w * PX;
  for (let y = y0, row = 0; y < y1; y += pw, row++) {
    let x = x0 - rand() * 1.6 * PX;
    while (x < x1) {
      const len = (1.1 + rand() * 1.5) * PX;
      g.fillStyle = shade(base, 0.92 + rand() * 0.14);
      g.fillRect(Math.max(x0, x), y, Math.min(x1, x + len) - Math.max(x0, x), Math.min(pw, y1 - y));
      // Grain streaks.
      g.fillStyle = 'rgba(80,50,20,0.05)';
      for (let k = 0; k < 3; k++) g.fillRect(Math.max(x0, x + rand() * len * 0.7), y + rand() * pw, len * 0.3, 1);
      g.fillStyle = 'rgba(60,35,15,0.28)';
      if (x > x0) g.fillRect(x, y, 1.2, pw);
      x += len;
    }
    g.fillStyle = 'rgba(60,35,15,0.22)';
    g.fillRect(x0, y, x1 - x0, 1);
  }
}

function herringbone(g: CanvasRenderingContext2D, r: Rect, base: string, seed: number) {
  const rand = rng(seed);
  g.save();
  g.beginPath();
  g.rect(r.x * PX, r.z * PX, r.w * PX, r.d * PX);
  g.clip();
  const L = 0.6 * PX;
  const Wd = 0.15 * PX;
  for (let y = r.z * PX - L; y < (r.z + r.d) * PX + L; y += Wd * 2) {
    for (let x = r.x * PX - L, k = 0; x < (r.x + r.w) * PX + L; x += L, k++) {
      for (const flip of [0, 1]) {
        g.save();
        g.translate(x + (flip ? L / 2 : 0), y + (flip ? Wd : 0));
        g.rotate(flip ? -Math.PI / 4 : Math.PI / 4);
        g.fillStyle = shade(base, 0.95 + rand() * 0.08);
        g.fillRect(0, 0, L * 0.7, Wd);
        g.strokeStyle = 'rgba(50,28,12,0.14)';
        g.lineWidth = 1;
        g.strokeRect(0, 0, L * 0.7, Wd);
        g.restore();
      }
    }
  }
  g.restore();
}

function felt(g: CanvasRenderingContext2D, r: Rect, base: string, seed: number) {
  const rand = rng(seed);
  g.fillStyle = base;
  g.fillRect(r.x * PX, r.z * PX, r.w * PX, r.d * PX);
  const n = Math.round(r.w * r.d * 180);
  for (let i = 0; i < n; i++) {
    g.fillStyle = rand() < 0.5 ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.05)';
    g.fillRect((r.x + rand() * r.w) * PX, (r.z + rand() * r.d) * PX, 1.5, 1.5);
  }
}

function tiles(g: CanvasRenderingContext2D, r: Rect, a: string, b: string, size: number, grout: string, seed: number, checker: boolean) {
  const rand = rng(seed);
  const s = size * PX;
  for (let y = r.z * PX, j = 0; y < (r.z + r.d) * PX - 0.5; y += s, j++) {
    for (let x = r.x * PX, i = 0; x < (r.x + r.w) * PX - 0.5; x += s, i++) {
      const col = checker && (i + j) % 2 ? b : a;
      g.fillStyle = shade(col, 0.96 + rand() * 0.07);
      g.fillRect(x, y, Math.min(s, (r.x + r.w) * PX - x), Math.min(s, (r.z + r.d) * PX - y));
      g.fillStyle = grout;
      g.fillRect(x, y, Math.min(s, (r.x + r.w) * PX - x), 1.5);
      g.fillRect(x, y, 1.5, Math.min(s, (r.z + r.d) * PX - y));
    }
  }
}

function terrazzo(g: CanvasRenderingContext2D, r: Rect, seed: number) {
  const rand = rng(seed);
  g.fillStyle = MAT.terrazzo;
  g.fillRect(r.x * PX, r.z * PX, r.w * PX, r.d * PX);
  const chips = ['#ddd2c2', '#c9d0cb', '#e3c3b2', '#c4beb6', '#f6f2ea'];
  const n = Math.round(r.w * r.d * 55);
  for (let i = 0; i < n; i++) {
    g.fillStyle = chips[Math.floor(rand() * chips.length)];
    const x = (r.x + rand() * r.w) * PX;
    const y = (r.z + rand() * r.d) * PX;
    const s = 1 + rand() * 2.2;
    g.beginPath();
    g.ellipse(x, y, s, s * (0.5 + rand() * 0.5), rand() * 3, 0, Math.PI * 2);
    g.fill();
  }
}

function rug(g: CanvasRenderingContext2D, cx: number, cz: number, w: number, d: number, variant: number) {
  const x = (cx - w / 2) * PX;
  const y = (cz - d / 2) * PX;
  const ww = w * PX;
  const hh = d * PX;
  const [base, border, accent] = [
    ['#3f6f73', '#d9c7a7', '#e0a458'],
    ['#8f6f63', '#eadcc4', '#5f4a3e'],
    ['#d4a85a', '#7d5a3c', '#f3e6cc'],
  ][variant % 3];
  g.save();
  g.shadowColor = 'rgba(0,0,0,0.25)';
  g.shadowBlur = 6;
  g.fillStyle = base;
  roundRect(g, x, y, ww, hh, 10);
  g.fill();
  g.restore();
  g.strokeStyle = border;
  g.lineWidth = 5;
  roundRect(g, x + 10, y + 10, ww - 20, hh - 20, 6);
  g.stroke();
  g.strokeStyle = accent;
  g.lineWidth = 2;
  roundRect(g, x + 20, y + 20, ww - 40, hh - 40, 4);
  g.stroke();
  // A simple repeating motif.
  g.fillStyle = accent;
  const step = 22;
  for (let px = x + 34; px < x + ww - 30; px += step)
    for (let py = y + 34; py < y + hh - 30; py += step) {
      if (variant === 1 && ((px - x) / step + (py - y) / step) % 2 < 1) continue;
      g.beginPath();
      g.moveTo(px, py - 4);
      g.lineTo(px + 4, py);
      g.lineTo(px, py + 4);
      g.lineTo(px - 4, py);
      g.closePath();
      g.globalAlpha = 0.55;
      g.fill();
      g.globalAlpha = 1;
    }
  // Fringes on the short sides.
  g.strokeStyle = border;
  g.lineWidth = 1.5;
  for (let py = y + 4; py < y + hh - 4; py += 4) {
    g.beginPath();
    g.moveTo(x, py);
    g.lineTo(x - 5, py);
    g.moveTo(x + ww, py);
    g.lineTo(x + ww + 5, py);
    g.stroke();
  }
}

export function roundRect(g: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  g.beginPath();
  g.moveTo(x + r, y);
  g.arcTo(x + w, y, x + w, y + h, r);
  g.arcTo(x + w, y + h, x, y + h, r);
  g.arcTo(x, y + h, x, y, r);
  g.arcTo(x, y, x + w, y, r);
  g.closePath();
}

function roomFloor(g: CanvasRenderingContext2D, room: Room, i: number) {
  const r = room.rect;
  switch (room.floor) {
    case 'oak':
      planks(g, r, MAT.oak, 11 + i);
      break;
    case 'walnut':
      herringbone(g, r, MAT.walnut, 23 + i);
      break;
    case 'walnut-planks':
      planks(g, r, MAT.walnutDark, 29 + i, 0.3);
      break;
    case 'carpet-sage':
      felt(g, r, MAT.carpetSage, 31 + i);
      break;
    case 'carpet-blue':
      felt(g, r, MAT.carpetBlue, 37 + i);
      break;
    case 'tile-cool':
      tiles(g, r, MAT.tileCool, MAT.tileCool, 0.5, 'rgba(120,130,140,0.45)', 41 + i, false);
      break;
    case 'tile-warm':
      tiles(g, r, MAT.tileWarm, MAT.tileTerracotta, 0.4, 'rgba(160,120,90,0.35)', 43 + i, true);
      break;
    case 'terrazzo':
      terrazzo(g, r, 47 + i);
      break;
  }
}

/** The whole floor: every room's material, rugs, and soft shading along the walls. */
export function floorTexture(L: OfficeLayout): CanvasTexture {
  const [c, g] = canvas(W * PX, D * PX);
  L.rooms.forEach((room, i) => roomFloor(g, room, i));
  for (const it of L.items) if (it.type === 'rug') rug(g, it.x, it.z, it.w, it.d, it.variant ?? 0);
  // Contact shading: walls and tall furniture darken the floor right next to them.
  // (Blurred via an off-canvas shape's shadow: canvas `filter` is missing in older WebKit.)
  const OFF = 100_000;
  g.save();
  g.shadowOffsetX = OFF;
  g.shadowColor = 'rgba(40,25,10,0.3)';
  g.shadowBlur = 16;
  g.fillStyle = '#000';
  for (const w of L.walls) {
    if (w.kind === 'low') continue;
    for (const p of wallPieces(w)) g.fillRect(p.x * PX - 4 - OFF, p.z * PX - 4, p.w * PX + 8, p.d * PX + 8);
  }
  g.shadowColor = 'rgba(40,25,10,0.2)';
  g.shadowBlur = 10;
  for (const it of L.items) {
    if (!it.block || it.type === 'rug') continue;
    const q = Math.abs(Math.round(it.rot / (Math.PI / 2))) % 2 === 1;
    const w = q ? it.d : it.w;
    const d = q ? it.w : it.d;
    g.fillRect((it.x - w / 2) * PX - 1 - OFF, (it.z - d / 2) * PX - 1, w * PX + 2, d * PX + 2);
  }
  g.restore();
  const t = tex(c);
  t.minFilter = LinearMipmapLinearFilter;
  t.anisotropy = 8;
  return t;
}

/** What you see through the windows: daylight with clouds, or dusk over a little skyline. */
export function skyTexture(theme: ThemeName): CanvasTexture {
  const [c, g] = canvas(512, 256);
  const night = theme === 'dark';
  const grad = g.createLinearGradient(0, 0, 0, 256);
  if (night) {
    grad.addColorStop(0, '#171a3f');
    grad.addColorStop(0.5, '#3b2f62');
    grad.addColorStop(0.82, '#c4706a');
    grad.addColorStop(1, '#f0a26a');
  } else {
    grad.addColorStop(0, '#9fcbe8');
    grad.addColorStop(0.7, '#d6ecf5');
    grad.addColorStop(1, '#f4f6ef');
  }
  g.fillStyle = grad;
  g.fillRect(0, 0, 512, 256);
  const r = rng(7);
  if (night) {
    for (let i = 0; i < 70; i++) {
      g.fillStyle = `rgba(255,255,240,${0.3 + r() * 0.6})`;
      g.fillRect(r() * 512, r() * 140, 1.4, 1.4);
    }
    g.fillStyle = 'rgba(255,240,210,0.9)';
    g.beginPath();
    g.arc(410, 52, 11, 0, Math.PI * 2);
    g.fill();
    g.fillStyle = '#3b2f62';
    g.beginPath();
    g.arc(415, 48, 10, 0, Math.PI * 2);
    g.fill();
  } else {
    g.fillStyle = 'rgba(255,255,255,0.85)';
    for (const [x, y, s] of [
      [90, 60, 1],
      [330, 40, 1.3],
      [460, 95, 0.8],
    ]) {
      for (let k = 0; k < 5; k++) {
        g.beginPath();
        g.ellipse(x + (k - 2) * 16 * s, y + Math.abs(k - 2) * 3 * s, 20 * s, 12 * s, 0, 0, Math.PI * 2);
        g.fill();
      }
    }
  }
  // A skyline across the bottom.
  let x = 0;
  while (x < 512) {
    const w = 18 + r() * 34;
    const h = 30 + r() * 70;
    g.fillStyle = night ? '#1a1830' : '#b9c9d3';
    g.fillRect(x, 256 - h, w, h);
    if (night) {
      for (let wy = 256 - h + 6; wy < 250; wy += 9)
        for (let wx = x + 4; wx < x + w - 4; wx += 7) if (r() < 0.32) {
          g.fillStyle = r() < 0.8 ? 'rgba(255,200,120,0.85)' : 'rgba(180,210,255,0.7)';
          g.fillRect(wx, wy, 3, 4);
        }
    } else {
      g.fillStyle = 'rgba(255,255,255,0.35)';
      for (let wy = 256 - h + 6; wy < 250; wy += 9) for (let wx = x + 4; wx < x + w - 4; wx += 7) if (r() < 0.4) g.fillRect(wx, wy, 3, 4);
    }
    x += w + 2;
  }
  return tex(c);
}

/** A soft radial glow (light pools, halos). */
export function glowTexture(): CanvasTexture {
  const [c, g] = canvas(128);
  const grad = g.createRadialGradient(64, 64, 0, 64, 64, 64);
  grad.addColorStop(0, 'rgba(255,255,255,1)');
  grad.addColorStop(0.35, 'rgba(255,255,255,0.45)');
  grad.addColorStop(1, 'rgba(255,255,255,0)');
  g.fillStyle = grad;
  g.fillRect(0, 0, 128, 128);
  return tex(c, false);
}

export type EmoteKind = 'alert' | 'question' | 'zz' | 'think1' | 'think2' | 'think3' | 'check' | 'hello' | 'spark';

/** Emote sprites, drawn once. */
export function emoteTexture(kind: EmoteKind): Texture {
  const [c, g] = canvas(128);
  g.lineJoin = 'round';
  const bubble = (fill: string, stroke: string) => {
    g.fillStyle = fill;
    g.strokeStyle = stroke;
    g.lineWidth = 5;
    roundRect(g, 18, 12, 92, 86, 30);
    g.fill();
    g.stroke();
    g.beginPath();
    g.moveTo(52, 96);
    g.lineTo(64, 118);
    g.lineTo(76, 96);
    g.closePath();
    g.fill();
    g.beginPath();
    g.moveTo(52, 98);
    g.lineTo(64, 118);
    g.lineTo(76, 98);
    g.stroke();
    g.fillRect(50, 88, 28, 10);
  };
  const glyph = (t: string, color: string, size = 64, y = 58) => {
    g.fillStyle = color;
    g.font = `800 ${size}px Inter, 'Inter Variable', system-ui, sans-serif`;
    g.textAlign = 'center';
    g.textBaseline = 'middle';
    g.fillText(t, 64, y);
  };
  switch (kind) {
    case 'alert':
      bubble('#ffb547', '#8a4b00');
      glyph('!', '#3a1f00', 70);
      break;
    case 'question':
      bubble('#ffb547', '#8a4b00');
      glyph('?', '#3a1f00', 64);
      break;
    case 'check':
      g.fillStyle = '#3fbf74';
      g.strokeStyle = '#1d6b3f';
      g.lineWidth = 5;
      g.beginPath();
      g.arc(64, 64, 40, 0, Math.PI * 2);
      g.fill();
      g.stroke();
      g.strokeStyle = '#ffffff';
      g.lineWidth = 11;
      g.lineCap = 'round';
      g.beginPath();
      g.moveTo(45, 66);
      g.lineTo(59, 80);
      g.lineTo(84, 50);
      g.stroke();
      break;
    case 'zz':
      g.font = `800 52px Inter, 'Inter Variable', system-ui, sans-serif`;
      g.lineWidth = 7;
      g.strokeStyle = '#2b2350';
      g.fillStyle = '#e6e0ff';
      g.textAlign = 'center';
      g.strokeText('z', 44, 84);
      g.fillText('z', 44, 84);
      g.font = `800 40px Inter, 'Inter Variable', system-ui, sans-serif`;
      g.strokeText('z', 80, 50);
      g.fillText('z', 80, 50);
      break;
    case 'think1':
    case 'think2':
    case 'think3': {
      const n = +kind.slice(-1);
      g.fillStyle = '#ffffff';
      g.strokeStyle = '#5b5f6e';
      g.lineWidth = 4;
      for (const [x, y, r] of [
        [40, 58, 24],
        [64, 46, 28],
        [88, 58, 24],
        [64, 70, 26],
      ]) {
        g.beginPath();
        g.arc(x, y, r, 0, Math.PI * 2);
        g.stroke();
      }
      for (const [x, y, r] of [
        [40, 58, 24],
        [64, 46, 28],
        [88, 58, 24],
        [64, 70, 26],
      ]) {
        g.beginPath();
        g.arc(x, y, r - 2, 0, Math.PI * 2);
        g.fill();
      }
      g.beginPath();
      g.arc(42, 102, 8, 0, Math.PI * 2);
      g.fill();
      g.stroke();
      g.beginPath();
      g.arc(30, 118, 5, 0, Math.PI * 2);
      g.fill();
      g.stroke();
      g.fillStyle = '#4b4f5e';
      for (let i = 0; i < n; i++) {
        g.beginPath();
        g.arc(46 + i * 18, 60, 6, 0, Math.PI * 2);
        g.fill();
      }
      break;
    }
    case 'hello':
      bubble('#ffffff', '#5b5f6e');
      glyph('hi', '#3b3f4e', 46, 56);
      break;
    case 'spark': {
      const grad = g.createRadialGradient(64, 64, 0, 64, 64, 60);
      grad.addColorStop(0, 'rgba(255,255,255,1)');
      grad.addColorStop(0.2, 'rgba(255,255,255,0.4)');
      grad.addColorStop(1, 'rgba(255,255,255,0)');
      g.fillStyle = grad;
      g.fillRect(0, 0, 128, 128);
      g.fillStyle = 'rgba(255,255,255,0.95)';
      g.beginPath();
      g.moveTo(64, 6);
      g.quadraticCurveTo(68, 60, 122, 64);
      g.quadraticCurveTo(68, 68, 64, 122);
      g.quadraticCurveTo(60, 68, 6, 64);
      g.quadraticCurveTo(60, 60, 64, 6);
      g.fill();
      break;
    }
  }
  return tex(c);
}

/** A thin soft ring (arrival rings, selection). */
export function ringTexture(): CanvasTexture {
  const [c, g] = canvas(128);
  const grad = g.createRadialGradient(64, 64, 30, 64, 64, 64);
  grad.addColorStop(0, 'rgba(255,255,255,0)');
  grad.addColorStop(0.62, 'rgba(255,255,255,0.05)');
  grad.addColorStop(0.82, 'rgba(255,255,255,1)');
  grad.addColorStop(1, 'rgba(255,255,255,0)');
  g.fillStyle = grad;
  g.fillRect(0, 0, 128, 128);
  return tex(c, false);
}
