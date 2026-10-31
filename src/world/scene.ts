// The neighbourhood scene: flat ground, a plot outline per lot, each owned building seen from
// outside (roof on, its windows lit while the open repo's crew works), and a DOM sign above each
// plot. Same camera feel as the office (CameraRig: drag pans, right-drag or Shift orbits, wheel
// or pinch zooms, W/A/S/D glides, Q/E quarter turns, 0 frames everything), the same lighting for
// the hour (palette.themeAt), and the same render-on-demand loop.

import {
  Color,
  DirectionalLight,
  Group,
  HemisphereLight,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  NeutralToneMapping,
  PCFShadowMap,
  PlaneGeometry,
  Scene,
  Vector3,
  WebGLRenderer,
  type BufferGeometry,
} from 'three';
import type { Lot } from '../lib/gen/Lot';
import type { LotSlot } from '../lib/gen/LotSlot';
import { CameraRig } from '../office/camera';
import { Builder } from '../office/gfx/builder';
import { color, lin, MAT, mix, themeAt, type OfficeTheme, type RGB } from '../office/palette';
import { browserHost, FrameLoop, type StepResult } from '../office/scheduler';
import { exteriorOf, STOREY, type Box } from './exterior';
import { buildingOrigin, extent, lightsOf, plotAt, plotsOf, signOf, slotKind, type Plot } from './map';
import { townOf, TownGfx } from './town';

export interface SceneOptions {
  /** A plot was clicked (null: the ground), at this screen point. */
  onpick: (slotId: string | null, at: { x: number; y: number }) => void;
  onhover?: (slotId: string | null) => void;
}

export interface SceneState {
  slots: LotSlot[];
  /** Each owned plot's building (by slot id). */
  lots: Map<string, Lot>;
  /** The open repo's workspace id. */
  live: string | null;
  /** The open repo's crew (the Lead aside) and how many of them work right now. */
  crew: number;
  working: number;
  /** Your other offices' tasks in flight, by workspace id: they keep working, lit. */
  away: Map<string, number>;
  /** The plot whose panel is open. */
  selected: string | null;
}

interface PlotGfx {
  plot: Plot;
  group: Group;
  edge: MeshBasicMaterial;
  fill: MeshStandardMaterial;
  glass: MeshStandardMaterial | null;
  /** Building height (the sign floats above it). */
  top: number;
  sign: HTMLDivElement;
  signKey: string;
}

const GROUND = '#b4c09c';
const GROUND_NIGHT = '#6f7a62';

export class NeighbourhoodScene {
  private renderer: WebGLRenderer;
  private scene = new Scene();
  private rig = new CameraRig(1);
  private hemi = new HemisphereLight();
  private key = new DirectionalLight();
  private ground: Mesh;
  private groundMat = new MeshStandardMaterial({ roughness: 1, metalness: 0 });
  private matte = new MeshStandardMaterial({ vertexColors: true, roughness: 0.85, metalness: 0 });
  private world = new Group();
  private labels: HTMLDivElement;
  private loop: FrameLoop;
  private ro: ResizeObserver;
  private gfx = new Map<string, PlotGfx>();
  private town: TownGfx | null = null;
  private plots: Plot[] = [];
  private geos: BufferGeometry[] = [];
  private state: SceneState = { slots: [], lots: new Map(), live: null, crew: 0, working: 0, away: new Map(), selected: null };
  private hover: string | null = null;
  private theme: OfficeTheme;
  private width = 1;
  private height = 1;
  private held = new Set<string>();
  private heldFast = false;
  private userMoved = false;
  private disposed = false;
  private cleanups: (() => void)[] = [];
  private v = new Vector3();
  private minuteTimer = 0;
  private fixedHour: number | null = null;
  /** The app's UI over the view (the rail, the header): the camera centres on what's left. */
  private insets = { top: 0, right: 0, bottom: 0, left: 0 };

  constructor(
    private container: HTMLElement,
    private opts: SceneOptions,
  ) {
    this.theme = themeAt(this.hour());
    this.renderer = new WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' });
    this.renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
    this.renderer.toneMapping = NeutralToneMapping;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = PCFShadowMap;
    this.renderer.shadowMap.autoUpdate = false;
    const canvas = this.renderer.domElement;
    canvas.className = 'nb-canvas';
    canvas.setAttribute('aria-hidden', 'true');
    container.appendChild(canvas);
    this.labels = document.createElement('div');
    this.labels.className = 'nb-signs';
    container.appendChild(this.labels);

    // Seen from the south-west, a quarter turn from the office's view: the ready-made designs
    // put their front door on the south side and their windows west and north.
    this.rig.turns = -1;
    this.rig.set({ yaw: this.rig.homeYaw + 0.4 });
    this.rig.minDist = 14;
    this.rig.maxDist = 420;
    // A near plane well off the camera keeps the depth precise out to the far zoom: the streets
    // sit centimetres over the grass and flickered against it at 0.3.
    this.rig.camera.near = 2;
    this.rig.camera.far = 1200;
    this.rig.camera.updateProjectionMatrix();
    this.key.castShadow = true;
    this.key.shadow.mapSize.set(2048, 2048);
    this.key.shadow.radius = 4;
    this.key.shadow.bias = -0.0005;
    this.key.shadow.normalBias = 0.03;
    this.scene.add(this.hemi, this.key, this.key.target, this.world);
    this.ground = new Mesh(new PlaneGeometry(1, 1).rotateX(-Math.PI / 2), this.groundMat);
    this.ground.receiveShadow = true;
    this.ground.position.y = -0.02;
    this.scene.add(this.ground);

    this.loop = new FrameLoop((now, dt) => this.step(now, dt), browserHost());
    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(container);
    this.listen();
    this.resize();
    this.applyTheme();
    // Relight for the hour now and then, as the office does.
    this.minuteTimer = window.setInterval(() => this.setHour(this.fixedHour), 60_000);
  }

  /** Light the map for a fixed hour (0..24) instead of the real clock (null). */
  setHour(h: number | null) {
    this.fixedHour = h;
    this.theme = themeAt(this.hour());
    this.applyTheme();
  }

  private hour(): number {
    if (this.fixedHour != null) return this.fixedHour;
    const d = new Date();
    return d.getHours() + d.getMinutes() / 60;
  }

  set(next: SceneState) {
    const rebuild =
      next.slots !== this.state.slots || next.lots !== this.state.lots || next.live !== this.state.live;
    this.state = next;
    if (rebuild) this.rebuild();
    else this.restyle();
    this.wake();
  }

  setInsets(insets: { top: number; right: number; bottom: number; left: number }) {
    this.insets = insets;
    this.rig.setFreeArea(this.width, this.height, insets);
    if (!this.userMoved && this.plots.length) this.overview(true);
    this.wake();
  }

  /** Frame the whole neighbourhood. */
  overview(instant = false) {
    const e = extent(this.plots);
    const i = this.insets;
    const availX = Math.max(0.3, (this.width - i.left - i.right) / this.width);
    const availY = Math.max(0.3, (this.height - i.top - i.bottom) / this.height);
    // Far enough that the whole grid's circle fits the free part of the view (rig.fit starts
    // 40 m out, made for one office: from there a big map is partly behind the camera).
    const r = Math.hypot(e.x1 - e.x0, e.z1 - e.z0) / 2;
    const half = Math.tan((this.rig.camera.fov * Math.PI) / 360);
    const fill = Math.min(availY, availX * this.rig.camera.aspect);
    const dist = (r / (half * fill)) * 0.92;
    this.rig.flyTo({ x: (e.x0 + e.x1) / 2, y: 0, z: (e.z0 + e.z1) / 2, yaw: this.rig.homeYaw + 0.4, pitch: 0.86, dist }, instant);
    this.userMoved = false;
    this.wake();
  }

  /** Fly to one plot. */
  focus(slotId: string) {
    const p = this.plots.find((q) => q.id === slotId);
    if (!p) return;
    this.rig.flyTo({ x: p.cx, z: p.cz, dist: Math.max(30, Math.max(p.w, p.d) * 2.1) });
    this.wake();
  }

  /** Where a plot's sign is on screen (the panel opens beside it). */
  screenOf(slotId: string): { x: number; y: number } | null {
    const g = this.gfx.get(slotId);
    if (!g) return null;
    this.v.set(g.plot.cx, g.top, g.plot.cz).project(this.rig.camera);
    if (this.v.z > 1) return null;
    return { x: (this.v.x * 0.5 + 0.5) * this.width, y: (-this.v.y * 0.5 + 0.5) * this.height };
  }

  private wake = () => this.loop.wake();

  // ───────────── building the map ─────────────

  private rebuild() {
    for (const g of this.gfx.values()) {
      this.world.remove(g.group);
      g.edge.dispose();
      g.fill.dispose();
      g.glass?.dispose();
      g.sign.remove();
    }
    for (const geo of this.geos) geo.dispose();
    this.geos = [];
    this.gfx.clear();
    const firstTime = this.plots.length === 0;
    this.plots = plotsOf(this.state.slots);
    const e = extent(this.plots);
    const pad = 40;
    // Ground to the horizon, whatever the zoom.
    this.ground.scale.set(e.x1 - e.x0 + pad * 60, 1, e.z1 - e.z0 + pad * 60);
    this.ground.position.set((e.x0 + e.x1) / 2, -0.02, (e.z0 + e.z1) / 2);
    // The town around the plots: streets, sidewalks, lamps, trees and the neighbours' houses.
    if (this.town) {
      this.world.remove(this.town.group);
      this.town.dispose();
    }
    this.town = new TownGfx(townOf(this.plots));
    this.town.setNight(this.theme.name === 'dark', this.theme.lamp);
    this.world.add(this.town.group);
    const tb = this.town.town.bounds;
    this.rig.boundsX = Math.max(Math.abs(tb.x0), Math.abs(tb.x1));
    this.rig.boundsZ = Math.max(Math.abs(tb.z0), Math.abs(tb.z1));
    this.state.slots.forEach((s, i) => this.addPlot(s, this.plots[i]));
    const cam = this.key.shadow.camera;
    const half = Math.max(tb.x1 - tb.x0, tb.z1 - tb.z0) / 2 + 4;
    cam.left = -half;
    cam.right = half;
    cam.top = half;
    cam.bottom = -half;
    cam.far = 400;
    cam.updateProjectionMatrix();
    this.key.target.position.set((tb.x0 + tb.x1) / 2, 0, (tb.z0 + tb.z1) / 2);
    this.placeKey();
    this.restyle();
    this.renderer.shadowMap.needsUpdate = true;
    if (firstTime || !this.userMoved) this.overview(firstTime);
  }

  private addPlot(s: LotSlot, p: Plot) {
    const group = new Group();
    const kind = slotKind(s);
    // The plot: a paved (owned) or grassy (for sale) pad, with an outline.
    const fill = new MeshStandardMaterial({ roughness: 1, metalness: 0 });
    const pad = new Mesh(this.geo(new PlaneGeometry(p.w, p.d).rotateX(-Math.PI / 2)), fill);
    pad.position.set(p.cx, 0, p.cz);
    pad.receiveShadow = true;
    const edge = new MeshBasicMaterial({ transparent: true, toneMapped: false });
    const b = new Builder();
    const t = 0.22;
    b.span(p.x0 - t, 0.005, p.z0 - t, p.x1 + t, 0.03, p.z0, [1, 1, 1]);
    b.span(p.x0 - t, 0.005, p.z1, p.x1 + t, 0.03, p.z1 + t, [1, 1, 1]);
    b.span(p.x0 - t, 0.005, p.z0, p.x0, 0.03, p.z1, [1, 1, 1]);
    b.span(p.x1, 0.005, p.z0, p.x1 + t, 0.03, p.z1, [1, 1, 1]);
    const outline = new Mesh(this.geo(b.build()), edge);
    outline.renderOrder = 1;
    group.add(pad, outline);

    let top = 0.4;
    let glass: MeshStandardMaterial | null = null;
    const lot = kind === 'office' || kind === 'building' ? this.state.lots.get(s.id) : undefined;
    const ex = lot && exteriorOf(lot);
    if (lot && ex) {
      const o = buildingOrigin(p, lot);
      const wb = new Builder();
      const wall = lin(MAT.wall);
      const cap = lin(MAT.slabDark);
      const roof = lin('#c9c2b6');
      const add = (bx: Box, c: RGB) => wb.span(bx[0] + o.x, bx[1], bx[2] + o.z, bx[3] + o.x, bx[4], bx[5] + o.z, c);
      // A plinth, then the walls, the roof and the doors.
      wb.span(ex.x0 + o.x - 0.3, 0, ex.z0 + o.z - 0.3, ex.x1 + o.x + 0.3, 0.12, ex.z1 + o.z + 0.3, lin(MAT.slab));
      for (const w of ex.walls) add(w, wall);
      ex.roof.forEach((r, i) => add(r, i === 0 ? roof : cap));
      for (const d of ex.doors) add(d.box, lin(d.kind === 'rollup' ? MAT.metal : d.kind === 'glass' ? MAT.glass : MAT.door));
      const m = new Mesh(this.geo(wb.build()), this.matte);
      m.castShadow = m.receiveShadow = true;
      group.add(m);
      if (ex.glass.length) {
        const gb = new Builder();
        for (const g of ex.glass) gb.span(g[0] + o.x, g[1], g[2] + o.z, g[3] + o.x, g[4], g[5] + o.z, [1, 1, 1]);
        glass = new MeshStandardMaterial({ roughness: 0.15, metalness: 0.1 });
        group.add(new Mesh(this.geo(gb.build()), glass));
      }
      top = STOREY + 0.7;
    }
    this.world.add(group);
    const sign = document.createElement('div');
    sign.className = 'nb-sign';
    sign.dataset.slot = s.id;
    sign.dataset.kind = kind;
    this.labels.appendChild(sign);
    this.gfx.set(s.id, { plot: p, group, edge, fill, glass, top, sign, signKey: '' });
  }

  private geo<T extends BufferGeometry>(g: T): T {
    this.geos.push(g);
    return g;
  }

  /** Colours that follow hover, selection, lights and the hour (no geometry change). */
  private restyle() {
    const t = this.theme;
    const night = t.name === 'dark';
    const ink: RGB = night ? lin('#f3ead8') : lin('#ffffff');
    for (const s of this.state.slots) {
      const g = this.gfx.get(s.id);
      if (!g) continue;
      const kind = slotKind(s);
      const live = !!s.workspace_id && s.workspace_id === this.state.live;
      const hot = this.hover === s.id || this.state.selected === s.id;
      const sale = kind === 'sale';
      const base = sale ? lin(night ? GROUND_NIGHT : GROUND) : lin(night ? '#9d9484' : '#ddd3c2');
      g.fill.color.copy(color(hot ? mix(base, [1, 1, 1], 0.18) : sale ? mix(base, [1, 1, 1], 0.06) : base));
      g.edge.color.copy(color(hot ? t.amber : sale ? mix(ink, base, 0.25) : ink));
      g.edge.opacity = hot ? 1 : sale ? 0.55 : 0.85;
      const working = live ? this.state.working : (this.state.away.get(s.workspace_id ?? '') ?? 0);
      const lights = lightsOf(s, live, working);
      if (g.glass) {
        if (lights === 'off') {
          g.glass.color.copy(color(night ? lin('#2a3046') : lin(MAT.glass)));
          g.glass.emissive.setRGB(0, 0, 0);
        } else {
          const k = (lights === 'busy' ? 1.6 : 0.9) * (night ? 1.3 : 0.7);
          g.glass.color.copy(color(lin('#ffe2a8')));
          g.glass.emissive.copy(color(t.lamp)).multiplyScalar(k);
        }
      }
      const sign = signOf(live ? { ...s, crew: this.state.crew } : s, live, working);
      const key = `${sign.title}|${sign.sub}|${sign.state}|${hot}`;
      if (key !== g.signKey) {
        g.signKey = key;
        g.sign.replaceChildren();
        const title = document.createElement('span');
        title.className = 'nb-title';
        title.textContent = sign.title;
        g.sign.appendChild(title);
        if (sign.sub) {
          const sub = document.createElement('span');
          sub.className = 'nb-sub';
          if (sign.state) {
            const dot = document.createElement('i');
            dot.className = 'nb-dot';
            sub.appendChild(dot);
          }
          sub.appendChild(document.createTextNode(sign.sub));
          g.sign.appendChild(sub);
        }
        g.sign.dataset.state = sign.state ?? '';
        g.sign.classList.toggle('hot', hot);
        g.sign.classList.toggle('busy', lights === 'busy');
      }
    }
  }

  private placeKey() {
    const [x, y, z] = this.theme.keyDir;
    const len = Math.hypot(x, y, z);
    const S = 160;
    const c = this.key.target.position;
    this.key.position.set(c.x + (x / len) * S, (y / len) * S, c.z + (z / len) * S);
  }

  private applyTheme() {
    const t = this.theme;
    this.renderer.toneMappingExposure = t.exposure;
    this.renderer.setClearColor(new Color(t.name === 'dark' ? 0x1a1d1f : 0xe9e4d6), 1);
    this.container.dataset.theme = t.name;
    this.hemi.color.setRGB(...t.hemiSky);
    this.hemi.groundColor.setRGB(...t.hemiGround);
    this.hemi.intensity = t.hemiIntensity;
    this.key.color.setRGB(...t.key);
    this.key.intensity = t.keyIntensity;
    this.key.shadow.intensity = t.shadowStrength;
    this.groundMat.color.copy(color(lin(t.name === 'dark' ? GROUND_NIGHT : GROUND)));
    this.town?.setNight(t.name === 'dark', t.lamp);
    this.placeKey();
    this.restyle();
    this.renderer.shadowMap.needsUpdate = true;
    this.wake();
  }

  // ───────────── frames ─────────────

  private resize() {
    const r = this.container.getBoundingClientRect();
    const w = Math.max(1, Math.round(r.width));
    const h = Math.max(1, Math.round(r.height));
    if (w === this.width && h === this.height) return;
    this.width = w;
    this.height = h;
    this.renderer.setSize(w, h, false);
    this.renderer.domElement.style.width = '100%';
    this.renderer.domElement.style.height = '100%';
    this.rig.setAspect(w / h);
    this.rig.setFreeArea(w, h, this.insets);
    if (!this.userMoved && this.plots.length) this.overview(true);
    this.wake();
  }

  private step(_now: number, dt: number): StepResult {
    const held = this.held.size > 0;
    if (held) {
      const v = (this.heldFast ? 2200 : 900) * Math.min(dt, 0.1);
      const h = this.held;
      this.rig.pan(((h.has('a') ? 1 : 0) - (h.has('d') ? 1 : 0)) * v, ((h.has('w') ? 1 : 0) - (h.has('s') ? 1 : 0)) * v, this.height);
    }
    const moving = this.rig.update(dt) || held;
    this.renderer.render(this.scene, this.rig.camera);
    this.placeSigns();
    return { animating: moving, wakeAt: Infinity };
  }

  /** Signs ride above their plot (crisp: whole pixels), hidden when behind the camera or off screen. */
  private placeSigns() {
    for (const g of this.gfx.values()) {
      this.v.set(g.plot.cx, g.top, g.plot.cz).project(this.rig.camera);
      const x = Math.round((this.v.x * 0.5 + 0.5) * this.width);
      const y = Math.round((-this.v.y * 0.5 + 0.5) * this.height);
      const off = this.v.z > 1 || x < -200 || x > this.width + 200 || y < -60 || y > this.height + 60;
      g.sign.style.visibility = off ? 'hidden' : '';
      if (!off) g.sign.style.transform = `translate(${x}px, ${y}px) translate(-50%, -100%)`;
    }
  }

  // ───────────── input ─────────────

  private slotAtScreen(clientX: number, clientY: number): string | null {
    const r = this.renderer.domElement.getBoundingClientRect();
    const g = this.rig.groundAt(((clientX - r.left) / r.width) * 2 - 1, -((clientY - r.top) / r.height) * 2 + 1);
    return g ? plotAt(this.plots, g.x, g.z) : null;
  }

  private setHover(id: string | null) {
    if (id === this.hover) return;
    this.hover = id;
    this.container.style.cursor = id ? 'pointer' : '';
    this.opts.onhover?.(id);
    this.restyle();
    this.wake();
  }

  private listen() {
    const el = this.container;
    const canvas = this.renderer.domElement;
    let down: { x: number; y: number; button: number; moved: boolean; id: number; orbit: boolean } | null = null;
    let last = { x: 0, y: 0 };
    const local = (e: { clientX: number; clientY: number }) => {
      const r = el.getBoundingClientRect();
      return { x: e.clientX - r.left, y: e.clientY - r.top };
    };
    const onDown = (e: PointerEvent) => {
      down = { x: e.clientX, y: e.clientY, button: e.button, moved: false, id: e.pointerId, orbit: e.button === 2 || e.button === 1 || e.altKey || e.shiftKey };
      last = { x: e.clientX, y: e.clientY };
      canvas.setPointerCapture(e.pointerId);
      el.focus({ preventScroll: true });
    };
    const onMove = (e: PointerEvent) => {
      if (down) {
        const dx = e.clientX - last.x;
        const dy = e.clientY - last.y;
        last = { x: e.clientX, y: e.clientY };
        if (Math.hypot(e.clientX - down.x, e.clientY - down.y) > 4) down.moved = true;
        if (!down.moved) return;
        if (down.orbit) this.rig.orbit(dx, dy);
        else this.rig.pan(dx, dy, this.height);
        this.userMoved = true;
        this.wake();
        return;
      }
      this.setHover(this.slotAtScreen(e.clientX, e.clientY));
    };
    const onUp = (e: PointerEvent) => {
      if (!down) return;
      const click = !down.moved && down.button === 0;
      canvas.releasePointerCapture?.(down.id);
      down = null;
      if (click) this.opts.onpick(this.slotAtScreen(e.clientX, e.clientY), local(e));
    };
    const onLeave = () => this.setHover(null);
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = canvas.getBoundingClientRect();
      const nx = ((e.clientX - r.left) / r.width) * 2 - 1;
      const ny = -((e.clientY - r.top) / r.height) * 2 + 1;
      const scale = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
      // As in the office: pinch (ctrl), ⌘ + wheel or a mouse wheel zooms; two-finger scroll pans.
      const mouseWheel = e.deltaMode !== 0 || (e.deltaX === 0 && Math.abs(e.deltaY) >= 50 && Number.isInteger(e.deltaY));
      if (e.ctrlKey || e.metaKey || mouseWheel) this.rig.zoom(Math.exp(-e.deltaY * scale * (e.ctrlKey ? 3 : 1) * 0.0018), this.rig.groundAt(nx, ny));
      else this.rig.pan(-e.deltaX * scale, -e.deltaY * scale, this.height);
      this.userMoved = true;
      this.wake();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if ((e.target as HTMLElement)?.closest?.('input, textarea, button, [role="dialog"]')) return;
      const k = e.key.toLowerCase();
      if (k === 'w' || k === 'a' || k === 's' || k === 'd') {
        e.preventDefault();
        this.held.add(k);
        this.heldFast = e.shiftKey;
        this.userMoved = true;
        this.wake();
        return;
      }
      const step = e.shiftKey ? 180 : 70;
      let handled = true;
      switch (e.key) {
        case 'ArrowLeft':
          this.rig.pan(step, 0, this.height);
          break;
        case 'ArrowRight':
          this.rig.pan(-step, 0, this.height);
          break;
        case 'ArrowUp':
          this.rig.pan(0, step, this.height);
          break;
        case 'ArrowDown':
          this.rig.pan(0, -step, this.height);
          break;
        case '+':
        case '=':
          this.rig.zoom(1.25);
          break;
        case '-':
        case '_':
          this.rig.zoom(0.8);
          break;
        case 'q':
        case 'Q':
          this.rig.turn(-1);
          break;
        case 'e':
        case 'E':
          this.rig.turn(1);
          break;
        case '0':
        case 'Home':
          this.overview();
          break;
        default:
          handled = false;
      }
      if (handled) {
        e.preventDefault();
        this.wake();
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      this.held.delete(e.key.toLowerCase());
      this.heldFast = e.shiftKey;
    };
    const onBlur = () => this.held.clear();
    const onContext = (e: Event) => e.preventDefault();
    const onSign = (e: MouseEvent) => {
      const t = (e.target as HTMLElement).closest?.('.nb-sign') as HTMLElement | null;
      if (t?.dataset.slot) this.opts.onpick(t.dataset.slot, local(e));
    };
    const onSignHover = (e: MouseEvent) => {
      const t = (e.target as HTMLElement).closest?.('.nb-sign') as HTMLElement | null;
      if (t?.dataset.slot) this.setHover(t.dataset.slot);
    };
    const onVis = () => this.loop.setVisible(document.visibilityState !== 'hidden');
    canvas.addEventListener('pointerdown', onDown);
    canvas.addEventListener('pointermove', onMove);
    canvas.addEventListener('pointerup', onUp);
    canvas.addEventListener('pointercancel', onUp);
    canvas.addEventListener('pointerleave', onLeave);
    canvas.addEventListener('wheel', onWheel, { passive: false });
    canvas.addEventListener('contextmenu', onContext);
    this.labels.addEventListener('click', onSign);
    this.labels.addEventListener('mouseover', onSignHover);
    el.addEventListener('keydown', onKey);
    el.addEventListener('keyup', onKeyUp);
    el.addEventListener('blur', onBlur);
    document.addEventListener('visibilitychange', onVis);
    this.cleanups.push(() => {
      canvas.removeEventListener('pointerdown', onDown);
      canvas.removeEventListener('pointermove', onMove);
      canvas.removeEventListener('pointerup', onUp);
      canvas.removeEventListener('pointercancel', onUp);
      canvas.removeEventListener('pointerleave', onLeave);
      canvas.removeEventListener('wheel', onWheel);
      canvas.removeEventListener('contextmenu', onContext);
      this.labels.removeEventListener('click', onSign);
      this.labels.removeEventListener('mouseover', onSignHover);
      el.removeEventListener('keydown', onKey);
      el.removeEventListener('keyup', onKeyUp);
      el.removeEventListener('blur', onBlur);
      document.removeEventListener('visibilitychange', onVis);
    });
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    clearInterval(this.minuteTimer);
    this.loop.dispose();
    this.ro.disconnect();
    for (const f of this.cleanups) f();
    for (const g of this.gfx.values()) {
      g.edge.dispose();
      g.fill.dispose();
      g.glass?.dispose();
    }
    for (const geo of this.geos) geo.dispose();
    this.town?.dispose();
    this.ground.geometry.dispose();
    this.groundMat.dispose();
    this.matte.dispose();
    this.labels.remove();
    this.renderer.dispose();
    // Release the GL context now (browsers cap live contexts; dispose() alone keeps it).
    this.renderer.forceContextLoss();
    this.renderer.domElement.remove();
  }
}
