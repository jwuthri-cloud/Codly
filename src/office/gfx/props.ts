// The office's live surfaces — each one shows real state, nothing invented:
//  • desk monitors: code lines in the agent's color while it edits, a document while it reads,
//    a terminal while it runs commands, an amber dialog while it waits for you;
//  • the task board: one card per real task, in To do / Doing / Review / Done; cards travel
//    (to a desk with their worker, back to Review, on to Done);
//  • the whiteboard: agents' todo lists and the team's notes as sticky notes;
//  • the test monitor (last real result, green or red, with counts) and rack LEDs that blink
//    only while a command or test is running;
//  • the mail room (an envelope per commit, a gold seal when merged) and the memory archive
//    (a drawer label lights per memory; the card catalog per knowledge fact);
//  • framed artifacts on desks (image artifacts show the real image).

import {
  BoxGeometry,
  CanvasTexture,
  Color,
  Euler,
  Group,
  InstancedMesh,
  Matrix4,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  Object3D,
  PlaneGeometry,
  Quaternion,
  SRGBColorSpace,
  Texture,
  TextureLoader,
  Vector3,
} from 'three';
import type { Artifact } from '../../lib/gen/Artifact';
import type { Screen } from '../behavior';
import { DESK_H, type Desk, type OfficeLayout } from '../layout';
import type { Column, Note, TaskCard } from '../model';
import { identity, lin, type OfficeTheme, type ThemeName } from '../palette';
import { clean, hash, rng } from '../text';
import { canvas, roundRect, tex } from './textures';

const FONT = `'Inter Variable', Inter, -apple-system, system-ui, sans-serif`;
const MONO = `'JetBrains Mono Variable', 'SF Mono', Menlo, monospace`;

function wrap(g: CanvasRenderingContext2D, text: string, maxW: number, maxLines: number): string[] {
  const words = text.split(' ');
  const lines: string[] = [];
  let cur = '';
  for (const w of words) {
    const t = cur ? cur + ' ' + w : w;
    if (g.measureText(t).width <= maxW) cur = t;
    else {
      if (cur) lines.push(cur);
      cur = w;
      if (lines.length >= maxLines) break;
    }
  }
  if (cur && lines.length < maxLines) lines.push(cur);
  if (lines.length === maxLines && words.join(' ') !== lines.join(' ')) {
    let last = lines[maxLines - 1];
    while (last.length > 1 && g.measureText(last + '…').width > maxW) last = last.slice(0, -1);
    lines[maxLines - 1] = last + '…';
  }
  return lines;
}

// ───────────── desk monitors ─────────────

interface ScreenState {
  mesh: Mesh;
  tex: CanvasTexture | null;
  ctx: CanvasRenderingContext2D | null;
  mode: Screen;
  color: number;
  label: string;
  step: number;
  sig: string;
}

const ANIMATED: Partial<Record<Screen, number>> = { code: 300, terminal: 450, chat: 450, search: 600, doc: 900 };

export class DeskScreens {
  readonly group = new Group();
  private states: ScreenState[] = [];
  private offMat = new MeshBasicMaterial({ color: 0x0d0f13 });
  private geo = new PlaneGeometry(0.52, 0.31);
  private glow = 1;

  constructor(desks: Desk[]) {
    for (const d of desks) {
      const m = new Mesh(this.geo, this.offMat);
      m.position.set(d.screen[0] - Math.sin(d.rot) * 0.017, d.screen[1] + (d.lead ? 0.02 : 0), d.screen[2] - Math.cos(d.rot) * 0.017);
      m.rotation.y = d.rot + Math.PI;
      this.group.add(m);
      this.states.push({ mesh: m, tex: null, ctx: null, mode: 'off', color: 0, label: '', step: -1, sig: '' });
    }
  }

  applyTheme(t: OfficeTheme) {
    this.glow = t.screenGlow;
    for (const s of this.states) if (s.mesh.material !== this.offMat) (s.mesh.material as MeshBasicMaterial).color.setScalar(this.glow);
  }

  set(desk: number, mode: Screen, color: number, label: string) {
    const s = this.states[desk];
    if (!s) return;
    s.mode = mode;
    s.color = color;
    s.label = label;
  }

  /** Repaint what changed; returns when an animated screen next needs a frame. */
  update(now: number, reducedMotion: boolean): number {
    let next = Infinity;
    for (const s of this.states) {
      if (s.mode === 'off') {
        if (s.mesh.material !== this.offMat) s.mesh.material = this.offMat;
        s.sig = 'off';
        continue;
      }
      const per = ANIMATED[s.mode];
      const step = per && !reducedMotion ? Math.floor(now / per) : 0;
      if (per && !reducedMotion) next = Math.min(next, (step + 1) * per);
      const sig = `${s.mode}|${s.color}|${s.label}|${step}`;
      if (sig === s.sig) continue;
      s.sig = sig;
      if (!s.ctx) {
        const [c, g] = canvas(192, 112);
        s.ctx = g;
        s.tex = tex(c);
        s.mesh.material = new MeshBasicMaterial({ map: s.tex, toneMapped: false });
        (s.mesh.material as MeshBasicMaterial).color.setScalar(this.glow);
      } else if (s.mesh.material === this.offMat) {
        s.mesh.material = new MeshBasicMaterial({ map: s.tex!, toneMapped: false });
        (s.mesh.material as MeshBasicMaterial).color.setScalar(this.glow);
      }
      paintScreen(s.ctx, s.mode, s.color, s.label, step);
      s.tex!.needsUpdate = true;
    }
    return next;
  }

  dispose() {
    for (const s of this.states) {
      s.tex?.dispose();
      if (s.mesh.material !== this.offMat) (s.mesh.material as MeshBasicMaterial).dispose();
    }
    this.offMat.dispose();
    this.geo.dispose();
  }
}

function paintScreen(g: CanvasRenderingContext2D, mode: Screen, color: number, label: string, step: number) {
  const W = 192;
  const H = 112;
  const id = identity(color, 'dark');
  const ink = id.screen;
  const r = rng(hash(label) + color);
  g.fillStyle = '#12151c';
  g.fillRect(0, 0, W, H);
  const lines = (n: number, x0: number, y0: number, lh: number, colors: string[], scroll: number) => {
    for (let i = 0; i < n; i++) {
      const k = i + scroll;
      const rr = rng(hash(label) + k * 31);
      const indent = (rr() * 3) | 0;
      const w = 30 + rr() * 90;
      g.fillStyle = colors[k % colors.length];
      g.globalAlpha = 0.85;
      g.fillRect(x0 + indent * 10, y0 + i * lh, w, 4);
      if (rr() < 0.5) {
        g.fillStyle = colors[(k + 1) % colors.length];
        g.fillRect(x0 + indent * 10 + w + 6, y0 + i * lh, 20 + rr() * 30, 4);
      }
      g.globalAlpha = 1;
    }
  };
  switch (mode) {
    case 'idle': {
      const grad = g.createLinearGradient(0, 0, W, H);
      grad.addColorStop(0, '#1d2333');
      grad.addColorStop(1, id.screen + '55');
      g.fillStyle = grad;
      g.fillRect(0, 0, W, H);
      g.fillStyle = 'rgba(255,255,255,0.75)';
      g.font = `600 15px ${FONT}`;
      g.textAlign = 'center';
      g.fillText(label || '', W / 2, H / 2 + 5);
      g.textAlign = 'left';
      break;
    }
    case 'code': {
      g.fillStyle = '#1a1e28';
      g.fillRect(0, 0, 22, H);
      g.fillStyle = 'rgba(255,255,255,0.18)';
      for (let i = 0; i < 9; i++) g.fillRect(6, 8 + i * 11, 8, 3);
      lines(9, 30, 8, 11, [ink, '#9aa6b8', '#e6c07b', ink, '#c3cad6', '#7fc7d9'], step % 40);
      g.fillStyle = ink;
      g.fillRect(30 + (step % 7) * 12, 8 + 8 * 11 - 2, 2, 8);
      break;
    }
    case 'doc': {
      g.fillStyle = '#f3efe6';
      g.fillRect(26, 0, W - 52, H);
      g.fillStyle = '#3b3f4a';
      g.fillRect(36, 10, 70, 6);
      for (let i = 0; i < 8; i++) {
        g.fillStyle = i === (step % 8) ? ink : '#9aa0aa';
        g.fillRect(36, 26 + i * 10, 60 + r() * 60, 3);
      }
      break;
    }
    case 'search': {
      lines(8, 10, 10, 12, ['#9aa6b8', '#c3cad6'], 0);
      g.fillStyle = ink;
      g.globalAlpha = 0.35;
      g.fillRect(4, 6 + (step % 8) * 12, W - 8, 12);
      g.globalAlpha = 1;
      g.strokeStyle = ink;
      g.lineWidth = 3;
      g.beginPath();
      g.arc(W - 36, H - 34, 12, 0, Math.PI * 2);
      g.moveTo(W - 27, H - 25);
      g.lineTo(W - 16, H - 14);
      g.stroke();
      break;
    }
    case 'terminal': {
      g.fillStyle = '#0b0d10';
      g.fillRect(0, 0, W, H);
      g.font = `600 10px ${MONO}`;
      const rows = 8;
      for (let i = 0; i < rows; i++) {
        const k = i + step;
        const rr = rng(hash(label) + k * 17);
        g.fillStyle = i === rows - 1 ? ink : rr() < 0.2 ? '#e6c07b' : '#8fe3a8';
        g.globalAlpha = 0.85;
        g.fillRect(10, 8 + i * 12, 20 + rr() * 120, 4);
        g.globalAlpha = 1;
      }
      g.fillStyle = ink;
      g.fillText('$ ' + (label || '').slice(0, 26), 8, H - 6);
      break;
    }
    case 'chat': {
      for (let i = 0; i < 3; i++) {
        const mine = i % 2 === 1;
        g.fillStyle = mine ? ink : '#2b3242';
        const w = 70 + ((hash(label) >> (i * 3)) % 60);
        const y = 10 + i * 32;
        roundRect(g, mine ? W - w - 10 : 10, y, w, 22, 8);
        g.fill();
      }
      g.fillStyle = 'rgba(255,255,255,0.8)';
      g.fillRect(16, 96, 10 + (step % 6) * 14, 3);
      break;
    }
    case 'think': {
      g.fillStyle = '#1a1e28';
      g.fillRect(0, 0, W, H);
      lines(6, 16, 14, 13, ['#4a5262', '#3d4452'], 0);
      break;
    }
    case 'alert': {
      g.fillStyle = '#1a1e28';
      g.fillRect(0, 0, W, H);
      g.fillStyle = '#ffb547';
      roundRect(g, 24, 22, W - 48, H - 44, 10);
      g.fill();
      g.fillStyle = '#3a1f00';
      g.font = `800 30px ${FONT}`;
      g.textAlign = 'center';
      g.fillText('!', W / 2, H / 2 + 11);
      g.textAlign = 'left';
      break;
    }
    case 'error': {
      g.fillStyle = '#2a1416';
      g.fillRect(0, 0, W, H);
      g.fillStyle = '#ff6b5e';
      g.fillRect(0, 0, W, 16);
      lines(6, 12, 28, 12, ['#ff9c92', '#c3cad6'], 0);
      break;
    }
  }
}

// ───────────── task board ─────────────

const COLUMNS: Column[] = ['todo', 'doing', 'review', 'done'];
const COLUMN_NAMES: Record<Column, string> = { todo: 'To do', doing: 'Doing', review: 'Review', done: 'Done' };
const CARD_W = 0.62;
const CARD_H = 0.4;

export type CardWhere = { kind: 'board' } | { kind: 'held'; agentId: string } | { kind: 'desk'; desk: number };

interface CardGfx {
  id: string;
  mesh: Mesh;
  tex: CanvasTexture;
  ctx: CanvasRenderingContext2D;
  sig: string;
  where: CardWhere;
  /** Animated move: from → to world pose. */
  anim: { from: Vector3; fromQ: Quaternion; at: number; ms: number } | null;
  target: { pos: Vector3; quat: Quaternion };
  hl: boolean;
}

export class TaskBoard {
  readonly group = new Group();
  private cork: Mesh;
  private cards = new Map<string, CardGfx>();
  private geo = new PlaneGeometry(CARD_W, CARD_H);
  private boardTex: CanvasTexture;
  private q = new Quaternion();
  private lift = new Vector3(0, 0.02, 0);
  private xAxis = new Vector3(1, 0, 0);
  private L: OfficeLayout;
  /** World pose of a card held in a hand / lying on a desk, provided by the engine. */
  handOf: (agentId: string) => Object3D | null = () => null;

  constructor(L: OfficeLayout) {
    this.L = L;
    const b = L.board;
    const [c, g] = canvas(1024, 224);
    this.paintBoard(g);
    this.boardTex = tex(c);
    const w = b.x1 - b.x0;
    const h = b.y1 - b.y0;
    this.cork = new Mesh(new PlaneGeometry(w, h), new MeshStandardMaterial({ map: this.boardTex, roughness: 0.95 }));
    this.cork.position.set((b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2, b.z + 0.012);
    this.cork.receiveShadow = true;
    this.group.add(this.cork);
  }

  private paintBoard(g: CanvasRenderingContext2D) {
    g.fillStyle = '#d6ab7c';
    g.fillRect(0, 0, 1024, 224);
    const r = rng(5);
    for (let i = 0; i < 2600; i++) {
      g.fillStyle = r() < 0.5 ? 'rgba(120,80,40,0.16)' : 'rgba(255,236,200,0.18)';
      g.fillRect(r() * 1024, r() * 224, 2, 2);
    }
    for (let i = 0; i < 4; i++) {
      const x = i * 256;
      g.fillStyle = 'rgba(255,248,232,0.92)';
      roundRect(g, x + 64, 8, 128, 26, 6);
      g.fill();
      g.fillStyle = '#5c4738';
      g.font = `700 17px ${FONT}`;
      g.textAlign = 'center';
      g.fillText(COLUMN_NAMES[COLUMNS[i]], x + 128, 27);
      if (i) {
        g.fillStyle = 'rgba(92,71,56,0.35)';
        g.fillRect(x - 1, 14, 2, 200);
      }
    }
    g.textAlign = 'left';
  }

  /** Where a card sits on the board (column, row). */
  private slot(col: Column, row: number, n: number): { pos: Vector3; quat: Quaternion } {
    const b = this.L.board;
    const ci = COLUMNS.indexOf(col);
    const colW = (b.x1 - b.x0) / 4;
    const xc = b.x0 + (ci + 0.5) * colW;
    const top = b.y1 - 0.4;
    const gap = n > 2 ? Math.min(CARD_H + 0.04, (b.y1 - b.y0 - 0.62) / Math.max(1, n - 1)) : CARD_H + 0.04;
    const rr = rng(hash(col + row));
    const pos = new Vector3(xc + (rr() - 0.5) * 0.14, top - row * gap, b.z + 0.03 + row * 0.003);
    const quat = new Quaternion().setFromAxisAngle(new Vector3(0, 0, 1), (rr() - 0.5) * 0.08);
    return { pos, quat };
  }

  /** The card's lying-on-the-desk pose. */
  private deskPose(d: Desk): { pos: Vector3; quat: Quaternion } {
    const fx = Math.sin(d.rot);
    const fz = Math.cos(d.rot);
    const rx = -fz;
    const rz = fx;
    const lx = d.lead ? 0.45 : -0.47;
    const lz = d.lead ? -0.08 : -0.12;
    const pos = new Vector3(d.x + rx * lx + fx * lz, DESK_H + (d.lead ? 0.03 : 0.016), d.z + rz * lx + fz * lz);
    // Lying flat, readable from the occupant's seat.
    const quat = new Quaternion().setFromAxisAngle(new Vector3(0, 1, 0), d.rot + Math.PI).multiply(new Quaternion().setFromAxisAngle(new Vector3(1, 0, 0), -Math.PI / 2 + 0.25));
    return { pos, quat };
  }

  /** Sync cards with the model's tasks; `where` says which are off the board. */
  sync(tasks: TaskCard[], where: (t: TaskCard) => CardWhere, colorOf: (agentId: string | null) => number | null, leadColor: (t: TaskCard) => number | null, now: number, animate: boolean, highlight: string | null) {
    const seen = new Set<string>();
    const byCol = new Map<Column, TaskCard[]>();
    const shown = tasks.slice().sort((a, b) => a.createdAt - b.createdAt || (a.key < b.key ? -1 : 1));
    for (const t of shown) {
      const w = where(t);
      if (w.kind !== 'board') continue;
      const list = byCol.get(t.column) ?? [];
      list.push(t);
      byCol.set(t.column, list);
    }
    // Done column keeps the newest few.
    const done = byCol.get('done');
    if (done && done.length > 5) byCol.set('done', done.slice(-5));
    for (const t of shown) {
      seen.add(t.id);
      let c = this.cards.get(t.id);
      if (!c) {
        const [cv, g] = canvas(192, 124);
        const tx = tex(cv);
        const mesh = new Mesh(this.geo, new MeshStandardMaterial({ map: tx, roughness: 0.9 }));
        mesh.castShadow = true;
        this.group.add(mesh);
        c = { id: t.id, mesh, tex: tx, ctx: g, sig: '', where: { kind: 'board' }, anim: null, target: { pos: new Vector3(), quat: new Quaternion() }, hl: false };
        this.cards.set(t.id, c);
        c.mesh.visible = false;
      }
      const ag = colorOf(t.assignee);
      const sig = `${t.key}|${t.title}|${t.status}|${ag}|${leadColor(t)}|${highlight === t.id}`;
      if (sig !== c.sig) {
        c.sig = sig;
        paintCard(c.ctx, t, ag, leadColor(t), highlight === t.id);
        c.tex.needsUpdate = true;
      }
      const w = where(t);
      let target: { pos: Vector3; quat: Quaternion } | null = null;
      if (w.kind === 'board') {
        const list = byCol.get(t.column) ?? [];
        const row = list.indexOf(t);
        if (row < 0) {
          c.mesh.visible = false;
          c.where = w;
          continue;
        }
        target = this.slot(t.column, row, list.length);
      } else if (w.kind === 'desk') {
        const d = this.L.desks[w.desk];
        if (d) target = this.deskPose(d);
      } else if (w.kind === 'held') {
        const hand = this.handOf(w.agentId);
        if (hand) {
          hand.updateWorldMatrix(true, false);
          const pos = new Vector3().setFromMatrixPosition(hand.matrixWorld);
          if (this.group.parent) pos.sub(this.group.parent.position);
          const quat = new Quaternion();
          hand.getWorldQuaternion(quat);
          quat.multiply(this.q.setFromAxisAngle(new Vector3(1, 0, 0), -0.3));
          target = { pos: pos.add(new Vector3(0, 0.02, 0)), quat };
        }
      }
      if (!target) {
        c.mesh.visible = false;
        continue;
      }
      const moved = c.where.kind !== w.kind || (w.kind === 'board' && !c.target.pos.equals(target.pos)) || (w.kind === 'desk' && c.where.kind === 'desk' && c.where.desk !== w.desk);
      const appear = !c.mesh.visible;
      c.where = w;
      if (w.kind === 'held') {
        // Follows the hand every frame: no tween.
        c.anim = null;
        c.mesh.position.copy(target.pos);
        c.mesh.quaternion.copy(target.quat);
      } else if (moved && animate && !appear) {
        c.anim = { from: c.mesh.position.clone(), fromQ: c.mesh.quaternion.clone(), at: now, ms: w.kind === 'board' ? 900 : 1100 };
      } else if (!c.anim) {
        c.mesh.position.copy(target.pos);
        c.mesh.quaternion.copy(target.quat);
      }
      c.target = target;
      c.mesh.visible = true;
    }
    for (const [id, c] of this.cards) {
      if (seen.has(id)) continue;
      c.mesh.removeFromParent();
      (c.mesh.material as MeshStandardMaterial).dispose();
      c.tex.dispose();
      this.cards.delete(id);
    }
  }

  /** Tween cards in flight; true while something moves. */
  update(now: number): boolean {
    let moving = false;
    for (const c of this.cards.values()) {
      if (c.where.kind === 'held') {
        const hand = this.handOf(c.where.agentId);
        if (hand) {
          hand.updateWorldMatrix(true, false);
          c.mesh.position.setFromMatrixPosition(hand.matrixWorld).add(this.lift);
          hand.getWorldQuaternion(c.mesh.quaternion);
          c.mesh.quaternion.multiply(this.q.setFromAxisAngle(this.xAxis, -0.3));
          // The board group sits in the office frame (offset from the world origin).
          if (this.group.parent) {
            this.group.parent.updateWorldMatrix(true, false);
            c.mesh.position.sub(this.group.parent.position);
          }
        }
        moving = true;
        continue;
      }
      if (!c.anim) continue;
      const t = Math.min(1, (now - c.anim.at) / c.anim.ms);
      const k = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
      c.mesh.position.lerpVectors(c.anim.from, c.target.pos, k);
      c.mesh.position.y += Math.sin(t * Math.PI) * 0.5;
      c.mesh.quaternion.slerpQuaternions(c.anim.fromQ, c.target.quat, k);
      if (t >= 1) c.anim = null;
      else moving = true;
    }
    return moving;
  }

  /** World position of a task's card (for camera framing / labels). */
  cardPosition(taskId: string): Vector3 | null {
    const c = this.cards.get(taskId);
    return c && c.mesh.visible ? c.mesh.position.clone() : null;
  }

  pickables(): Mesh[] {
    return [...this.cards.values()].filter((c) => c.mesh.visible).map((c) => c.mesh);
  }

  taskOfMesh(m: Object3D): string | null {
    for (const c of this.cards.values()) if (c.mesh === m) return c.id;
    return null;
  }

  heldBy(agentId: string): boolean {
    for (const c of this.cards.values()) if (c.where.kind === 'held' && c.where.agentId === agentId) return true;
    return false;
  }

  dispose() {
    for (const c of this.cards.values()) {
      (c.mesh.material as MeshStandardMaterial).dispose();
      c.tex.dispose();
    }
    this.cards.clear();
    this.geo.dispose();
    this.boardTex.dispose();
    this.cork.geometry.dispose();
    (this.cork.material as MeshStandardMaterial).dispose();
  }
}

function paintCard(g: CanvasRenderingContext2D, t: TaskCard, color: number | null, lead: number | null, hl: boolean) {
  const W = 192;
  const H = 124;
  g.clearRect(0, 0, W, H);
  g.fillStyle = t.status === 'cancelled' ? '#ece7dc' : '#fffaf0';
  g.fillRect(0, 0, W, H);
  const stripe = t.status === 'failed' || t.status === 'blocked' ? '#d9534a' : color != null ? identity(color, 'light').ink : '#b5aa98';
  g.fillStyle = stripe;
  g.fillRect(0, 0, W, 14);
  if (hl) {
    g.strokeStyle = stripe;
    g.lineWidth = 8;
    g.strokeRect(4, 4, W - 8, H - 8);
  }
  g.fillStyle = '#3a2f28';
  g.font = `800 19px ${FONT}`;
  g.fillText(t.key, 12, 38);
  if (lead != null) {
    g.fillStyle = identity(lead, 'light').ink;
    g.beginPath();
    g.arc(W - 18, 30, 7, 0, Math.PI * 2);
    g.fill();
  }
  g.fillStyle = '#4b4038';
  g.font = `600 15px ${FONT}`;
  wrap(g, t.title, W - 22, 3).forEach((l, i) => g.fillText(l, 12, 60 + i * 19));
  if (t.status === 'done') {
    g.save();
    g.translate(W - 40, H - 30);
    g.rotate(-0.25);
    g.strokeStyle = '#2f9a5c';
    g.lineWidth = 4;
    g.strokeRect(-30, -14, 60, 28);
    g.fillStyle = '#2f9a5c';
    g.font = `800 15px ${FONT}`;
    g.textAlign = 'center';
    g.fillText('DONE', 0, 6);
    g.restore();
  } else if (t.status === 'failed' || t.status === 'blocked') {
    g.fillStyle = '#d9534a';
    g.font = `800 13px ${FONT}`;
    g.fillText(t.status.toUpperCase(), 12, H - 10);
  } else if (t.status === 'verifying' || t.status === 'reviewing' || t.status === 'merging') {
    g.fillStyle = '#8a6d3b';
    g.font = `700 12px ${FONT}`;
    g.fillText(t.status, 12, H - 10);
  }
  g.textAlign = 'left';
  // Pin.
  g.fillStyle = '#c4443a';
  g.beginPath();
  g.arc(W / 2, 7, 5, 0, Math.PI * 2);
  g.fill();
}

// ───────────── whiteboard ─────────────

export interface TodoList {
  agentId: string;
  name: string;
  color: number;
  steps: { text: string; done: boolean; active: boolean }[];
}

export class Whiteboard {
  readonly mesh: Mesh;
  private tex: CanvasTexture;
  private g: CanvasRenderingContext2D;
  private sig = '';

  constructor(L: OfficeLayout) {
    const wb = L.whiteboard;
    const [c, g] = canvas(1280, 352);
    this.g = g;
    this.tex = tex(c);
    this.mesh = new Mesh(new PlaneGeometry(wb.x1 - wb.x0, wb.y1 - wb.y0), new MeshStandardMaterial({ map: this.tex, roughness: 0.5 }));
    this.mesh.position.set((wb.x0 + wb.x1) / 2, (wb.y0 + wb.y1) / 2, wb.z + 0.02);
    this.mesh.receiveShadow = true;
    this.paint([], [], () => 0, () => '');
  }

  /** Redraw when the todo lists or notes change. */
  set(lists: TodoList[], notes: Note[], colorOf: (author: string) => number, nameOf: (author: string) => string) {
    const sig = JSON.stringify([lists, notes.map((n) => n.id)]);
    if (sig === this.sig) return;
    this.sig = sig;
    this.paint(lists, notes, colorOf, nameOf);
    this.tex.needsUpdate = true;
  }

  private paint(lists: TodoList[], notes: Note[], colorOf: (author: string) => number, nameOf: (author: string) => string) {
    const g = this.g;
    const W = 1280;
    const H = 352;
    g.fillStyle = '#f7f6f1';
    g.fillRect(0, 0, W, H);
    g.fillStyle = 'rgba(0,0,0,0.035)';
    for (let i = 0; i < 6; i++) g.fillRect(30 + i * 210, 20 + (i % 2) * 40, 160, 3);
    const split = 700;
    g.fillStyle = '#2f6fd6';
    g.font = `800 30px ${FONT}`;
    g.fillText('Plans', 22, 40);
    g.fillStyle = '#c4443a';
    g.fillText('Team notes', split + 22, 40);
    g.strokeStyle = 'rgba(60,60,80,0.25)';
    g.lineWidth = 2;
    g.beginPath();
    g.moveTo(split, 16);
    g.lineTo(split, H - 16);
    g.stroke();
    if (!lists.length) {
      g.fillStyle = 'rgba(60,60,80,0.35)';
      g.font = `600 22px ${FONT}`;
      g.fillText('No plan yet', 24, 84);
    }
    const colW = Math.min(225, (split - 30) / Math.max(1, lists.length));
    lists.slice(0, 3).forEach((l, i) => {
      const x = 18 + i * colW;
      const ink = identity(l.color, 'light').ink;
      g.fillStyle = ink;
      g.font = `800 22px ${FONT}`;
      g.fillText(clean(l.name, 18), x + 4, 76);
      l.steps.slice(0, 6).forEach((s, j) => {
        const y = 92 + j * 42;
        g.fillStyle = s.done ? 'rgba(255,215,94,0.45)' : s.active ? '#ffd75e' : 'rgba(255,215,94,0.85)';
        g.save();
        g.translate(x + 2, y);
        g.rotate(((hash(s.text) % 7) - 3) * 0.004);
        g.fillRect(0, 0, colW - 14, 36);
        g.restore();
        g.fillStyle = s.done ? 'rgba(60,50,30,0.5)' : '#3a3020';
        g.font = `${s.active ? 800 : 600} 19px ${FONT}`;
        const txt = wrap(g, s.text, colW - 52, 1)[0] ?? '';
        g.fillText(txt, x + 34, y + 25);
        g.strokeStyle = s.done ? '#2f9a5c' : '#6b5a3a';
        g.lineWidth = 2.5;
        g.strokeRect(x + 10, y + 10, 15, 15);
        if (s.done) {
          g.beginPath();
          g.moveTo(x + 11, y + 17);
          g.lineTo(x + 17, y + 23);
          g.lineTo(x + 27, y + 8);
          g.stroke();
          g.fillStyle = 'rgba(60,50,30,0.45)';
          g.fillRect(x + 34, y + 18, Math.min(g.measureText(txt).width, colW - 52), 2);
        }
      });
    });
    const shown = notes.slice(-6);
    if (!shown.length) {
      g.fillStyle = 'rgba(60,60,80,0.35)';
      g.font = `600 22px ${FONT}`;
      g.fillText('No notes yet', split + 24, 84);
    }
    shown.forEach((n, i) => {
      const col = i % 3;
      const row = Math.floor(i / 3);
      const x = split + 22 + col * 188;
      const y = 56 + row * 146;
      const tilt = ((hash(n.id) % 9) - 4) * 0.012;
      const c = identity(colorOf(n.author), 'light');
      g.save();
      g.translate(x + 85, y + 65);
      g.rotate(tilt);
      g.shadowColor = 'rgba(0,0,0,0.18)';
      g.shadowBlur = 6;
      g.shadowOffsetY = 3;
      g.fillStyle = ['#ffe58a', '#ffd1dc', '#cdeccf', '#cfe3ff', '#ffe0b8'][hash(n.author) % 5];
      g.fillRect(-85, -65, 170, 130);
      g.shadowColor = 'transparent';
      g.fillStyle = c.ink;
      g.fillRect(-85, -65, 170, 8);
      g.fillStyle = '#3a3020';
      g.font = `600 18px ${FONT}`;
      wrap(g, n.text, 156, 4).forEach((l, k) => g.fillText(l, -77, -34 + k * 22));
      g.fillStyle = c.ink;
      g.font = `800 15px ${FONT}`;
      g.fillText('— ' + clean(nameOf(n.author), 16), -77, 56);
      g.restore();
    });
  }

  dispose() {
    this.tex.dispose();
    this.mesh.geometry.dispose();
    (this.mesh.material as MeshStandardMaterial).dispose();
  }
}

// ───────────── server room ─────────────

export interface TestState {
  running: { command: string; color: number; since: number } | null;
  last: { ok: boolean; passed: number | null; failed: number | null; command: string; at: number } | null;
  history: boolean[];
}

export class ServerRoom {
  readonly group = new Group();
  private screen: Mesh;
  private tex: CanvasTexture;
  private g: CanvasRenderingContext2D;
  private sig = '';
  private leds: InstancedMesh;
  private ledMat = new MeshBasicMaterial({ toneMapped: false });
  private ledGeo = new BoxGeometry(0.06, 0.035, 0.014);
  private ledBase: Color[] = [];
  private ledSig = '';
  private theme: ThemeName = 'dark';

  constructor(L: OfficeLayout) {
    const tm = L.testMonitor;
    const [c, g] = canvas(512, 256);
    this.g = g;
    this.tex = tex(c);
    this.screen = new Mesh(new PlaneGeometry(tm.w, tm.h), new MeshBasicMaterial({ map: this.tex, toneMapped: false }));
    this.screen.position.set(tm.x, tm.y, tm.z + 0.05);
    this.group.add(this.screen);
    // LEDs on each rack's doors.
    const per = 12;
    this.leds = new InstancedMesh(this.ledGeo, this.ledMat, L.racks.length * per);
    const m = new Matrix4();
    const q = new Quaternion();
    const one = new Vector3(1, 1, 1);
    let k = 0;
    const r = rng(99);
    const palette = [lin('#5dff9d'), lin('#5dff9d'), lin('#5dff9d'), lin('#6ab8ff'), lin('#ffc35d')];
    for (const rk of L.racks) {
      q.setFromAxisAngle(new Vector3(0, 1, 0), rk.rot);
      const fx = Math.sin(rk.rot);
      const fz = Math.cos(rk.rot);
      const rx = -fz;
      const rz = fx;
      for (let i = 0; i < per; i++) {
        const side = i < per / 2 ? -1 : 1;
        const row = i % (per / 2);
        const lx = side * 0.3 + (row % 2) * 0.06 - 0.03;
        const y = 0.35 + row * 0.22;
        const pos = new Vector3(rk.x + fx * 0.395 + rx * lx, y, rk.z + fz * 0.395 + rz * lx);
        this.leds.setMatrixAt(k, m.compose(pos, q, one));
        this.ledBase.push(new Color().setRGB(...palette[Math.floor(r() * palette.length)]));
        k++;
      }
    }
    this.leds.instanceMatrix.needsUpdate = true;
    this.leds.computeBoundingSphere();
    this.group.add(this.leds);
  }

  applyTheme(t: OfficeTheme) {
    this.theme = t.name;
    (this.screen.material as MeshBasicMaterial).color.setScalar(t.screenGlow);
    this.ledSig = '';
  }

  /** Paint the test monitor and LEDs; returns the next stepped frame time. */
  update(state: TestState, busy: boolean, now: number, reducedMotion: boolean): number {
    let next = Infinity;
    const step = state.running && !reducedMotion ? Math.floor(now / 300) : 0;
    if (state.running && !reducedMotion) next = (step + 1) * 300;
    const sig = JSON.stringify([state.running?.command, state.running?.color, state.last, state.history, step]);
    if (sig !== this.sig) {
      this.sig = sig;
      this.paint(state, step);
      this.tex.needsUpdate = true;
    }
    // LEDs: blink (4 Hz) only while a command or test really runs.
    const blink = busy && !reducedMotion;
    const ls = blink ? Math.floor(now / 250) : -1;
    if (blink) next = Math.min(next, (ls + 1) * 250);
    const lsig = `${busy}|${ls}|${this.theme}`;
    if (lsig !== this.ledSig) {
      this.ledSig = lsig;
      const dim = this.theme === 'dark' ? 0.35 : 0.5;
      const c = new Color();
      for (let i = 0; i < this.ledBase.length; i++) {
        let on = busy ? (blink ? (hash(`${i}:${ls}`) & 3) !== 0 : true) : i % 3 === 0;
        if (!busy && i % 7 === 0) on = true;
        const k = on ? (this.theme === 'dark' ? 1.4 : 1.0) : dim * 0.4;
        c.copy(this.ledBase[i]).multiplyScalar(k);
        this.leds.setColorAt(i, c);
      }
      if (this.leds.instanceColor) this.leds.instanceColor.needsUpdate = true;
    }
    return next;
  }

  private paint(s: TestState, step: number) {
    const g = this.g;
    const W = 512;
    const H = 256;
    const last = s.last;
    const bg = s.running ? '#12161f' : last ? (last.ok ? '#0f2a1b' : '#2e1214') : '#12161f';
    g.fillStyle = bg;
    g.fillRect(0, 0, W, H);
    g.font = `700 20px ${FONT}`;
    g.fillStyle = 'rgba(255,255,255,0.55)';
    g.fillText('TEST BENCH', 24, 38);
    if (s.running) {
      const ink = identity(s.running.color, 'dark').screen;
      g.fillStyle = ink;
      g.font = `700 34px ${MONO}`;
      g.fillText(clean(s.running.command, 22), 24, 112);
      g.fillStyle = 'rgba(255,255,255,0.7)';
      g.font = `600 22px ${FONT}`;
      g.fillText('Running' + '.'.repeat((step % 3) + 1), 24, 156);
      // Indeterminate stripes (honest: we don't know the progress).
      for (let i = 0; i < 12; i++) {
        g.fillStyle = (i + step) % 4 === 0 ? ink : 'rgba(255,255,255,0.12)';
        g.fillRect(24 + i * 38, 186, 30, 10);
      }
    } else if (last) {
      g.fillStyle = last.ok ? '#5ee08f' : '#ff6b5e';
      g.font = `800 64px ${FONT}`;
      const big = last.ok ? (last.passed != null ? `✓ ${last.passed} passed` : '✓ passed') : last.failed != null ? `✕ ${last.failed} failed` : '✕ failed';
      g.fillText(big, 24, 128);
      g.fillStyle = 'rgba(255,255,255,0.7)';
      g.font = `600 22px ${MONO}`;
      const sub = (!last.ok && last.passed != null ? `${last.passed} passed · ` : '') + clean(last.command, 26);
      g.fillText(sub, 26, 168);
    } else {
      g.fillStyle = 'rgba(255,255,255,0.4)';
      g.font = `600 30px ${FONT}`;
      g.fillText('No test run yet', 24, 120);
    }
    // Recent results as dots (real history).
    s.history.slice(-10).forEach((ok, i) => {
      g.fillStyle = ok ? '#5ee08f' : '#ff6b5e';
      g.beginPath();
      g.arc(W - 30 - i * 22, 30, 7, 0, Math.PI * 2);
      g.fill();
    });
  }

  dispose() {
    this.tex.dispose();
    this.screen.geometry.dispose();
    (this.screen.material as MeshBasicMaterial).dispose();
    this.ledGeo.dispose();
    this.ledMat.dispose();
  }
}

// ───────────── mail room ─────────────

export class MailRoom {
  readonly group = new Group();
  private flag: Group;
  private flagUp = 0;
  private flagAngle = -Math.PI / 2;
  private envelopes: InstancedMesh;
  private seals: InstancedMesh;
  private mat = new MeshStandardMaterial({ roughness: 0.8 });
  private sealMat = new MeshStandardMaterial({ color: new Color().setRGB(...lin('#e3b341')), roughness: 0.4, metalness: 0.4, emissive: new Color(0.25, 0.17, 0.02) });
  private geo = new BoxGeometry(0.34, 0.21, 0.02);
  private sealGeo = new BoxGeometry(0.06, 0.06, 0.012);
  private slots: Vector3[] = [];
  private q = new Quaternion();
  private sig = '';

  constructor(L: OfficeLayout) {
    const mb = L.mailbox;
    this.flag = new Group();
    const pole = new Mesh(new BoxGeometry(0.02, 0.26, 0.02), new MeshStandardMaterial({ color: 0x333338 }));
    pole.position.y = 0.13;
    const cloth = new Mesh(new BoxGeometry(0.012, 0.09, 0.13), new MeshStandardMaterial({ color: new Color().setRGB(...lin('#f2c14e')) }));
    cloth.position.set(0, 0.21, 0.065);
    this.flag.add(pole, cloth);
    // On the mailbox's right side.
    const fx = Math.sin(mb.rot);
    const fz = Math.cos(mb.rot);
    this.flag.position.set(mb.x - fz * 0.23 + fx * 0.02, 0.55, mb.z + fx * 0.23 + fz * 0.02);
    this.flag.rotation.y = mb.rot + Math.PI / 2;
    this.group.add(this.flag);
    // Envelope slots in the pigeonholes (row by row, from the top).
    const ph = L.pigeonholes;
    const Wd = 3.0;
    const Hh = 1.05;
    for (let j = ph.rows - 1; j >= 0; j--)
      for (let i = 0; i < ph.cols; i++) this.slots.push(new Vector3(ph.x - Wd / 2 + ((i + 0.5) * Wd) / ph.cols, 0.25 + (j * Hh) / ph.rows + 0.11, ph.z + 0.02));
    this.envelopes = new InstancedMesh(this.geo, this.mat, this.slots.length);
    this.seals = new InstancedMesh(this.sealGeo, this.sealMat, this.slots.length);
    this.envelopes.count = 0;
    this.seals.count = 0;
    this.envelopes.castShadow = true;
    this.group.add(this.envelopes, this.seals);
  }

  /** One envelope per commit (agent color); a gold seal when its task merged. */
  set(commits: { agentColor: number; merged: boolean; sha: string }[]) {
    const shown = commits.slice(-this.slots.length);
    const sig = shown.map((c) => `${c.sha}${c.agentColor}${c.merged ? 1 : 0}`).join(',');
    if (sig === this.sig) return;
    this.sig = sig;
    const m = new Matrix4();
    const one = new Vector3(1, 1, 1);
    let s = 0;
    shown.forEach((c, i) => {
      const p = this.slots[i];
      const tilt = ((hash(c.sha) % 9) - 4) * 0.03;
      // Leaning back in the cubby, so its face shows from above.
      this.q.setFromEuler(new Euler(-0.55, 0, tilt));
      this.envelopes.setMatrixAt(i, m.compose(p, this.q, one));
      const col = identity(c.agentColor, 'light').shirt;
      this.envelopes.setColorAt(i, new Color().setRGB(0.85 * 0.9 + col[0] * 0.1, 0.8 * 0.9 + col[1] * 0.1, 0.7 * 0.9 + col[2] * 0.1).lerp(new Color().setRGB(...col), 0.35));
      if (c.merged) this.seals.setMatrixAt(s++, m.compose(p.clone().add(new Vector3(0, -0.02, 0.016)), this.q, one));
    });
    this.envelopes.count = shown.length;
    this.seals.count = s;
    this.envelopes.instanceMatrix.needsUpdate = true;
    if (this.envelopes.instanceColor) this.envelopes.instanceColor.needsUpdate = true;
    this.seals.instanceMatrix.needsUpdate = true;
    this.envelopes.computeBoundingSphere();
    this.seals.computeBoundingSphere();
  }

  /** Raise the flag (outgoing mail) for a while. */
  raise(now: number, ms = 9000) {
    this.flagUp = now + ms;
  }

  update(now: number, dt: number): { moving: boolean; nextAt: number } {
    // Up (vertical) while there is outgoing mail; folded down otherwise.
    const goal = now < this.flagUp ? 0 : -Math.PI / 2;
    const d = goal - this.flagAngle;
    let moving = false;
    if (Math.abs(d) > 0.002) {
      this.flagAngle += d * (1 - Math.exp(-dt * 7));
      moving = true;
    } else this.flagAngle = goal;
    this.flag.rotation.z = this.flagAngle;
    return { moving, nextAt: now < this.flagUp ? this.flagUp + 5 : Infinity };
  }

  dispose() {
    this.geo.dispose();
    this.sealGeo.dispose();
    this.mat.dispose();
    this.sealMat.dispose();
    this.flag.traverse((o) => {
      if (o instanceof Mesh) {
        o.geometry.dispose();
        (o.material as MeshStandardMaterial).dispose();
      }
    });
  }
}

// ───────────── archive ─────────────

export class Archive {
  readonly group = new Group();
  private plates: InstancedMesh;
  private catPlates: InstancedMesh;
  private mat = new MeshBasicMaterial({ toneMapped: false });
  private geo = new BoxGeometry(0.24, 0.05, 0.01);
  private catGeo = new BoxGeometry(0.09, 0.03, 0.008);
  private flashAt = -1e9;
  private flashIdx = -1;
  private catFlashAt = -1e9;
  private count = 0;
  private triples = 0;
  private theme: ThemeName = 'dark';
  private sig = '';

  constructor(L: OfficeLayout) {
    const m = new Matrix4();
    const one = new Vector3(1, 1, 1);
    const q = new Quaternion();
    const n = L.cabinets.length * 3;
    this.plates = new InstancedMesh(this.geo, this.mat, n);
    let k = 0;
    for (const cb of L.cabinets) {
      q.setFromAxisAngle(new Vector3(0, 1, 0), cb.rot);
      const fx = Math.sin(cb.rot);
      const fz = Math.cos(cb.rot);
      for (let d = 0; d < 3; d++) {
        const y = 0.2 + (2 - d) * 0.36 + 0.1;
        this.plates.setMatrixAt(k++, m.compose(new Vector3(cb.x + fx * 0.32, y, cb.z + fz * 0.32), q, one));
      }
    }
    const cat = L.catalog;
    this.catPlates = new InstancedMesh(this.catGeo, this.mat, 24);
    k = 0;
    for (let j = 3; j >= 0; j--)
      for (let i = 0; i < 6; i++) {
        const x = cat.x - 0.7 + 0.12 + i * 0.235;
        const y = 0.2 + j * 0.2 + 0.03;
        this.catPlates.setMatrixAt(k++, m.compose(new Vector3(x, y, cat.z + 0.365), q.identity(), one));
      }
    for (const im of [this.plates, this.catPlates]) {
      im.instanceMatrix.needsUpdate = true;
      im.computeBoundingSphere();
      this.group.add(im);
    }
  }

  applyTheme(t: OfficeTheme) {
    this.theme = t.name;
    this.sig = '';
  }

  set(memories: number, triples: number, now: number, flash: boolean) {
    if (flash && memories > this.count) {
      this.flashAt = now;
      this.flashIdx = (memories - 1) % this.plates.count;
    }
    if (flash && triples > this.triples) this.catFlashAt = now;
    this.count = memories;
    this.triples = triples;
  }

  update(now: number): boolean {
    const fl = Math.max(0, 1 - (now - this.flashAt) / 1600);
    const cf = Math.max(0, 1 - (now - this.catFlashAt) / 1400);
    const sig = `${this.count}|${this.triples}|${fl.toFixed(2)}|${cf.toFixed(2)}|${this.theme}`;
    if (sig === this.sig) return false;
    this.sig = sig;
    const dark = this.theme === 'dark';
    const off = new Color().setRGB(...lin(dark ? '#3a3f3a' : '#e8e1d2'));
    const lit = new Color().setRGB(...lin('#ffd98a')).multiplyScalar(dark ? 1.3 : 1);
    const hot = new Color().setRGB(...lin('#fff1c4')).multiplyScalar(dark ? 2.2 : 1.4);
    const c = new Color();
    const n = this.plates.count;
    const litN = Math.min(n, this.count);
    for (let i = 0; i < n; i++) {
      c.copy(i < litN ? lit : off);
      if (i === this.flashIdx && fl > 0) c.lerp(hot, fl);
      this.plates.setColorAt(i, c);
    }
    const ct = Math.min(24, this.triples);
    for (let i = 0; i < 24; i++) {
      c.copy(i < ct ? lit : off);
      if (i === ct - 1 && cf > 0) c.lerp(hot, cf);
      this.catPlates.setColorAt(i, c);
    }
    if (this.plates.instanceColor) this.plates.instanceColor.needsUpdate = true;
    if (this.catPlates.instanceColor) this.catPlates.instanceColor.needsUpdate = true;
    return fl > 0 || cf > 0;
  }

  dispose() {
    this.geo.dispose();
    this.catGeo.dispose();
    this.mat.dispose();
  }
}

// ───────────── artifacts on desks ─────────────

export class DeskFrames {
  readonly group = new Group();
  private frames = new Map<string, { mesh: Group; tex: Texture | null; url: string | null; mat: MeshBasicMaterial }>();
  private loader = new TextureLoader();
  private frameGeo = new BoxGeometry(0.3, 0.24, 0.02);
  private picGeo = new PlaneGeometry(0.26, 0.2);
  private frameMat = new MeshStandardMaterial({ color: new Color().setRGB(...lin('#5c4738')), roughness: 0.6 });
  constructor(private onLoad: () => void) {}

  /** Place the newest artifact of each agent on its desk. */
  set(list: { artifact: Artifact; desk: Desk; url: string | null; slot: number }[], now: number, animate: boolean) {
    const keep = new Set<string>();
    for (const { artifact: a, desk: d, url, slot } of list) {
      const key = a.id;
      keep.add(key);
      let f = this.frames.get(key);
      if (!f) {
        const g = new Group();
        const frame = new Mesh(this.frameGeo, this.frameMat);
        frame.castShadow = true;
        const mat = new MeshBasicMaterial({ color: 0xffffff });
        const pic = new Mesh(this.picGeo, mat);
        pic.position.z = 0.011;
        g.add(frame, pic);
        // A little stand.
        const stand = new Mesh(this.frameGeo, this.frameMat);
        stand.scale.set(0.3, 0.9, 1);
        stand.position.set(0, -0.02, -0.06);
        stand.rotation.x = -0.5;
        g.add(stand);
        this.group.add(g);
        f = { mesh: g, tex: null, url: null, mat };
        this.frames.set(key, f);
        if (animate) g.userData.born = now;
        f.tex = docTexture(a);
        mat.map = f.tex;
        mat.needsUpdate = true;
      }
      if (url && url !== f.url && (a.kind === 'image' || a.kind === 'svg')) {
        f.url = url;
        const ff = f;
        this.loader.load(
          url,
          (t) => {
            t.colorSpace = SRGBColorSpace;
            ff.tex?.dispose();
            ff.tex = t;
            ff.mat.map = t;
            ff.mat.needsUpdate = true;
            this.onLoad();
          },
          undefined,
          () => {},
        );
      }
      const fx = Math.sin(d.rot);
      const fz = Math.cos(d.rot);
      const rx = -fz;
      const rz = fx;
      const off = slot * 0.34;
      f.mesh.position.set(d.propSpot[0] + rx * off * (d.lead ? 1 : -1), DESK_H + (d.lead ? 0.15 : 0.13), d.propSpot[1] + rz * off * (d.lead ? 1 : -1));
      // Facing the occupant (and so, often, the camera from behind them).
      f.mesh.rotation.set(0, d.rot + Math.PI + (d.lead ? -0.3 : 0.3), 0);
    }
    for (const [k, f] of this.frames) {
      if (keep.has(k)) continue;
      f.mesh.removeFromParent();
      f.tex?.dispose();
      f.mat.dispose();
      this.frames.delete(k);
    }
  }

  /** Pop-in animation; true while running. */
  update(now: number): boolean {
    let moving = false;
    for (const f of this.frames.values()) {
      const born = f.mesh.userData.born as number | undefined;
      if (born == null) continue;
      const t = Math.min(1, (now - born) / 500);
      const s = t < 1 ? 1 + Math.sin(t * Math.PI) * 0.35 - (1 - t) * 0.8 : 1;
      f.mesh.scale.setScalar(Math.max(0.01, s));
      if (t >= 1) delete f.mesh.userData.born;
      else moving = true;
    }
    return moving;
  }

  dispose() {
    for (const f of this.frames.values()) {
      f.tex?.dispose();
      f.mat.dispose();
    }
    this.frameGeo.dispose();
    this.picGeo.dispose();
    this.frameMat.dispose();
  }
}

function docTexture(a: Artifact): Texture {
  const [c, g] = canvas(128, 100);
  const colors: Record<string, string> = { image: '#6aa0d8', svg: '#8c6bb1', html: '#d97757', markdown: '#5a8f8a', pdf: '#c45b4a', csv: '#6d8b5b' };
  g.fillStyle = '#fbf8f1';
  g.fillRect(0, 0, 128, 100);
  g.fillStyle = colors[a.kind] ?? '#9aa0aa';
  g.fillRect(0, 0, 128, 24);
  g.fillStyle = '#ffffff';
  g.font = `800 15px ${FONT}`;
  g.fillText(a.kind.toUpperCase(), 8, 17);
  g.fillStyle = '#4b4038';
  g.font = `600 12px ${FONT}`;
  wrap(g, clean(a.title || a.path, 40), 112, 3).forEach((l, i) => g.fillText(l, 8, 44 + i * 16));
  return tex(c);
}
