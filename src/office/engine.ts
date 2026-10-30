// OfficeEngine: the framework-free 3D office. Owns the renderer, scene, camera, input, the
// event model, the behaviour layer and the DOM overlay. The Svelte wrapper (OfficeView.svelte)
// feeds it props and events; everything else happens here.
//
// Render on demand: a frame is drawn only when something moves (a walk, a pose blend, the
// camera, an effect), on a stepped loop tick (typing, LEDs), or when the model changes. When
// the office is still there are no animation frames and no timers at all.

import {
  AdditiveBlending,
  Color,
  DirectionalLight,
  Group,
  HemisphereLight,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  NeutralToneMapping,
  OctahedronGeometry,
  PCFShadowMap,
  PlaneGeometry,
  Scene,
  Vector3,
  WebGLRenderer,
  type Texture,
} from 'three';
import type { Agent } from '../lib/gen/Agent';
import type { Approval } from '../lib/gen/Approval';
import type { Artifact } from '../lib/gen/Artifact';
import type { EventRow } from '../lib/gen/EventRow';
import type { RepoFile } from '../lib/gen/RepoFile';
import type { Task } from '../lib/gen/Task';
import { aStar, smooth } from './astar';
import { BreakRoom } from './breakroom';
import { activityFor, plumbobOf, type RestActivity } from './sims';
import { breakPreference, reactionTo, traitsOf } from './traits';
import { DWELL_MS, Dwell, Errands, breakIntent, deriveIntent, errandAllowedFor, idleIntent, leisure, errandPlace, intentKey, isBlocked, placeKey, type Errand, type Intent, type Place } from './behavior';
import type { Spot } from './breakroom';
import { spotsOf, USE_LABEL, usesOf } from './use';
import { CameraRig, DEFAULT_PITCH, DEFAULT_YAW } from './camera';
import { BuildingGfx } from './gfx/building';
import { DecorLayer, model } from './gfx/decor';
import { BUILT_LABEL, builtinPieces, freeToUse, modeled, stationActions } from './gfx/builtins';
import { applyDecor, blocker, BY_ID, onTop, snap, surfaceAt, type Piece } from './buy';
import type { Placed } from '../lib/gen/Placed';
import type { Provider } from '../lib/gen/Provider';
import { Character, lookFor, PropKit } from './gfx/character';
import { CONFETTI, Confetti, FloorRing, FxSystem, Glow, PaperPlane, Puff, Sparkles } from './gfx/fx';
import { GamesCorner, type GamePlayers } from './gfx/games';
import { Archive, DeskFrames, DeskScreens, MailRoom, ServerRoom, TaskBoard, Whiteboard, type CardWhere, type TestState, type TodoList } from './gfx/props';
import { canvas as makeCanvas, emoteTexture, glowTexture, ringTexture, tex, type EmoteKind } from './gfx/textures';
import { cellCenter, cellOf, COLS, D, officeLayout, passableFn, ROWS, W, type Desk, type OfficeLayout, type Slot, type StationId } from './layout';
import { emptyModel, type AgentState, type Fx, type OfficeModel, type TaskCard, type Tone, type WorldSelection } from './model';
import { chipFor } from './bubble';
import { OUTCOME_MS, sentence, type Sentence } from './narrate';
import { identity, lin, THEMES, type OfficeTheme, type ThemeName } from './palette';
import { reduce, syncAgents, syncTasks } from './reducer';
import { browserHost, FrameLoop, type StepResult } from './scheduler';
import { Overlay, type LabelSpec } from './overlay';
import { clean, hash } from './text';

export type { WorldSelection };

/** External narration (the app's crew-strip sentence), when provided. */
export type NarrateFn = (agentId: string) => { parts: { t: string; code?: boolean }[]; tone?: string } | null;

export interface TickerItem {
  id: number;
  text: string;
  code: string | null;
  tone: Tone;
  agentId: string | null;
  color: string | null;
  at: number;
}

export interface AgentListItem {
  id: string;
  name: string;
  role: string;
  provider: string;
  sentence: string;
  tone: Tone;
  color: string;
}

export interface EngineOptions {
  theme: ThemeName;
  reducedMotion: boolean;
  backdrop: boolean;
  onselect: (sel: WorldSelection | null) => void;
  onhover: (sel: WorldSelection | null) => void;
  onticker?: (items: TickerItem[]) => void;
  onagents?: (items: AgentListItem[]) => void;
  /** Announce something important right away (approvals) to assistive tech. */
  onannounce?: (text: string) => void;
  /** A piece of furniture was clicked (its click menu), or the click went elsewhere (null). */
  onfurniture?: (f: FurnitureMenu | null) => void;
  artifactUrl?: (a: Artifact) => string | null;
  narrate?: NarrateFn;
}

const HELLO_MS = 1800;
/** How long a speech bubble stays up. */
const SAY_MS = 4500;
const OX = -W / 2;
const OZ = -D / 2;

/** A clicked piece of furniture and what it can be used for (The Sims' click menu). */
export interface FurnitureMenu {
  label: string;
  actions: { label: string; intent: Intent }[];
  /** Where to show the menu (container pixels). */
  x: number;
  y: number;
}

/** Build/Buy: the piece following the cursor. */
interface Carry {
  item: string;
  /** A piece already in the office, being moved; null for a new one. */
  uid: string | null;
  rot: number;
  /** Where it would go (floor plan), once the cursor is over the floor. */
  at: { x: number; y: number; z: number } | null;
  /** Why it can't go there, or null. */
  why: string | null;
  onplace: (p: Placed) => void;
  onsell: (uid: string) => void;
  onstate: (why: string | null) => void;
  onend: () => void;
}

interface Actor {
  c: Character;
  agentId: string;
  desk: Desk | null;
  /** Station slot this actor holds ("station:index"), released when it leaves. */
  slot: string | null;
  /** The place key it is walking to / standing at. */
  placeKey: string | null;
  shown: Intent | null;
  leaving: boolean;
  bornAt: number;
  hello: boolean;
  /** The last check result it reacted to (traits.ts). */
  cheerAt?: number;
  /** The last promotion it danced for. */
  promotedAt?: number | null;
}

/** Where ghosts float in the memory archive (layout x, z, facing). */
const GHOST_SPOTS: [number, number, number][] = [
  [25.9, 22.6, 0],
  [22.7, 19.6, 0.5],
  [26.4, 19.7, -0.4],
];

interface Intern {
  c: Character;
  parent: string;
  born: number;
  leaving: number | null;
}

export class OfficeEngine {
  private container: HTMLElement;
  private renderer: WebGLRenderer;
  private scene = new Scene();
  private world = new Group();
  private rig: CameraRig;
  private loop: FrameLoop;
  private overlay: Overlay;
  private theme: OfficeTheme;
  private opts: EngineOptions;
  private L: OfficeLayout;
  private passable: (x: number, y: number) => boolean;
  private hemi = new HemisphereLight();
  private key = new DirectionalLight();
  private building: BuildingGfx;
  private screens: DeskScreens;
  private board: TaskBoard;
  private whiteboard: Whiteboard;
  private server: ServerRoom;
  private games: GamesCorner;
  private gazeBufs = new Map<string, Vector3>();
  private breakRoom = new BreakRoom({ prefer: leisure.prefer });
  /** The Sims' plumbob over the selected agent, coloured by its state (sims.ts). */
  private plumbob = new Mesh(new OctahedronGeometry(0.11, 0).scale(1, 1.7, 1), new MeshStandardMaterial({ roughness: 0.25, metalness: 0.1, transparent: true, opacity: 0.92 }));
  /** What an agent last said (a chat message), shown as a speech bubble for a few seconds. */
  private said = new Map<string, { text: string; at: number }>();
  /** An agent's needs (sims.ts), so a tired agent goes for coffee on its break. */
  private needsOf: ((id: string) => { energy: number | null; morale: number } | null) | null = null;
  /** What each agent is doing, so its energy and morale move with it (sims.ts). */
  private onActivity: ((id: string, now: number, act: RestActivity) => void) | null = null;
  private mail: MailRoom;
  private archive: Archive;
  private frames: DeskFrames;
  private fx = new FxSystem();
  private props = new PropKit();
  private actorsGroup = new Group();
  private actors = new Map<string, Actor>();
  private interns = new Map<string, Intern>();
  /** Agents let go, haunting the memory archive (see-through, floating). */
  private ghosts: Character[] = [];
  private decor = new DecorLayer();
  /** The office's own furniture, as models (gfx/builtins.ts). */
  private builtins = new DecorLayer();
  private builtinList: Piece[] = [];
  /** Agents sent to use something from a click menu (until real work calls them back). */
  private forced = new Map<string, Intent>();
  /** Each agent's own intent at the last sync (to tell whether it is free for a break). */
  private lastRaw = new Map<string, Intent>();
  private pieces: Piece[] = [];
  /** Build/Buy mode: clicks pick up furniture; `carry` is the piece following the cursor. */
  private build: { onpick: (uid: string) => void; carry: Carry | null } | null = null;
  private ghostKey = '';
  private emotes = new Map<EmoteKind, Texture>();
  private glowTex: Texture;
  private ringTex: Texture;
  private selRing: Mesh;
  private hovRing: Mesh;

  private model: OfficeModel = emptyModel();
  private dwell = new Dwell<Intent>(DWELL_MS, intentKey);
  private errands: Errands;
  private deskOf = new Map<string, number>();
  private slotOwner = new Map<string, string>();
  private cardOverride = new Map<string, { kind: 'board' } | { kind: 'held'; agentId: string; untilDesk: boolean; at: number }>();
  private agentsProp: Agent[] = [];
  private selection: WorldSelection | null = null;
  private hoverProp: WorldSelection | null = null;
  private hoverLocal: WorldSelection | null = null;
  private followOn = false;
  private ticker: TickerItem[] = [];
  private intentWakeAt = Infinity;
  private lost = false;
  private disposed = false;
  private shadowDirty = true;
  private width = 1;
  private height = 1;
  private userMoved = false;
  private mounted = false;
  private insets = { top: 0, right: 0, bottom: 0, left: 0 };
  private agentListSig = '';
  private files = 0;
  private ro: ResizeObserver;
  private io: IntersectionObserver | null = null;
  private cleanups: (() => void)[] = [];
  private v = new Vector3();
  private announced = new Set<string>();
  readonly stats = { frames: 0, renders: 0 };

  constructor(container: HTMLElement, opts: EngineOptions) {
    this.container = container;
    this.opts = opts;
    this.theme = THEMES[opts.theme];
    this.L = officeLayout();
    this.passable = passableFn(this.L);
    this.renderer = new WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance', preserveDrawingBuffer: false });
    this.renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
    this.renderer.toneMapping = NeutralToneMapping;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = PCFShadowMap;
    this.renderer.shadowMap.autoUpdate = false;
    const canvas = this.renderer.domElement;
    canvas.className = 'of-canvas';
    canvas.setAttribute('aria-hidden', 'true');
    container.appendChild(canvas);
    this.overlay = new Overlay(container);

    this.rig = new CameraRig(1);
    this.rig.boundsX = W / 2 + 1;
    this.rig.boundsZ = D / 2 + 1;
    this.key.castShadow = true;
    this.key.shadow.mapSize.set(2048, 2048);
    this.key.shadow.radius = 4;
    this.key.shadow.bias = -0.0005;
    this.key.shadow.normalBias = 0.025;
    this.scene.add(this.hemi, this.key, this.key.target);

    this.world.position.set(OX, 0, OZ);
    this.scene.add(this.world);
    this.building = new BuildingGfx(this.L);
    this.screens = new DeskScreens(this.L.desks);
    this.board = new TaskBoard(this.L);
    this.board.handOf = (id) => this.actors.get(id)?.c.hand ?? null;
    this.whiteboard = new Whiteboard(this.L);
    this.server = new ServerRoom(this.L);
    this.games = new GamesCorner(this.L, { x: OX, z: OZ }, makeCanvas, tex);
    this.mail = new MailRoom(this.L);
    this.archive = new Archive(this.L);
    this.frames = new DeskFrames(() => this.wake());
    this.glowTex = glowTexture();
    this.ringTex = ringTexture();
    const ringGeo = new PlaneGeometry(1, 1).rotateX(-Math.PI / 2);
    this.selRing = new Mesh(ringGeo, new MeshBasicMaterial({ map: this.ringTex, transparent: true, depthWrite: false, blending: AdditiveBlending, toneMapped: false }));
    this.hovRing = new Mesh(ringGeo, new MeshBasicMaterial({ map: this.ringTex, transparent: true, depthWrite: false, opacity: 0.55, blending: AdditiveBlending, toneMapped: false }));
    this.selRing.visible = this.hovRing.visible = false;
    this.selRing.renderOrder = this.hovRing.renderOrder = 3;
    this.world.add(this.building.group, this.screens.group, this.board.group, this.whiteboard.mesh, this.server.group, this.games.group, this.mail.group, this.archive.group, this.frames.group, this.builtins.group, this.decor.group, this.fx.group, this.selRing, this.hovRing);
    void builtinPieces(this.L)
      .then((p) => this.builtins.set(p))
      .then((list) => {
        this.builtinList = list ?? [];
        this.regrid();
        this.shadowDirty = true;
        this.wake();
      });
    // Characters live in world coordinates (the office frame is offset to center it).
    this.scene.add(this.actorsGroup);
    this.plumbob.visible = false;
    this.plumbob.renderOrder = 4;
    this.scene.add(this.plumbob);

    this.errands = new Errands((agentId, e, phase) => this.onErrand(agentId, e, phase));
    this.loop = new FrameLoop((now, dt) => this.step(now, dt), browserHost());
    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(container);
    this.resize();
    this.listen();
    this.applyTheme();
    this.overview(true);
    this.wake();
    const w = window as unknown as { __OFFICE_DEBUG?: boolean; __officeEngine?: OfficeEngine };
    if (w.__OFFICE_DEBUG) w.__officeEngine = this;
  }

  // ───────────── public API ─────────────

  setFiles(files: RepoFile[] | undefined) {
    this.files = files?.length ?? 0;
    this.wake();
  }

  setAgents(agents: Agent[]) {
    this.agentsProp = agents;
    syncAgents(this.model, agents, Date.now());
    this.assignDesks();
    this.sync();
  }

  setTasks(tasks: Task[]) {
    syncTasks(this.model, tasks);
    this.sync();
  }

  setApprovals(list: Approval[]) {
    for (const ap of list) {
      if (ap.status !== 'pending') {
        this.model.resolved.add(ap.id);
        const a = this.model.agents.get(ap.agent_id);
        if (a?.approval?.id === ap.id) a.approval = null;
        this.model.approvals.delete(ap.id);
      } else if (!this.model.resolved.has(ap.id)) {
        const a = this.model.agents.get(ap.agent_id);
        if (a && (!a.approval || a.approval.id === 'status')) {
          reduce(this.model, { id: 0, ts: ap.created_at || Date.now(), agent_id: ap.agent_id, task_id: ap.task_id, run_id: ap.run_id, kind: { type: 'approval_requested', approval: ap } }, { replay: true });
        }
      }
    }
    this.sync();
  }

  setArtifacts(list: Artifact[]) {
    for (const a of list) this.model.artifacts.set(a.id, a);
    this.placeFrames(false);
    this.wake();
  }

  setSelection(sel: WorldSelection | null) {
    const prev = this.selection;
    this.selection = sel && (sel.agentId || sel.taskId || sel.path) ? sel : null;
    if (JSON.stringify(prev) === JSON.stringify(this.selection)) return;
    if (!this.selection) {
      this.followOn = false;
      this.rig.follow = null;
      if (prev && !this.userMoved) this.overview(false);
    } else this.focusOn(this.selection);
    this.sync();
  }

  setHover(h: WorldSelection | null) {
    const prevTask = this.hoverProp?.taskId ?? null;
    this.hoverProp = h && (h.agentId || h.taskId || h.path) ? h : null;
    if ((this.hoverProp?.taskId ?? null) !== prevTask) this.surfacesDirty = true;
    this.wake();
  }

  setTheme(t: ThemeName) {
    if (this.theme.name === t) return;
    this.theme = THEMES[t];
    this.opts.theme = t;
    this.applyTheme();
  }

  setReducedMotion(r: boolean) {
    this.opts.reducedMotion = r;
    this.wake();
  }

  setBackdrop(b: boolean) {
    if (this.opts.backdrop === b) return;
    this.opts.backdrop = b;
    this.container.classList.toggle('of-backdrop', b);
    this.applyFreeArea();
    this.applyTheme();
    if (!this.selection) this.overview(false);
  }

  setInsets(i: { top?: number; right?: number; bottom?: number; left?: number } | undefined) {
    const next = { top: i?.top ?? 0, right: i?.right ?? 0, bottom: i?.bottom ?? 0, left: i?.left ?? 0 };
    if (JSON.stringify(next) === JSON.stringify(this.insets)) return;
    this.insets = next;
    this.applyFreeArea();
    if (!this.selection && !this.userMoved) this.overview(false);
    this.wake();
  }

  setNarrate(fn: NarrateFn | undefined) {
    this.opts.narrate = fn;
    this.wake();
  }

  setArtifactUrl(fn: ((a: Artifact) => string | null) | undefined) {
    this.opts.artifactUrl = fn;
    this.placeFrames(false);
  }

  /** Fold persisted history into the current state without animating. */
  replay(history: EventRow[]) {
    for (const ev of history) for (const f of reduce(this.model, ev, { replay: true })) if (f.t === 'ticker') this.pushTicker(f, false);
    // The roster (a snapshot newer than most of history) has the last word on status.
    syncAgents(this.model, this.agentsProp, Date.now());
    this.assignDesks();
    this.mounted = true;
    this.sync(true);
    this.placeFrames(false);
    this.emitTicker();
  }

  /** A live event. */
  push(ev: EventRow) {
    if (this.disposed) return;
    if (ev.id > 0 && ev.id <= this.model.lastEventId) return;
    const fxs = reduce(this.model, ev, { replay: false });
    if (ev.kind.type === 'agent_changed' || ev.kind.type === 'status_changed') this.assignDesks();
    if (ev.kind.type === 'chat' && this.model.agents.has(ev.kind.message.author)) {
      this.said.set(ev.kind.message.author, { text: clean(ev.kind.message.text, 72), at: performance.now() });
    }
    const now = performance.now();
    const wall = Date.now();
    for (const f of fxs) this.play(f, now, wall);
    this.sync();
    if (fxs.some((f) => f.t === 'ticker')) this.emitTicker();
  }

  /** The office's live view of an agent (for the Sims panel). */
  agentState(id: string): AgentState | null {
    return this.model.agents.get(id) ?? null;
  }

  /** Where an agent's head is on screen (px from the canvas' top-left), for the click menu. */
  screenOf(id: string): { x: number; y: number } | null {
    const act = this.actors.get(id);
    if (!act) return null;
    const v = act.c.headTop(new Vector3()).project(this.rig.camera);
    if (v.z > 1) return null;
    return { x: ((v.x + 1) / 2) * this.width, y: ((1 - v.y) / 2) * this.height };
  }

  /** Screen point of the front door (where job offers are pinned and new hires walk in). */
  doorScreen(): { x: number; y: number } | null {
    const d = this.L.door.inside;
    const v = new Vector3(d.x + OX, 2.2, d.z + OZ).project(this.rig.camera);
    if (v.z > 1 || Math.abs(v.x) > 1.1 || Math.abs(v.y) > 1.1) return null;
    return { x: ((v.x + 1) / 2) * this.width, y: ((1 - v.y) / 2) * this.height };
  }

  /** The memory archive on screen (where the ghosts are), or null when off screen. */
  archiveScreen(): { x: number; y: number } | null {
    const v = new Vector3(24.8 + OX, 2.3, 18.6 + OZ).project(this.rig.camera); // over its back cabinets
    if (v.z > 1 || Math.abs(v.x) > 1.1 || Math.abs(v.y) > 1.1) return null;
    return { x: ((v.x + 1) / 2) * this.width, y: ((1 - v.y) / 2) * this.height };
  }

  /** Build/Buy: the furniture placed in the office; agents walk around what blocks. */
  setDecor(placed: Placed[]) {
    void this.decor.set(placed).then((pieces) => {
      if (!pieces || this.disposed) return;
      this.pieces = pieces;
      this.regrid();
      this.shadowDirty = true;
      this.wake();
    });
  }

  /** Agents free for a break right now (between tasks). */
  freeAgents(): string[] {
    return [...this.lastRaw].filter(([, i]) => idleIntent(i)).map(([id]) => id);
  }

  /** Send an idle agent to use something (a click menu action); false when it is busy. */
  sendTo(agentId: string, intent: Intent): boolean {
    const raw = this.lastRaw.get(agentId);
    if (!raw || !idleIntent(raw)) return false;
    this.forced.set(agentId, intent);
    this.sync();
    return true;
  }

  private pieceByUid(uid: string): Piece | undefined {
    return this.pieces.find((p) => p.uid === uid) ?? this.builtinList.find((p) => p.uid === uid);
  }

  private usable(): Piece[] {
    return [...this.pieces, ...this.builtinList.filter((p) => freeToUse(this.L.items[+p.uid.split(':')[1]]))];
  }

  /** The walking grid with the furniture placed, seats on furniture kept reachable.
   *  ponytail: agents already walking keep their path (through a new piece); the next walk avoids it. */
  private regrid() {
    const seats = this.usable().flatMap((p) => usesOf(p.item).flatMap((use) => spotsOf(p, use).filter((s) => s.pose === 'sit')));
    applyDecor(this.L, this.pieces, seats);
  }

  /** What agents may use on their own during breaks: bought pieces, and a few built-in ones. */
  private spots(): Spot[] {
    const out: Spot[] = [];
    for (const p of this.usable())
      for (const use of usesOf(p.item)) {
        // A spot to stand in front must be on free floor (another piece may be there).
        const n = spotsOf(p, use).filter((s) => s.pose === 'sit' || this.passable(...cellOf(s.x, s.z))).length;
        if (n) out.push({ uid: p.uid, use, n });
      }
    return out;
  }

  /** The piece of furniture under a click, with its menu; null when there is none. */
  private furnitureAt(clientX: number, clientY: number): FurnitureMenu | null {
    const r = this.renderer.domElement.getBoundingClientRect();
    const where = { x: clientX - r.left, y: clientY - r.top };
    const ray = this.rayAt(clientX, clientY);
    const uid = this.decor.pick(ray) ?? this.builtins.pick(ray);
    if (uid?.startsWith('item:')) {
      const it = this.L.items[+uid.split(':')[1]];
      const p = this.pieceByUid(uid);
      const own = stationActions(this.L, it);
      const free = freeToUse(it) && p ? usesOf(p.item).map((use) => ({ label: USE_LABEL[use], intent: breakIntent({ game: 'piece', slot: 0, piece: uid, use }) })) : [];
      return { label: BUILT_LABEL[it.type] ?? (p && BY_ID.get(p.item)?.label) ?? 'Furniture', actions: [...own, ...free], ...where };
    }
    if (uid) {
      const p = this.pieceByUid(uid);
      if (!p) return null;
      const actions = usesOf(p.item).map((use) => ({ label: USE_LABEL[use], intent: breakIntent({ game: 'piece', slot: 0, piece: uid, use }) }));
      return { label: BY_ID.get(p.item)?.label ?? 'Furniture', actions, ...where };
    }
    // Code-built pieces with a use (foosball, pool): where the ray crosses their top (~0.7 up).
    const { origin: o, direction: d } = ray.ray;
    if (d.y >= 0) return null;
    const t = (0.7 - o.y) / d.y;
    const [x, z] = [o.x + d.x * t - OX, o.z + d.z * t - OZ];
    const it = this.L.items.find((i) => !modeled(i.type) && BUILT_LABEL[i.type] && Math.abs(x - i.x) < i.w / 2 + 0.1 && Math.abs(z - i.z) < i.d / 2 + 0.1);
    return it ? { label: BUILT_LABEL[it.type]!, actions: stationActions(this.L, it), ...where } : null;
  }

  /** Build/Buy mode on (`onpick`: a placed piece was clicked) or off. */
  setBuildMode(onpick: ((uid: string) => void) | null) {
    if (!onpick) this.endCarry();
    this.build = onpick ? { onpick, carry: null } : null;
  }

  /** Whether a piece can be moved or sold (nothing sits on it). */
  movable(uid: string): boolean {
    const p = this.pieces.find((x) => x.uid === uid);
    return !p || !onTop(this.pieces, p).length;
  }

  /**
   * Carry a piece around until a click puts it down (`onplace`): a new one, or one already in the
   * office (`uid`, hidden meanwhile). R turns it, Esc puts it back, Delete sells it.
   */
  carry(item: string, cb: Omit<Carry, 'item' | 'uid' | 'rot' | 'at' | 'why'>, uid: string | null = null) {
    const f = BY_ID.get(item);
    if (!this.build || !f) return;
    this.endCarry();
    const cur = uid ? this.pieces.find((p) => p.uid === uid) : undefined;
    this.build.carry = { item, uid, rot: cur?.rot ?? 0, at: null, why: null, ...cb };
    this.decor.hide(uid);
    void model(f).then(() => cur && this.moveCarry(cur.x, cur.z));
    this.wake();
  }

  rotateCarry() {
    const c = this.build?.carry;
    if (!c) return;
    c.rot += Math.PI / 2;
    if (c.at) this.moveCarry(c.at.x, c.at.z);
  }

  endCarry() {
    const c = this.build?.carry;
    if (!c) return;
    this.build!.carry = null;
    this.decor.hideGhost();
    this.decor.hide(null);
    c.onend();
    this.wake();
  }

  /** The carried piece to (x, z) on the floor plan: snapped, on a table if it's small. */
  private moveCarry(x: number, z: number) {
    const c = this.build?.carry;
    const f = c && BY_ID.get(c.item);
    if (!c || !f) return;
    void model(f).then(({ dims }) => {
      if (this.build?.carry !== c) return;
      const top = f.small ? surfaceAt(this.pieces, x, z, c.uid ?? undefined) : null;
      const at = top ? { x, z, rot: c.rot } : snap(x, z, c.rot, dims);
      const y = top ? top.y + top.dims.h : 0;
      const why = blocker(this.L, this.pieces, { uid: c.uid ?? '', item: c.item, x: at.x, y, z: at.z, rot: at.rot, dims });
      c.at = { x: at.x, y, z: at.z };
      c.why = why;
      c.onstate(why);
      void this.decor.showGhost(c.item, at.x, y, at.z, at.rot, !why).then(() => this.wake());
    });
  }

  private dropCarry() {
    const c = this.build?.carry;
    if (!c?.at || c.why) return;
    const p: Placed = { uid: c.uid ?? crypto.randomUUID(), item: c.item, x: c.at.x, y: c.at.y, z: c.at.z, rot: snap(0, 0, c.rot, { w: 1, d: 1, h: 1 }).rot };
    c.onplace(p);
    this.endCarry();
    this.setDecor([...this.pieces.filter((x) => x.uid !== p.uid), p]); // shown now; the core's answer follows
  }

  /** Agents let go (the core's ghosts): up to three float by the memory archive. */
  setGhosts(list: { id: string; color: number; seed: string; provider: Provider }[]) {
    const shown = list.slice(0, GHOST_SPOTS.length);
    const key = shown.map((g) => g.id).join();
    if (key === this.ghostKey) return;
    this.ghostKey = key;
    for (const c of this.ghosts) c.dispose();
    const now = performance.now();
    this.ghosts = shown.map((g, i) => {
      const c = new Character(`ghost:${g.id}`, lookFor(g.seed, g.color, 'worker', g.provider), { emotes: this.emote, props: this.props });
      const [x, z, face] = GHOST_SPOTS[i];
      c.teleport({ x: x + OX, z: z + OZ, face }, 'ghost', now, false);
      c.haunt(now);
      this.actorsGroup.add(c.root);
      return c;
    });
    this.wake();
  }

  setNeeds(fn: ((id: string) => { energy: number | null; morale: number } | null) | null) {
    this.needsOf = fn;
  }

  setActivity(fn: ((id: string, now: number, act: RestActivity) => void) | null) {
    this.onActivity = fn;
  }

  setWalls(up: boolean) {
    this.building.setWalls(up);
    this.shadowDirty = true;
    this.wake();
  }

  get frameCount() {
    return this.loop.stats.frames;
  }

  get loopActive() {
    return this.loop.active;
  }

  dispose() {
    this.disposed = true;
    this.loop.dispose();
    this.ro.disconnect();
    this.io?.disconnect();
    for (const c of this.cleanups) c();
    this.fx.clear();
    for (const a of this.actors.values()) a.c.dispose();
    for (const i of this.interns.values()) i.c.dispose();
    this.actors.clear();
    this.interns.clear();
    this.building.dispose();
    this.screens.dispose();
    this.board.dispose();
    this.whiteboard.dispose();
    this.server.dispose();
    this.games.dispose();
    this.mail.dispose();
    for (const c of this.ghosts) c.dispose();
    this.decor.dispose();
    this.builtins.dispose();
    applyDecor(this.L, []);
    this.archive.dispose();
    this.frames.dispose();
    this.props.dispose();
    for (const t of this.emotes.values()) t.dispose();
    this.glowTex.dispose();
    this.ringTex.dispose();
    this.selRing.geometry.dispose();
    (this.selRing.material as MeshBasicMaterial).dispose();
    (this.hovRing.material as MeshBasicMaterial).dispose();
    this.overlay.dispose();
    this.renderer.dispose();
    // Release the GL context now (browsers cap live contexts; dispose() alone keeps it).
    this.renderer.forceContextLoss();
    this.renderer.domElement.remove();
    const w = window as unknown as { __officeEngine?: OfficeEngine };
    if (w.__officeEngine === this) delete w.__officeEngine;
  }

  // ───────────── internals ─────────────

  wake = () => {
    this.loop.wake();
  };

  private emote = (k: EmoteKind): Texture => {
    let t = this.emotes.get(k);
    if (!t) {
      t = emoteTexture(k);
      this.emotes.set(k, t);
    }
    return t;
  };

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
    this.applyFreeArea();
    if (!this.selection && !this.userMoved) this.overview(true);
    this.wake();
  }

  private insetTop(): number {
    return this.insets.top || (this.opts.backdrop ? 70 : 0);
  }

  private freeInsets() {
    return { ...this.insets, top: this.insetTop() };
  }

  private applyFreeArea() {
    this.rig.setFreeArea(this.width, this.height, this.freeInsets());
  }

  private applyTheme() {
    const t = this.theme;
    const dim = this.opts.backdrop ? 0.78 : 1;
    this.renderer.toneMappingExposure = t.exposure * dim;
    this.scene.background = null;
    this.renderer.setClearColor(new Color(t.name === 'dark' ? 0x12131f : 0xf1eadf), 1);
    this.container.style.setProperty('--of-sky', t.sky);
    this.container.dataset.theme = t.name;
    this.hemi.color.setRGB(...t.hemiSky);
    this.hemi.groundColor.setRGB(...t.hemiGround);
    this.hemi.intensity = t.hemiIntensity;
    this.key.color.setRGB(...t.key);
    this.key.intensity = t.keyIntensity;
    this.key.shadow.intensity = t.shadowStrength;
    const [x, y, z] = t.keyDir;
    const len = Math.hypot(x, y, z);
    const S = 40;
    this.key.position.set((x / len) * S, (y / len) * S, (z / len) * S);
    this.key.target.position.set(0, 0, 0);
    const cam = this.key.shadow.camera;
    cam.left = -21;
    cam.right = 21;
    cam.top = 21;
    cam.bottom = -21;
    cam.near = 5;
    cam.far = 90;
    cam.updateProjectionMatrix();
    this.building.applyTheme(t);
    this.screens.applyTheme(t);
    this.server.applyTheme(t);
    this.archive.applyTheme(t);
    for (const a of this.actors.values()) this.paintDesk(a);
    this.renderer.domElement.style.background = 'transparent';
    this.shadowDirty = true;
    this.sync();
  }

  // ───────────── desks & places ─────────────

  /** Stable desks: Leads in the corner office (by arrival), everyone else by id hash. */
  private assignDesks() {
    const leads = [...this.model.agents.values()].filter((a) => a.role === 'lead');
    const leadDesks = this.L.desks.filter((d) => d.lead);
    const podDesks = this.L.desks.filter((d) => !d.lead);
    const taken = new Set(this.deskOf.values());
    for (const a of leads) {
      if (this.deskOf.has(a.id)) continue;
      const d = leadDesks.find((x) => !taken.has(x.index));
      if (d) {
        this.deskOf.set(a.id, d.index);
        taken.add(d.index);
      }
    }
    const others = [...this.model.agents.values()].filter((a) => !this.deskOf.has(a.id)).sort((x, y) => (x.id < y.id ? -1 : 1));
    for (const a of others) {
      const n = podDesks.length;
      // Fill pods front to back, but spread agents: preferred desk from the id, then probe.
      const start = hash(a.seed || a.id) % n;
      for (let k = 0; k < n; k++) {
        const d = podDesks[(start + k * 5) % n] ?? podDesks[(start + k) % n];
        if (!taken.has(d.index)) {
          this.deskOf.set(a.id, d.index);
          taken.add(d.index);
          break;
        }
      }
    }
  }

  private deskFor(id: string): Desk | null {
    const k = this.deskOf.get(id);
    return k == null ? null : (this.L.desks[k] ?? null);
  }

  /** Resolve a place to a concrete slot for an actor (allocating station slots). */
  private resolve(a: Actor, place: Place): { slot: Slot; key: string } | null {
    const desk = a.desk;
    const deskSlot = (stand: boolean) => (desk ? { slot: stand ? desk.stand : desk.seat, key: stand ? 'desk:stand' : 'desk' } : null);
    switch (place.kind) {
      case 'desk':
        return deskSlot(!!place.stand) ?? this.resolve(a, { kind: 'station', id: 'meeting.table' });
      case 'door':
        return { slot: this.L.door.outside, key: 'door' };
      case 'station':
        return this.claim(a, place.id, placeKey(place), place.slot) ?? deskSlot(false);
      case 'break':
        return deskSlot(false); // matched in sync(); unmatched breaks stay at the desk
      case 'piece': {
        const p = this.pieceByUid(place.uid);
        const slots = p ? spotsOf(p, place.use) : [];
        for (const k of [place.slot, ...slots.keys()]) {
          const id = `piece:${place.uid}:${k}`;
          const owner = this.slotOwner.get(id);
          if (!slots[k] || (owner && owner !== a.agentId)) continue;
          this.take(a, id);
          return { slot: slots[k], key: placeKey(place) };
        }
        return deskSlot(false);
      }
      case 'lead': {
        const ld = this.deskFor(place.leadId);
        if (!ld) return deskSlot(false);
        const li = this.L.desks.filter((d) => d.lead).indexOf(ld);
        const st = this.L.stations['lead.visit'];
        for (const k of [li * 2, li * 2 + 1]) {
          const id = `lead.visit:${k}`;
          const owner = this.slotOwner.get(id);
          if (!owner || owner === a.agentId) {
            this.take(a, id);
            return st.slots[k] ? { slot: st.slots[k], key: placeKey(place) } : deskSlot(false);
          }
        }
        return deskSlot(false);
      }
    }
  }

  private claim(a: Actor, station: StationId, key: string, want?: number): { slot: Slot; key: string } | null {
    const st = this.L.stations[station];
    if (want != null && st.slots[want]) {
      // A seat the break room gave it (its partner has the other one).
      const id = `${station}:${want}`;
      const owner = this.slotOwner.get(id);
      if (!owner || owner === a.agentId) {
        this.take(a, id);
        return { slot: st.slots[want], key };
      }
    }
    if (a.slot?.startsWith(station + ':')) {
      const k = +a.slot.slice(station.length + 1);
      return { slot: st.slots[k], key };
    }
    // Closest free slot.
    let best = -1;
    let bd = Infinity;
    const p = a.c.position;
    st.slots.forEach((s, k) => {
      const owner = this.slotOwner.get(`${station}:${k}`);
      if (owner && owner !== a.agentId) return;
      const d = Math.hypot(s.x + OX - p.x, s.z + OZ - p.z);
      if (d < bd) {
        bd = d;
        best = k;
      }
    });
    if (best < 0) return null;
    this.take(a, `${station}:${best}`);
    return { slot: st.slots[best], key };
  }

  private take(a: Actor, slotId: string) {
    if (a.slot && a.slot !== slotId) this.slotOwner.delete(a.slot);
    a.slot = slotId;
    this.slotOwner.set(slotId, a.agentId);
  }

  private release(a: Actor) {
    if (a.slot) this.slotOwner.delete(a.slot);
    a.slot = null;
  }

  // ───────────── the director: model → characters ─────────────

  private sync(instant = false) {
    if (!this.mounted) {
      this.wake();
      return;
    }
    const now = performance.now();
    const wall = Date.now();
    let wakeAt = Infinity;
    const seen = new Set<string>();
    // Pass 1: every agent's intent. Breaks are matched together (pairs for pool and foosball).
    const pending: { a: AgentState; act: Actor; intent: Intent }[] = [];
    for (const a of this.model.agents.values()) {
      if (!a.present) continue;
      seen.add(a.id);
      let act = this.actors.get(a.id);
      if (!act) act = this.spawn(a, instant, now);
      else if (act.leaving) {
        act.leaving = false;
        act.c.fadeTo(1, now, 200);
      }
      act.desk = this.deskFor(a.id);
      if (String(act.c.look.shirt) !== String(identity(a.color, 'light').shirt) || act.c.look.role !== a.role) {
        // Identity or role changed: rebuild the character in place.
        const pos = act.c.position.clone();
        const rot = act.c.root.rotation.y;
        act.c.dispose();
        act.c = this.makeCharacter(a);
        act.c.position.copy(pos);
        act.c.root.rotation.y = rot;
        act.placeKey = null;
      }
      const allowed = !this.opts.reducedMotion && errandAllowedFor(a);
      const errand = this.errands.current(a.id, wall, allowed);
      const { intent, nextAt } = deriveIntent(a, this.model, wall, errand);
      wakeAt = Math.min(wakeAt, nextAt);
      this.lastRaw.set(a.id, intent);
      // Sent to use something: until real work calls it back.
      if (this.forced.has(a.id) && !idleIntent(intent)) this.forced.delete(a.id);
      pending.push({ a, act, intent: this.forced.get(a.id) ?? intent });
    }
    const onBreak = pending.filter((p) => p.intent.place.kind === 'break').sort((x, y) => x.a.lastEventAt - y.a.lastEventAt || (x.a.id < y.a.id ? -1 : 1));
    const seats = this.breakRoom.update(
      onBreak.map((p) => ({ id: p.a.id, prefer: breakPreference(traitsOf(p.a.id), this.needsOf?.(p.a.id) ?? null) })),
      wall,
      this.spots(),
    );
    wakeAt = Math.min(wakeAt, this.breakRoom.nextAt());
    // Pass 2: show them.
    for (const { a, act, intent: raw } of pending) {
      const intent = raw.place.kind === 'break' ? breakIntent(seats.get(a.id) ?? null) : raw;
      this.onActivity?.(a.id, wall, activityFor(intent.place.kind === 'station' ? intent.place.id : null, intent.pose));
      // Personality: its own reaction to a fresh check result (a thump, a fist pump…).
      const ch = a.cheer;
      if (ch && ch.at !== act.cheerAt) {
        act.cheerAt = ch.at;
        const r = reactionTo(traitsOf(a.id), ch.ok);
        if (r && !instant && !this.opts.reducedMotion && wall - ch.at < 5000) act.c.react(r, now);
      }
      if (a.promotedAt !== (act.promotedAt ?? null)) {
        act.promotedAt = a.promotedAt;
        if (a.promotedAt && !instant && !this.opts.reducedMotion && wall - a.promotedAt < 5000) act.c.react('dance', now);
      }
      const shown = instant ? this.dwell.request(a.id, intent, wall, true) : this.dwell.request(a.id, intent, wall);
      this.apply(act, a, intent, shown, now, instant);
    }
    for (const [id, act] of this.actors) {
      if (seen.has(id) || act.leaving) continue;
      this.leave(act, now);
    }
    this.syncInterns(now, instant);
    this.syncSurfaces(now, instant);
    this.intentWakeAt = wakeAt;
    this.notifyAgents();
    this.wake();
  }

  private makeCharacter(a: AgentState): Character {
    const c = new Character(a.id, lookFor(a.seed, a.color, a.role, a.provider), { emotes: this.emote, props: this.props });
    this.actorsGroup.add(c.root);
    return c;
  }

  private spawn(a: AgentState, instant: boolean, now: number): Actor {
    const c = this.makeCharacter(a);
    const act: Actor = { c, agentId: a.id, desk: this.deskFor(a.id), slot: null, placeKey: null, shown: null, leaving: false, bornAt: now, hello: !instant };
    this.actors.set(a.id, act);
    if (!instant) {
      // Walk in through the front door.
      const d = this.L.door.outside;
      c.teleport({ x: d.x + OX, z: d.z + OZ, face: Math.PI }, 'door', now, false);
      c.opacity = 0;
      c.fadeTo(1, now, 350);
      if (!this.opts.reducedMotion) this.fx.add(new FloorRing(new Vector3(d.x, 0, d.z - 0.4), identity(a.color, this.theme.name).glow, now, this.ringTex));
      // A wave hello at the door (KayKit clip).
      if (!this.opts.reducedMotion) c.react('wave', now + 250);
    }
    return act;
  }

  private leave(act: Actor, now: number) {
    act.leaving = true;
    this.release(act);
    this.errands.clear(act.agentId);
    this.dwell.delete(act.agentId);
    const d = this.L.door.outside;
    act.c.setEmote(null, now);
    act.c.setProp(null);
    act.placeKey = 'door';
    if (this.opts.reducedMotion) {
      act.c.fadeTo(0, now, 300);
      return;
    }
    this.walkTo(act, { slot: d, key: 'door' }, now);
  }

  private apply(act: Actor, a: AgentState, raw: Intent, shown: Intent, now: number, instant: boolean) {
    const c = act.c;
    act.shown = shown;
    // Truth first (≤ 250 ms): emote, screen, lamp follow the raw intent.
    c.setEmote(raw.emote, now);
    this.paintDesk(act, raw);
    // Place: resolve and walk if it changed.
    let res = this.resolve(act, shown.place);
    if (!res) res = act.desk ? { slot: act.desk.seat, key: 'desk' } : null;
    if (!res) return;
    if (shown.place.kind !== 'station' && shown.place.kind !== 'lead') this.release(act);
    if (res.key !== act.placeKey) {
      act.placeKey = res.key;
      if (instant || !this.mounted) {
        c.teleport({ x: res.slot.x + OX, z: res.slot.z + OZ, face: res.slot.face }, res.key, now, false);
      } else this.walkTo(act, res, now);
    }
    // Card in hand: the real task card rides along; otherwise a prop.
    // The real task card rides in the hand when held; then no other prop.
    c.setProp(this.board.heldBy(a.id) ? null : shown.prop);
    // Pose on arrival (and right away when already there).
    if (!c.walking && c.destKey === res.key && !c.arrivedKey) {
      const sit = res.slot.pose === 'sit' && (shown.pose === 'sit' || shown.pose === 'type' || shown.pose === 'read' || shown.pose === 'think' || shown.pose === 'phone' || shown.pose === 'sleep' || shown.pose === 'slump' || shown.pose === 'review' || shown.pose === 'gamepad');
      c.setPose(shown.pose === 'stand' && res.slot.pose === 'sit' ? 'sit' : shown.pose, now, sit || res.slot.pose === 'sit' ? (res.slot.seat ?? 0.33) : undefined);
    }
  }

  private walkTo(act: Actor, res: { slot: Slot; key: string }, now: number) {
    const c = act.c;
    const final = { x: res.slot.x + OX, z: res.slot.z + OZ, face: res.slot.face };
    if (this.opts.reducedMotion) {
      c.teleport(final, res.key, now, true);
      return;
    }
    const [sx, sz] = cellOf(c.position.x - OX, c.position.z - OZ);
    const [gx, gz] = cellOf(res.slot.x, res.slot.z);
    const raw = aStar(COLS, ROWS, sx, sz, gx, gz, this.passable);
    if (!raw) {
      c.teleport(final, res.key, now, true);
      return;
    }
    const cells = smooth([sx, sz], raw, this.passable);
    const pts: [number, number][] = cells.slice(0, -1).map(([x, z]) => {
      const [wx, wz] = cellCenter(x, z);
      return [wx + OX, wz + OZ];
    });
    let len = 0;
    let px = c.position.x;
    let pz = c.position.z;
    for (const [x, z] of [...pts, [final.x, final.z] as [number, number]]) {
      len += Math.hypot(x - px, z - pz);
      px = x;
      pz = z;
    }
    c.speed = 2.0 + Math.min(1.3, len / 12);
    c.walk(pts, final, res.key, now);
    this.shadowDirty = true;
  }

  /** Desk lamp + monitor follow the agent's real state. */
  private paintDesk(act: Actor, intent?: Intent) {
    const d = act.desk;
    if (!d) return;
    const a = this.model.agents.get(act.agentId);
    const it = intent ?? act.shown;
    if (!a || !it) return;
    const quiet = a.status === 'idle' || a.status === 'done' || a.status === 'offline';
    this.building.setDeskLamp(d.index, a.approval ? 'amber' : !a.present || it.icon === 'sleep' || it.icon === 'rate' ? 'off' : 'on');
    const label = it.screen === 'terminal' ? (this.focusCommand(a) ?? '') : a.name;
    this.screens.set(d.index, it.screen === 'idle' && quiet && a.status === 'offline' ? 'off' : it.screen, a.color, label);
  }

  private focusCommand(a: AgentState): string | null {
    for (const t of a.tools.values()) if (t.command) return clean(t.command, 40);
    return null;
  }

  private syncInterns(now: number, instant: boolean) {
    const want = new Set<string>();
    for (const a of this.model.agents.values()) {
      if (!a.present) continue;
      const act = this.actors.get(a.id);
      const d = act?.desk;
      if (!d) continue;
      let k = 0;
      for (const sa of a.subagents.values()) {
        if (k++ > 1) break;
        const key = `${a.id}:${sa.callId}`;
        want.add(key);
        if (this.interns.has(key)) continue;
        const c = new Character(key, lookFor(a.seed + ':' + sa.callId, a.color, 'worker', a.provider, true), { emotes: this.emote, props: this.props });
        const off = k === 2 ? 0.45 : 0;
        const fx = Math.sin(d.rot);
        const fz = Math.cos(d.rot);
        c.teleport({ x: d.intern.x + OX - fx * off, z: d.intern.z + OZ - fz * off, face: d.intern.face }, 'intern', now, false);
        c.setPose('read', now);
        c.setProp('paper');
        if (!instant && !this.opts.reducedMotion) c.root.scale.setScalar(0.01);
        this.actorsGroup.add(c.root);
        this.interns.set(key, { c, parent: a.id, born: instant ? now - 1000 : now, leaving: null });
      }
    }
    for (const [key, i] of this.interns) if (!want.has(key) && i.leaving == null) i.leaving = now;
  }

  // ───────────── surfaces: board, whiteboard, server, mail, archive ─────────────

  private cardWhere = (t: TaskCard): CardWhere => {
    const o = this.cardOverride.get(t.id);
    if (o?.kind === 'held' && this.actors.has(o.agentId) && !this.actors.get(o.agentId)!.leaving) return { kind: 'held', agentId: o.agentId };
    if (o?.kind === 'board') return { kind: 'board' };
    if (t.column === 'doing' && t.assignee) {
      const a = this.model.agents.get(t.assignee);
      const d = this.deskFor(t.assignee);
      if (a?.present && d) return { kind: 'desk', desk: d.index };
    }
    return { kind: 'board' };
  };

  private leadColorOf = (t: TaskCard): number | null => {
    const thread = t.planId ? this.model.plans.get(t.planId)?.thread : null;
    if (!thread) return null;
    for (const a of this.model.agents.values()) if (a.role === 'lead' && a.thread === thread) return a.color;
    return null;
  };

  private syncSurfaces(now: number, instant: boolean) {
    const m = this.model;
    const sel = this.selection?.taskId ?? this.hoverLocal?.taskId ?? this.hoverProp?.taskId ?? null;
    this.board.sync(
      [...m.tasks.values()],
      this.cardWhere,
      (id) => (id ? (m.agents.get(id)?.color ?? null) : null),
      this.leadColorOf,
      now,
      !instant,
      sel,
    );
    const lists: TodoList[] = [...m.agents.values()]
      .filter((a) => a.todo.length && a.present)
      .sort((x, y) => y.todoAt - x.todoAt)
      .slice(0, 3)
      .map((a) => ({ agentId: a.id, name: a.name, color: a.color, steps: a.todo }));
    this.whiteboard.set(
      lists,
      m.notes,
      (author) => m.agents.get(author)?.color ?? hash(author) % 8,
      (author) => m.agents.get(author)?.name ?? (author === 'user' ? 'You' : 'Codly'),
    );
    const merged = new Set(m.merges.map((x) => x.taskId).filter(Boolean));
    this.mail.set(m.commits.map((cm) => ({ sha: cm.sha, agentColor: cm.agentId ? (m.agents.get(cm.agentId)?.color ?? 0) : 0, merged: !!cm.taskId && merged.has(cm.taskId) })));
    this.archive.set(m.memories.length, m.triples, now, !instant);
  }

  private testState(): { state: TestState; busy: boolean } {
    let running: TestState['running'] = null;
    let busy = false;
    for (const a of this.model.agents.values()) {
      for (const t of a.tools.values()) {
        if (t.kind === 'execute' || t.kind === 'test') busy = true;
        if (t.kind === 'test' && (!running || t.since > running.since)) running = { command: clean(t.command ?? 'tests', 40), color: a.color, since: t.since };
      }
    }
    const tt = this.model.tests;
    return { state: { running, last: tt ? { ok: tt.ok, passed: tt.passed, failed: tt.failed, command: tt.command, at: tt.at } : null, history: this.model.testHistory }, busy };
  }

  private placeFrames(animate: boolean) {
    const byAgent = new Map<string, Artifact[]>();
    for (const art of this.model.artifacts.values()) {
      if (!art.agent_id) continue;
      const list = byAgent.get(art.agent_id) ?? [];
      list.push(art);
      byAgent.set(art.agent_id, list);
    }
    const out: { artifact: Artifact; desk: Desk; url: string | null; slot: number }[] = [];
    for (const [id, list] of byAgent) {
      const d = this.deskFor(id);
      if (!d) continue;
      list
        .sort((a, b) => b.created_at - a.created_at)
        .slice(0, 2)
        .forEach((art, k) => out.push({ artifact: art, desk: d, url: this.opts.artifactUrl?.(art) ?? null, slot: k }));
    }
    this.frames.set(out, performance.now(), animate && !this.opts.reducedMotion);
    this.shadowDirty = true;
  }

  // ───────────── effects ─────────────

  private local(x: number, y: number, z: number) {
    return new Vector3(x, y, z);
  }

  private charLocal(id: string, dy = 0.75): Vector3 | null {
    const act = this.actors.get(id);
    if (!act) return null;
    const p = act.c.position;
    return new Vector3(p.x - OX, dy, p.z - OZ);
  }

  private play(f: Fx, now: number, wall: number) {
    const rm = this.opts.reducedMotion;
    const t = this.theme;
    switch (f.t) {
      case 'ticker':
        this.pushTicker(f, false);
        if (f.tone === 'warn' && f.agentId && !this.announced.has(f.text)) {
          this.announced.add(f.text);
          this.opts.onannounce?.(f.text + (f.code ? ` ${f.code}` : ''));
        }
        return;
      case 'errand':
        // (The agent may turn up a beat after its task starts: queue anyway; unrun errands expire.)
        if (!rm && this.model.agents.has(f.agentId)) {
          this.errands.push(f.agentId, f.kind, f.taskId, wall);
          if (f.kind === 'card-take' && f.taskId) this.cardOverride.set(f.taskId, { kind: 'board' });
          if (f.kind === 'card-return' && f.taskId) this.cardOverride.set(f.taskId, { kind: 'held', agentId: f.agentId, untilDesk: false, at: wall });
        }
        return;
      case 'card':
        return;
      case 'plane': {
        const from = this.charLocal(f.agentId, 0.95);
        const lead = f.to ? this.deskFor(f.to) : null;
        const to = lead ? this.local(lead.x, 0.85, lead.z) : this.local(21.5, 0.9, 3.5);
        if (!from) return;
        if (rm) this.fx.add(new Glow(to, t.lamp, 1.2, now, this.glowTex, 900));
        else this.fx.add(new PaperPlane(from, to, now, () => this.fx.add(new Glow(to, t.lamp, 1.2, performance.now(), this.glowTex, 900))));
        return;
      }
      case 'sticky': {
        const wb = this.L.whiteboard;
        this.fx.add(new Glow(this.local((wb.x0 + wb.x1) / 2 + 1.4, 1.3, wb.z + 0.2), lin('#ffe58a'), 2.6, now, this.glowTex, 1300));
        return;
      }
      case 'drawer':
        return;
      case 'catalog':
        return;
      case 'mail': {
        this.mail.raise(now);
        const mb = this.L.mailbox;
        this.fx.add(new Glow(this.local(mb.x, 0.9, mb.z), lin('#ffd98a'), 1.4, now, this.glowTex, 1000));
        return;
      }
      case 'merge': {
        const mb = this.L.mailbox;
        const at = this.local(mb.x + 0.5, 1.0, mb.z);
        this.mail.raise(now, 4000);
        if (!rm) this.fx.add(new Confetti(at, CONFETTI, now, (wall & 0xffff) ^ 7, 80, 1800, this.glowTex));
        this.fx.add(new Glow(at, lin('#ffd98a'), 2.2, now, this.glowTex, 1500));
        return;
      }
      case 'tests': {
        const tm = this.L.testMonitor;
        const at = this.local(tm.x, tm.y, tm.z + 0.4);
        this.fx.add(new Glow(at, f.ok ? t.success : t.danger, 3.2, now, this.glowTex, 1600));
        if (f.ok && !rm) this.fx.add(new Confetti(this.local(tm.x, tm.y + 0.6, tm.z + 0.6), CONFETTI, now, (wall & 0xffff) ^ 3, 70, 1700, this.glowTex));
        if (!f.ok && !rm) this.fx.add(new Puff(this.local(tm.x, tm.y + 0.7, tm.z + 0.4), lin('#8f96a3'), now, this.glowTex));
        return;
      }
      case 'shelf': {
        const s = this.L.shelves[(wall >>> 4) % this.L.shelves.length]; // >>> keeps it positive (Date.now() overflows >> )
        this.fx.add(new Glow(this.local(s.x + Math.sin(s.rot) * 0.4, 1.1, s.z + Math.cos(s.rot) * 0.4), lin('#ffe1a8'), 2.4, now, this.glowTex, 1400));
        return;
      }
      case 'sparkle': {
        const d = this.deskFor(f.agentId);
        if (d && !rm) this.fx.add(new Sparkles(this.local(d.screen[0], d.screen[1] + 0.1, d.screen[2]), identity(this.model.agents.get(f.agentId)?.color ?? 0, t.name).glow, now, this.emote('spark'), wall & 0xffff));
        return;
      }
      case 'trash': {
        const d = this.deskFor(f.agentId);
        if (d && !rm) this.fx.add(new Puff(this.local(d.x, 0.75, d.z), lin('#c9c2b6'), now, this.glowTex));
        return;
      }
      case 'artifact':
        this.placeFrames(true);
        return;
      case 'hello':
        return;
    }
  }

  private onErrand(agentId: string, e: Errand, phase: 'act' | 'done' | 'drop') {
    const wall = Date.now();
    const now = performance.now();
    if (e.taskId && (e.kind === 'card-take' || e.kind === 'card-return')) {
      if (e.kind === 'card-take') {
        if (phase === 'act') this.cardOverride.set(e.taskId, { kind: 'held', agentId, untilDesk: true, at: wall });
        else if (phase === 'drop') this.cardOverride.delete(e.taskId);
      } else if (phase !== 'done') this.cardOverride.delete(e.taskId);
      this.surfacesDirty = true;
    }
    if (phase === 'act') {
      const p = this.charLocal(agentId, 0.7);
      if (p && !this.opts.reducedMotion) {
        if (e.kind === 'commit') this.fx.add(new Puff(this.local(this.L.mailbox.x, 0.85, this.L.mailbox.z), lin('#f3e6cc'), now, this.glowTex));
        if (e.kind === 'memory') this.fx.add(new Glow(p.clone().setY(0.9), lin('#ffd98a'), 1.6, now, this.glowTex, 1100));
      }
    }
    // Never re-enter sync() from inside it: the next frame picks this up.
    this.needSync = true;
    this.wake();
  }

  private needSync = false;
  private surfacesDirty = false;

  // ───────────── selection, hover, camera ─────────────

  private overview(instant: boolean) {
    const pts: Vector3[] = [];
    for (const [x, z] of [
      [0, 0],
      [W, 0],
      [W, D],
      [0, D],
    ])
      pts.push(new Vector3(x + OX, 0, z + OZ), new Vector3(x + OX, 2.3, z + OZ));
    const ins = this.freeInsets();
    const ax = Math.max(0.3, (this.width - ins.left - ins.right) / Math.max(1, this.width));
    const ay = Math.max(0.3, (this.height - ins.top - ins.bottom) / Math.max(1, this.height));
    const v = this.rig.fit(pts, DEFAULT_YAW, DEFAULT_PITCH, this.opts.backdrop ? 0.02 : 0.05, this.opts.backdrop ? 1 : ax, ay);
    if (instant) this.rig.set(v);
    else this.rig.flyTo(v, this.opts.reducedMotion);
    this.userMoved = false;
    this.wake();
  }

  private focusOn(sel: WorldSelection) {
    const rm = this.opts.reducedMotion;
    if (sel.agentId) {
      const act = this.actors.get(sel.agentId);
      if (act) {
        const p = act.c.position;
        this.rig.flyTo({ x: p.x, z: p.z, y: 0.4, dist: 15, pitch: 0.86 }, rm);
        if (this.followOn) this.rig.follow = () => this.actors.get(sel.agentId!)?.c.position ?? null;
      }
      return;
    }
    if (sel.taskId) {
      const p = this.board.cardPosition(sel.taskId);
      if (p) this.rig.flyTo({ x: p.x + OX + 2.5, z: p.z + OZ, y: 0.6, dist: 13, pitch: 0.8 }, rm);
      this.rig.follow = null;
      return;
    }
    if (sel.path) {
      // A file: whoever is working on it right now.
      for (const a of this.model.agents.values())
        for (const t of a.tools.values())
          if (t.path && (t.path === sel.path || t.path.endsWith('/' + sel.path))) {
            const act = this.actors.get(a.id);
            if (act) this.rig.flyTo({ x: act.c.position.x, z: act.c.position.z, y: 0.4, dist: 16, pitch: 0.86 }, rm);
            return;
          }
    }
  }

  private toggleFollow() {
    const id = this.selection?.agentId;
    if (!id) return;
    this.followOn = !this.followOn;
    this.rig.follow = this.followOn ? () => this.actors.get(id)?.c.position ?? null : null;
    if (this.followOn) this.rig.flyTo({ dist: Math.min(this.rig.goal.dist, 15) });
    this.wake();
  }

  private rayAt(clientX: number, clientY: number) {
    const r = this.renderer.domElement.getBoundingClientRect();
    return this.rig.raycaster(((clientX - r.left) / r.width) * 2 - 1, -((clientY - r.top) / r.height) * 2 + 1);
  }

  private groundOf(clientX: number, clientY: number) {
    const r = this.renderer.domElement.getBoundingClientRect();
    return this.rig.groundAt(((clientX - r.left) / r.width) * 2 - 1, -((clientY - r.top) / r.height) * 2 + 1);
  }

  private pick(clientX: number, clientY: number): WorldSelection | null {
    const r = this.renderer.domElement.getBoundingClientRect();
    const x = clientX - r.left;
    const y = clientY - r.top;
    let best: { id: string; d: number } | null = null;
    const pxPerUnit = this.height / (2 * Math.tan((this.rig.camera.fov * Math.PI) / 360) * Math.max(1, this.rig.cur.dist));
    const rad = Math.max(18, 0.55 * pxPerUnit);
    for (const [id, act] of this.actors) {
      if (act.leaving) continue;
      act.c.root.updateWorldMatrix(true, true);
      this.v.copy(act.c.position).setY(0.65 * act.c.look.scale).project(this.rig.camera);
      if (this.v.z > 1) continue;
      const sx = (this.v.x * 0.5 + 0.5) * r.width;
      const sy = (-this.v.y * 0.5 + 0.5) * r.height;
      const dist = Math.hypot(sx - x, (sy - y) * 0.8);
      if (dist < rad && (!best || dist < best.d)) best = { id, d: dist };
    }
    if (best) return { agentId: best.id };
    const nx = (x / r.width) * 2 - 1;
    const ny = -(y / r.height) * 2 + 1;
    const ray = this.rig.raycaster(nx, ny);
    const hits = ray.intersectObjects(this.board.pickables(), false);
    if (hits.length) {
      const id = this.board.taskOfMesh(hits[0].object);
      if (id) return { taskId: id };
    }
    return null;
  }

  private listen() {
    const el = this.container;
    const canvas = this.renderer.domElement;
    let down: { x: number; y: number; button: number; moved: boolean; id: number; orbit: boolean } | null = null;
    let last = { x: 0, y: 0 };
    let hoverQueued = false;
    let hoverAt = { x: 0, y: 0 };
    const interactive = () => !this.opts.backdrop;

    const onDown = (e: PointerEvent) => {
      if (!interactive()) return;
      down = { x: e.clientX, y: e.clientY, button: e.button, moved: false, id: e.pointerId, orbit: e.button === 2 || e.button === 1 || e.altKey || e.shiftKey };
      last = { x: e.clientX, y: e.clientY };
      canvas.setPointerCapture(e.pointerId);
      el.focus({ preventScroll: true });
    };
    const onMove = (e: PointerEvent) => {
      if (!interactive()) return;
      if (down) {
        const dx = e.clientX - last.x;
        const dy = e.clientY - last.y;
        last = { x: e.clientX, y: e.clientY };
        if (Math.hypot(e.clientX - down.x, e.clientY - down.y) > 4) down.moved = true;
        if (!down.moved) return;
        if (down.orbit) this.rig.orbit(dx, dy);
        else {
          this.rig.pan(dx, dy, this.height);
          this.followOn = false;
        }
        this.userMoved = true;
        this.wake();
        return;
      }
      hoverAt = { x: e.clientX, y: e.clientY };
      if (!hoverQueued) {
        hoverQueued = true;
        requestAnimationFrame(() => {
          hoverQueued = false;
          if (this.disposed) return;
          if (this.build) {
            const g = this.build.carry && this.groundOf(hoverAt.x, hoverAt.y);
            if (g) this.moveCarry(g.x - OX, g.z - OZ);
            return;
          }
          this.setLocalHover(this.pick(hoverAt.x, hoverAt.y));
        });
      }
    };
    const onUp = (e: PointerEvent) => {
      if (!down) return;
      const wasClick = !down.moved && down.button === 0;
      canvas.releasePointerCapture?.(down.id);
      down = null;
      if (wasClick && this.build) {
        if (this.build.carry) this.dropCarry();
        else {
          const uid = this.decor.pick(this.rayAt(e.clientX, e.clientY));
          if (uid) this.build.onpick(uid);
        }
      } else if (wasClick) {
        const sel = this.pick(e.clientX, e.clientY);
        const f = sel ? null : this.furnitureAt(e.clientX, e.clientY);
        this.opts.onfurniture?.(f);
        if (!f) this.opts.onselect(sel);
      }
    };
    const onLeave = () => this.setLocalHover(null);
    const onWheel = (e: WheelEvent) => {
      if (!interactive()) return;
      e.preventDefault();
      const r = canvas.getBoundingClientRect();
      const nx = ((e.clientX - r.left) / r.width) * 2 - 1;
      const ny = -((e.clientY - r.top) / r.height) * 2 + 1;
      const scale = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
      // Pinch (ctrl) or ⌘ + wheel zooms; a mouse wheel zooms; two-finger scroll pans.
      const mouseWheel = e.deltaMode !== 0 || (e.deltaX === 0 && Math.abs(e.deltaY) >= 50 && Number.isInteger(e.deltaY));
      if (e.ctrlKey || e.metaKey || mouseWheel) {
        const dy = e.deltaY * scale * (e.ctrlKey ? 3 : 1);
        this.rig.zoom(Math.exp(-dy * 0.0018), this.rig.groundAt(nx, ny));
      } else {
        this.rig.pan(-e.deltaX * scale, -e.deltaY * scale, this.height);
        this.followOn = false;
      }
      this.userMoved = true;
      this.wake();
    };
    const onKey = (e: KeyboardEvent) => {
      if (!interactive() || e.metaKey || e.ctrlKey || e.altKey) return;
      if ((e.target as HTMLElement)?.closest?.('.of-sr')) return;
      const c = this.build?.carry;
      if (c && (e.key === 'r' || e.key === 'R' || e.key === 'Escape' || ((e.key === 'Delete' || e.key === 'Backspace') && c.uid))) {
        e.preventDefault();
        if (e.key === 'Escape') this.endCarry();
        else if (e.key === 'r' || e.key === 'R') this.rotateCarry();
        else if (c.uid) c.onsell(c.uid);
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
        case 'f':
        case 'F':
          this.toggleFollow();
          break;
        case 'Escape':
          if (this.selection) this.opts.onselect(null);
          else this.overview(false);
          break;
        case '0':
        case 'Home':
          this.overview(false);
          break;
        default:
          handled = false;
      }
      if (handled) {
        if (e.key !== 'Escape' && e.key !== '0' && e.key !== 'Home' && e.key !== 'f' && e.key !== 'F') this.userMoved = true;
        e.preventDefault();
        this.wake();
      }
    };
    const onContext = (e: Event) => e.preventDefault();
    const onLost = (e: Event) => {
      e.preventDefault();
      this.lost = true;
      this.loop.setVisible(false);
    };
    const onRestored = () => {
      this.lost = false;
      this.shadowDirty = true;
      this.loop.setVisible(document.visibilityState !== 'hidden');
      this.wake();
    };
    const onVis = () => this.loop.setVisible(document.visibilityState !== 'hidden' && !this.lost);

    canvas.addEventListener('pointerdown', onDown);
    canvas.addEventListener('pointermove', onMove);
    canvas.addEventListener('pointerup', onUp);
    canvas.addEventListener('pointercancel', onUp);
    canvas.addEventListener('pointerleave', onLeave);
    canvas.addEventListener('wheel', onWheel, { passive: false });
    canvas.addEventListener('contextmenu', onContext);
    canvas.addEventListener('webglcontextlost', onLost);
    canvas.addEventListener('webglcontextrestored', onRestored);
    el.addEventListener('keydown', onKey);
    document.addEventListener('visibilitychange', onVis);
    this.cleanups.push(() => {
      canvas.removeEventListener('pointerdown', onDown);
      canvas.removeEventListener('pointermove', onMove);
      canvas.removeEventListener('pointerup', onUp);
      canvas.removeEventListener('pointercancel', onUp);
      canvas.removeEventListener('pointerleave', onLeave);
      canvas.removeEventListener('wheel', onWheel);
      canvas.removeEventListener('contextmenu', onContext);
      canvas.removeEventListener('webglcontextlost', onLost);
      canvas.removeEventListener('webglcontextrestored', onRestored);
      el.removeEventListener('keydown', onKey);
      document.removeEventListener('visibilitychange', onVis);
    });
    if (typeof IntersectionObserver !== 'undefined') {
      this.io = new IntersectionObserver((entries) => {
        for (const en of entries) this.loop.setOnScreen(en.isIntersecting);
      });
      this.io.observe(el);
    }
    const onLabel = (e: MouseEvent) => {
      if (!interactive()) return;
      const t = (e.target as HTMLElement).closest?.('.of-label') as HTMLElement | null;
      if (!t) return;
      if (t.dataset.agent) this.opts.onselect({ agentId: t.dataset.agent });
      else if (t.dataset.task) this.opts.onselect({ taskId: t.dataset.task });
    };
    this.overlay.root.addEventListener('click', onLabel);
    this.cleanups.push(() => this.overlay.root.removeEventListener('click', onLabel));
  }

  private setLocalHover(h: WorldSelection | null) {
    if (JSON.stringify(h) === JSON.stringify(this.hoverLocal)) return;
    const taskChanged = (h?.taskId ?? null) !== (this.hoverLocal?.taskId ?? null);
    this.hoverLocal = h;
    this.renderer.domElement.style.cursor = h ? 'pointer' : '';
    this.opts.onhover(h);
    if (taskChanged) this.syncSurfaces(performance.now(), false);
    this.wake();
  }

  // ───────────── frame ─────────────

  private step(now: number, dt: number): StepResult {
    this.stats.frames++;
    const wall = Date.now();
    // Time-driven intent changes (excursion thresholds, dwell ends, errands finishing).
    const dwellDue = this.dwell.flush(wall).length > 0;
    if (dwellDue || this.needSync || wall >= this.intentWakeAt || wall >= this.errands.nextAt()) {
      this.needSync = false;
      this.sync();
    } else if (this.surfacesDirty) this.syncSurfaces(now, false);
    this.surfacesDirty = false;
    const camMoving = this.rig.update(dt);
    let busy = camMoving;
    let nextAt = Infinity;
    const rm = this.opts.reducedMotion;
    const camDist = this.rig.cur.dist;
    const emoteScale = Math.min(2.4, Math.max(1, camDist / 20));
    // Live: continuous motion (breathing, glances, games) while the office is on screen.
    // Live whenever the office is drawn at all (hidden = 0 frames); stepped only for reduced motion.
    const lively = !rm;
    if (lively) busy = this.lifeStep(now, dt) || busy;
    for (const [id, act] of this.actors) {
      act.c.emoteScale = emoteScale;
      const r = act.c.update(now, dt, rm, lively);
      if (r.moving) busy = true;
      if (act.c.walking) this.shadowDirty = true;
      nextAt = Math.min(nextAt, r.nextAt);
      // Arrivals: the pose for the place, errands start acting.
      if (!act.c.walking && act.c.arrivedKey && act.placeKey === act.c.arrivedKey && act.shown) {
        const key = act.c.arrivedKey;
        if (act.leaving) {
          if (act.c.opacity > 0.99 && key === 'door') act.c.fadeTo(0, now, 300);
        } else {
          const a = this.model.agents.get(id);
          if (a) {
            this.errands.arrived(id, key, wall, (e) => {
              const pl = errandPlace(e, a, this.model);
              return pl ? placeKey(pl) : null;
            });
            for (const [tid, o] of this.cardOverride) {
              if (o.kind === 'held' && o.agentId === id && o.untilDesk && (key === 'desk' || wall - o.at > 15_000)) {
                this.cardOverride.delete(tid);
                this.surfacesDirty = true;
              }
            }
            const res = this.resolve(act, act.shown.place);
            if (res && res.key === key) {
              const s = act.shown;
              const sit = res.slot.pose === 'sit';
              act.c.setPose(s.pose === 'stand' && sit ? 'sit' : s.pose, now, sit ? (res.slot.seat ?? 0.33) : undefined);
            }
          }
        }
        act.c.arrivedKey = null;
        act.c.destKey = act.placeKey;
      }
      if (act.leaving && act.c.opacity <= 0.001) {
        act.c.dispose();
        this.actors.delete(id);
        this.dwell.delete(id);
        busy = true;
      }
    }
    for (const [key, i] of this.interns) {
      const t = i.leaving != null ? Math.min(1, (now - i.leaving) / 300) : Math.min(1, (now - i.born) / 350);
      if (i.leaving != null) {
        i.c.root.scale.setScalar(Math.max(0.001, (1 - t) * i.c.look.scale));
        busy = true;
        if (t >= 1 || rm) {
          i.c.dispose();
          this.interns.delete(key);
        }
        continue;
      }
      if (t < 1 && !rm) {
        const s = t < 1 ? 1 + Math.sin(t * Math.PI) * 0.25 - (1 - t) * 0.9 : 1;
        i.c.root.scale.setScalar(Math.max(0.01, s) * i.c.look.scale);
        busy = true;
      } else i.c.root.scale.setScalar(i.c.look.scale);
      const r = i.c.update(now, dt, rm, lively);
      if (r.moving) busy = true;
      nextAt = Math.min(nextAt, r.nextAt);
    }
    this.ghosts.forEach((g, i) => {
      const r = g.update(now, dt, rm, lively);
      if (!rm) g.root.position.y = 0.14 + 0.06 * Math.sin(now / 950 + i * 2.1);
      if (r.moving || (lively && !rm)) busy = true;
    });
    // Cards in flight, and held cards following hands.
    if (this.board.update(now)) busy = true;
    if (this.surfacesDirty) {
      this.syncSurfaces(now, false);
      this.surfacesDirty = false;
    }
    const mailR = this.mail.update(now, dt);
    if (mailR.moving) busy = true;
    nextAt = Math.min(nextAt, mailR.nextAt);
    if (this.archive.update(now)) busy = true;
    if (this.frames.update(now)) busy = true;
    if (this.fx.update(now)) busy = true;
    nextAt = Math.min(nextAt, this.screens.update(now, rm));
    const ts = this.testState();
    nextAt = Math.min(nextAt, this.server.update(ts.state, ts.busy, now, rm));
    this.placeRings();
    if (this.placePlumbob(now, dt)) busy = true;

    if (this.shadowDirty) {
      this.renderer.shadowMap.needsUpdate = true;
      this.shadowDirty = false;
    }
    if (!this.lost) {
      this.renderer.render(this.scene, this.rig.camera);
      this.stats.renders++;
    }
    const live = this.updateLabels(wall);

    // Next scheduled change while still.
    const toPerf = (w: number) => now + (w - wall);
    let wakeAt = nextAt;
    wakeAt = Math.min(wakeAt, toPerf(this.dwell.nextAt()), toPerf(this.intentWakeAt), toPerf(this.errands.nextAt()));
    if (live) wakeAt = Math.min(wakeAt, now + 1000 - (wall % 1000));
    for (const a of this.model.agents.values()) {
      if (a.outcome && wall - a.outcome.at < OUTCOME_MS) wakeAt = Math.min(wakeAt, toPerf(a.outcome.at + OUTCOME_MS + 20));
    }
    for (const act of this.actors.values()) if (act.hello && now - act.bornAt < HELLO_MS) wakeAt = Math.min(wakeAt, act.bornAt + HELLO_MS + 20);
    // Frame budget: ~30 fps while live (20 behind the blurred conversation), full rate for a
    // walk otherwise; nothing when still and not live.
    const interval = !busy ? 0 : lively ? (this.opts.backdrop ? 50 : 33) : this.opts.backdrop ? 33 : 0;
    return { animating: busy, wakeAt, interval };
  }

  /** Who plays which game (they get its cues), and everyone else glances at people walking by. */
  private lifeStep(now: number, dt: number): boolean {
    const players: GamePlayers = { foosball: [], pool: [], console: [] };
    const playing = new Set<string>();
    for (const act of this.actors.values()) {
      const [station, idx] = act.slot?.split(':') ?? [];
      const game = station?.startsWith('games.') ? (station.slice(6) as keyof GamePlayers) : null;
      const there = !!game && act.placeKey === 'st:' + station && act.c.destKey === act.placeKey && !act.leaving;
      if (game && there && (game === 'pool' || !act.c.walking) && players[game]) {
        players[game].push({ slot: Number(idx) || 0, c: act.c });
        playing.add(act.agentId);
      }
    }
    const moving = this.games.update(now, dt, players);
    const walkers = [...this.actors.values()].filter((a) => a.c.walking && a.c.opacity > 0.5);
    for (const act of this.actors.values()) {
      if (playing.has(act.agentId)) continue;
      act.c.cue = null;
      let best: Actor | null = null;
      let bd = 2.6;
      if (!act.c.walking) {
        for (const w of walkers) {
          if (w === act) continue;
          const d = w.c.position.distanceTo(act.c.position);
          if (d < bd) {
            bd = d;
            best = w;
          }
        }
      }
      if (best) {
        let v = this.gazeBufs.get(act.agentId);
        if (!v) this.gazeBufs.set(act.agentId, (v = new Vector3()));
        act.c.gaze = best.c.headTop(v);
      } else act.c.gaze = null;
    }
    return moving;
  }

  private placePlumbob(now: number, dt: number): boolean {
    const id = this.selection?.agentId;
    const act = id ? this.actors.get(id) : null;
    if (!act || act.leaving || this.opts.backdrop) {
      this.plumbob.visible = false;
      return false;
    }
    const color = { green: 0x4fdc7b, amber: 0xffbf3c, red: 0xff6b5b, blue: 0x62a8ff, grey: 0xb9c0cc }[plumbobOf(this.model.agents.get(act.agentId) ?? null)];
    const m = this.plumbob.material as MeshStandardMaterial;
    m.color.setHex(color);
    m.emissive.setHex(color).multiplyScalar(0.35);
    const head = act.c.headTop(new Vector3());
    const still = this.opts.reducedMotion;
    this.plumbob.visible = true;
    this.plumbob.position.set(head.x, head.y + 0.42 + (still ? 0 : Math.sin(now / 520) * 0.03), head.z);
    if (!still) this.plumbob.rotation.y += dt * 1.6;
    this.plumbob.scale.setScalar(act.c.look.scale);
    return !still;
  }

  private placeRings() {
    const sel = this.selection?.agentId ? this.actors.get(this.selection.agentId) : null;
    const hovId = this.hoverLocal?.agentId ?? this.hoverProp?.agentId;
    const hov = hovId && hovId !== this.selection?.agentId ? this.actors.get(hovId) : null;
    for (const [ring, act, size] of [
      [this.selRing, sel, 1.25],
      [this.hovRing, hov, 1.1],
    ] as const) {
      if (!act || act.leaving) {
        ring.visible = false;
        continue;
      }
      ring.visible = true;
      const p = act.c.position;
      ring.position.set(p.x - OX, 0.025, p.z - OZ);
      ring.scale.setScalar(size * act.c.look.scale);
      const a = this.model.agents.get(act.agentId);
      (ring.material as MeshBasicMaterial).color.setRGB(...identity(a?.color ?? 0, this.theme.name).glow);
    }
  }

  private sentenceOf(a: AgentState, wall: number): Sentence {
    const ext = this.opts.narrate?.(a.id);
    if (ext && ext.parts?.length) {
      let pre = '';
      let code: string | null = null;
      let post = '';
      for (const p of ext.parts) {
        const t = clean(p.t, 80);
        if (p.code && code == null) code = t;
        else if (code == null) pre += t;
        else post += t;
      }
      const tone: Tone = ext.tone === 'wait' ? 'warn' : ext.tone === 'blocked' ? 'danger' : ext.tone === 'done' ? 'success' : ext.tone === 'idle' || ext.tone === 'off' ? 'muted' : 'normal';
      const text = ext.parts.map((p) => clean(p.t, 80)).join('');
      return { text, pre: pre.trimEnd(), code, post, tone, live: /\d:\d\d/.test(text) };
    }
    return sentence(a, wall);
  }

  private updateLabels(wall: number): boolean {
    const specs: LabelSpec[] = [];
    let live = false;
    const sel = this.selection;
    const hov = this.hoverLocal ?? this.hoverProp;
    const now = performance.now();
    const head = new Vector3();
    for (const act of this.actors.values()) {
      const a = this.model.agents.get(act.agentId);
      if (!a || act.leaving) continue;
      const id = identity(a.color, this.theme.name);
      const selected = sel?.agentId === a.id;
      const hovered = hov?.agentId === a.id;
      const urgent = !!a.approval || isBlocked(a);
      const fresh = !!a.outcome && wall - a.outcome.at < OUTCOME_MS;
      const said = this.said.get(a.id);
      const saying = !!said && now - said.at < SAY_MS;
      if (saying) live = true;
      const hello = act.hello && now - act.bornAt < HELLO_MS;
      let s: Sentence | null = null;
      if (saying && !urgent) {
        // A speech bubble: what it just said in the chat.
        s = { text: said!.text, pre: `“${said!.text}”`, code: null, post: '', tone: 'normal', live: false };
      } else if (selected || hovered || urgent || fresh || hello) {
        s = hello && !selected && !hovered ? { text: `Hi, I’m ${a.name}`, pre: `Hi, I’m ${a.name}`, code: null, post: '', tone: 'normal', live: false } : this.sentenceOf(a, wall);
        if (s.live && (selected || hovered)) live = true;
      }
      act.c.root.updateWorldMatrix(true, true);
      act.c.headTop(head);
      // The selected agent's plumbob floats between its head and its label.
      const extra = act.c.look.scale * ((act.shown?.emote === 'rain' ? 0.62 : act.shown?.emote ? 0.58 : 0.12) + (selected && !this.opts.backdrop ? 0.5 : 0));
      const pri = (selected ? 1000 : 0) + (hovered ? 900 : 0) + (a.approval ? 500 : 0) + (isBlocked(a) ? 300 : 0) + (fresh || saying ? 150 : 0) + (a.role === 'lead' ? 120 : 0) + 100;
      specs.push({
        key: 'a:' + a.id,
        kind: 'agent',
        world: head.clone().setY(head.y + extra),
        dy: -4,
        priority: pri,
        title: a.name,
        pre: s?.pre ?? '',
        code: s?.code ?? null,
        post: s?.post ?? '',
        tone: s?.tone ?? (a.status === 'idle' || a.status === 'offline' ? 'muted' : 'normal'),
        color: id.ink,
        badge: a.role === 'lead' ? 'Lead' : a.badge ?? (a.role === 'reviewer' ? 'Review' : a.subagents.size ? `+${a.subagents.size}` : null),
        icon: act.shown?.icon ?? null,
        chip: s ? null : (chipFor(a)?.text ?? null),
        selected,
        hovered,
        agentId: a.id,
        compact: !s,
        urgent,
        dim: a.status === 'idle' && !selected && !hovered && !urgent,
      });
    }
    for (const r of this.L.rooms) {
      const title = r.id === 'library' && this.files ? `${r.name} · ${this.files.toLocaleString()} files` : r.name;
      specs.push({ key: 'r:' + r.id, kind: 'room', world: new Vector3(r.label[0] + OX, 0.05, r.label[1] + OZ), dy: 0, priority: 20, title });
    }
    this.overlay.update(specs, this.rig.camera, this.width, this.height, this.freeInsets());
    return live;
  }

  private pushTicker(f: Extract<Fx, { t: 'ticker' }>, emit = true) {
    const ag = f.agentId ? this.model.agents.get(f.agentId) : null;
    this.ticker.push({ id: f.eventId || Math.random(), text: f.text, code: f.code, tone: f.tone, agentId: f.agentId, color: ag ? identity(ag.color, this.theme.name).ink : null, at: f.at });
    if (this.ticker.length > 6) this.ticker.splice(0, this.ticker.length - 6);
    if (emit) this.emitTicker();
  }

  private emitTicker() {
    this.opts.onticker?.([...this.ticker]);
  }

  private notifyAgents() {
    if (!this.opts.onagents) return;
    const wall = Date.now();
    const list: AgentListItem[] = [];
    for (const a of this.model.agents.values()) {
      if (!a.present) continue;
      const s = this.sentenceOf(a, wall);
      list.push({ id: a.id, name: a.name, role: a.role, provider: a.provider, sentence: s.text.replace(/ · \d+:\d\d(:\d\d)?/, ''), tone: s.tone, color: identity(a.color, this.theme.name).ink });
    }
    const sig = JSON.stringify(list);
    if (sig === this.agentListSig) return;
    this.agentListSig = sig;
    this.opts.onagents(list);
  }
}
