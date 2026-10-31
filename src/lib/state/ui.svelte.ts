// UI state shared by every surface. One selection drives everything: click an agent, a task or a
// file anywhere and the sheet opens on it, the office follows it, the plan scrolls to it. One
// hover cross-highlights (crew pill ↔ task card ↔ timeline lane ↔ the agent in the office).
// Small preferences persist in localStorage.

import type { Provider } from '../gen/Provider';

/** Home + four rooms (REDESIGN §2). */
export type Room = 'home' | 'plan' | 'changes' | 'knowledge' | 'terminal';
/** blend = office behind the work (default) · office = full office (⌘1) · work = no office (⌘2) ·
 *  map = the neighbourhood, every lot of yours from outside (⌘3). */
export type Mode = 'blend' | 'office' | 'work' | 'map';
export type Selection =
  | { kind: 'agent'; id: string; eventId?: number }
  | { kind: 'task'; id: string; eventId?: number }
  | { kind: 'file'; id: string }
  | null;
export type Hover = { agentId: string | null; taskId: string | null; path: string | null };
export type DiffTarget = { kind: 'plan' | 'task'; id: string } | null;
/** List (default): the plan's tasks grouped by status · Flow: the DAG · Timeline: agent lanes. */
export type PlanLens = 'list' | 'flow' | 'timeline';
export type ChangesLens = 'diffs' | 'artifacts';
export type KnowledgeLens = 'memory' | 'graph' | 'search' | 'find' | 'files' | 'messages';

export const ROOMS: { id: Room; label: string; icon: string; key: string; hint: string }[] = [
  { id: 'home', label: 'Home', icon: 'home', key: 'H', hint: 'Your requests, the crew and the office' },
  { id: 'plan', label: 'Plan', icon: 'plan', key: 'P', hint: 'Tasks, dependencies and who does what' },
  { id: 'changes', label: 'Changes', icon: 'diff', key: 'C', hint: 'Diffs and artifacts' },
  { id: 'knowledge', label: 'Knowledge', icon: 'memory', key: 'K', hint: 'Team memory, knowledge graph, code search' },
  { id: 'terminal', label: 'Terminal', icon: 'terminal', key: 'T', hint: 'A shell in the repo or any worktree' },
];

const PREFS_KEY = 'codly.ui.v2';

/** `?onboard=1` in the browser demo: a fresh machine and a fresh quest, whatever was saved. */
export const ONBOARD_DEMO = typeof location !== 'undefined' && new URLSearchParams(location.search).get('onboard') === '1';

/** Request threads: "lead" (the first) and "lead:<id>" (parallel requests, each with its own Lead). */
export const isRequestThread = (thread: string | null | undefined): thread is string => thread === 'lead' || !!thread?.startsWith('lead:');

/** A short id for a new request thread ("lead:" + shortId()). */
export function shortId(): string {
  const b = new Uint8Array(6);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => (x % 36).toString(36)).join('');
}

type Prefs = {
  room: Room;
  mode: Mode | 'city';
  focus: boolean;
  /** The left rail shows labels next to its icons. */
  rail: boolean;
  /** First-run quest: finished once (or skipped), the levels done, sound on. */
  onboarded: boolean;
  quest: string[];
  questSound: boolean;
  sections: Record<string, boolean>;
  /** v2: the list lens became the default (older `planLens` values are ignored on purpose). */
  planView: PlanLens;
  changesLens: ChangesLens;
  knowledgeLens: KnowledgeLens;
  timelineZoom: number;
  pinned: string[];
  keptPlans: string[];
};

function loadPrefs(): Partial<Prefs> {
  try {
    return JSON.parse(localStorage.getItem(PREFS_KEY) ?? '{}') as Partial<Prefs>;
  } catch {
    return {};
  }
}

/** Wall clock. The browser demo can steer it (virtual clock) so relative times stay coherent. */
export const wallNow = (): number => (globalThis as { __codlyNow?: () => number }).__codlyNow?.() ?? Date.now();

const mq = (q: string) => (typeof matchMedia === 'function' ? matchMedia(q) : null);

class Ui {
  private prefs = loadPrefs();

  room = $state<Room>(this.prefs.room ?? 'home');
  mode = $state<Mode>(this.prefs.mode === 'city' ? 'office' : (this.prefs.mode ?? 'blend'));
  /** ⌘. — dim and blur the office for reading. */
  focus = $state(this.prefs.focus ?? false);
  /** The rail's icons-only (52) and labelled (184) widths; everything that sits beside it reads railW. */
  railOpen = $state(this.prefs.rail ?? false);
  railW = $derived(this.railOpen ? 184 : 52);
  /** The first-run quest overlay: shown until finished or skipped, and again from Settings or the palette. */
  onboarded = $state(!ONBOARD_DEMO && (this.prefs.onboarded ?? false));
  onboardingOpen = $state(false);
  quest = $state<string[]>(ONBOARD_DEMO ? [] : (this.prefs.quest ?? []));
  questSound = $state(this.prefs.questSound ?? true);
  selection = $state<Selection>(null);
  hover = $state<Hover>({ agentId: null, taskId: null, path: null });

  paletteOpen = $state(false);
  paletteSeed = $state('');
  settingsOpen = $state(false);
  shortcutsOpen = $state(false);
  /** The model dialog: starts an agent of `provider` (agentId null) or switches `agentId`'s model. */
  modelDialog = $state<{ provider: Provider; agentId: string | null } | null>(null);

  /** Conversation thread on Home: a request thread ("lead" / "lead:<id>") or "agent:<id>" (1:1). */
  thread = $state('lead');
  /** The request you are on (the thread a 1:1 chat returns to). */
  request = $state('lead');
  /** Requests you opened (⌘N) that have no message yet: they show as tabs until you leave them empty. */
  draftRequests = $state<{ thread: string; ts: number }[]>([]);
  /** Bumped to focus the composer (New request, palette…). */
  composerFocus = $state(0);
  /** Plan card to scroll into view on Home (the pinned progress header). */
  revealPlan = $state<string | null>(null);
  planLens = $state<PlanLens>(this.prefs.planView && ['list', 'flow', 'timeline'].includes(this.prefs.planView) ? this.prefs.planView : 'list');
  changesLens = $state<ChangesLens>(this.prefs.changesLens ?? 'diffs');
  knowledgeLens = $state<KnowledgeLens>(this.prefs.knowledgeLens ?? 'memory');
  planId = $state<string | null>(null);
  diffTarget = $state<DiffTarget>(null);
  artifactOpen = $state<string | null>(null);
  terminalCwd = $state<string | null>(null);
  timelineZoom = $state(this.prefs.timelineZoom ?? 5);
  memoryEntity = $state<string | null>(null);
  /** ⇧⌘⌫ asks this approval card to open its "deny with a note" field. */
  denyNoteFor = $state<string | null>(null);
  /** Sheet section to scroll to when it opens (e.g. a subagent companion → "subagents"). */
  sheetFocus = $state<string | null>(null);
  /** Approval card to scroll into view on Home (toast "Review"). */
  revealApproval = $state<string | null>(null);
  /** Memories you pinned (local until the core has a pin command). */
  pinned = $state<string[]>(this.prefs.pinned ?? []);
  /** Plans whose branch you chose to keep instead of merging (hides the merge prompt). */
  keptPlans = $state<string[]>(this.prefs.keptPlans ?? []);

  /** Sheet sections: expanded state is remembered per section, not per agent. */
  sections = $state<Record<string, boolean>>({
    now: true,
    needs: true,
    controls: false,
    task: true,
    steps: true,
    files: false,
    subagents: true,
    output: false,
    cost: false,
    history: false,
    ...(this.prefs.sections ?? {}),
  });

  /** Composer draft injected by other surfaces (palette "Ask the Lead…", file mentions). */
  composerSeed = $state<{ thread: string; text: string; nonce: number } | null>(null);

  systemDark = $state(mq('(prefers-color-scheme: dark)')?.matches ?? true);
  reducedMotion = $state(mq('(prefers-reduced-motion: reduce)')?.matches ?? false);
  width = $state(typeof window !== 'undefined' ? window.innerWidth : 1440);

  /** Wall clock for relative times; ticks once per second while visible. */
  now = $state(wallNow());

  constructor() {
    mq('(prefers-color-scheme: dark)')?.addEventListener('change', (e) => (this.systemDark = e.matches));
    mq('(prefers-reduced-motion: reduce)')?.addEventListener('change', (e) => (this.reducedMotion = e.matches));
    if (typeof window !== 'undefined') {
      window.addEventListener('resize', () => (this.width = window.innerWidth));
      setInterval(() => {
        if (document.visibilityState === 'visible') this.now = wallNow();
      }, 1000);
    }
  }

  persist() {
    const p: Prefs = {
      room: this.room,
      mode: this.mode,
      focus: this.focus,
      rail: this.railOpen,
      onboarded: this.onboarded,
      quest: this.quest,
      questSound: this.questSound,
      sections: this.sections,
      planView: this.planLens,
      changesLens: this.changesLens,
      knowledgeLens: this.knowledgeLens,
      timelineZoom: this.timelineZoom,
      pinned: this.pinned,
      keptPlans: this.keptPlans,
    };
    try {
      localStorage.setItem(PREFS_KEY, JSON.stringify(p));
    } catch {
      /* private mode */
    }
  }

  // ───────────── navigation ─────────────

  go(room: Room) {
    this.room = room;
    if ((this.mode === 'office' && room !== 'home') || this.mode === 'map') this.mode = 'blend';
  }

  /** ⌘1 / ⌘2 / ⌘3: toggle full office / full work / the neighbourhood; again returns to the blend. */
  toggleMode(m: Exclude<Mode, 'blend'>) {
    this.mode = this.mode === m ? 'blend' : m;
    if (this.mode === 'office' || this.mode === 'map') this.room = 'home';
    if (this.mode === 'map') this.selection = null;
  }

  select(sel: Selection) {
    this.selection = sel;
  }
  selectAgent(id: string | null, eventId?: number) {
    this.select(id ? { kind: 'agent', id, eventId } : null);
  }
  selectTask(id: string | null, eventId?: number) {
    this.select(id ? { kind: 'task', id, eventId } : null);
  }
  selectFile(path: string | null) {
    this.select(path ? { kind: 'file', id: path } : null);
  }

  setHover(h: Partial<Hover> | null) {
    this.hover = h ? { agentId: h.agentId ?? null, taskId: h.taskId ?? null, path: h.path ?? null } : { agentId: null, taskId: null, path: null };
  }

  openThread(thread: string) {
    if (isRequestThread(thread)) return this.openRequest(thread);
    this.thread = thread;
    this.go('home');
    if (this.mode === 'office') this.mode = 'blend';
  }

  /** Switch to a request thread (its Lead, its plan, its approvals). Empty drafts you leave are dropped. */
  openRequest(thread: string) {
    // A draft you leave either has a message by now (the chat list carries it) or is dropped.
    this.draftRequests = this.draftRequests.filter((d) => d.thread === thread);
    this.request = thread;
    this.thread = thread;
    this.go('home');
    if (this.mode === 'office') this.mode = 'blend';
  }

  /** ⌘N — a fresh request thread with its own Lead, in parallel with the others. */
  newRequest() {
    const thread = `lead:${shortId()}`;
    this.draftRequests = [...this.draftRequests, { thread, ts: wallNow() }];
    this.openRequest(thread);
    this.composerFocus++;
  }

  openDiff(target: DiffTarget) {
    this.diffTarget = target;
    this.changesLens = 'diffs';
    this.go('changes');
  }

  openArtifact(id: string) {
    this.artifactOpen = id;
    this.changesLens = 'artifacts';
    this.go('changes');
  }

  /** The Plan room on a plan (and on the request it answers, when given). */
  openPlan(planId?: string | null, thread?: string | null) {
    if (planId) this.planId = planId;
    if (thread && isRequestThread(thread)) this.request = thread;
    this.go('plan');
  }

  openPalette(seed = '') {
    this.paletteSeed = seed;
    this.paletteOpen = true;
  }

  toggleSection(id: string) {
    this.sections = { ...this.sections, [id]: !(this.sections[id] ?? true) };
  }

  togglePin(id: string) {
    this.pinned = this.pinned.includes(id) ? this.pinned.filter((x) => x !== id) : [...this.pinned, id];
  }

  keepPlan(id: string) {
    if (!this.keptPlans.includes(id)) this.keptPlans = [...this.keptPlans, id];
  }

  seedComposer(thread: string, text: string) {
    if (isRequestThread(thread)) this.request = thread;
    this.thread = thread;
    this.composerSeed = { thread, text, nonce: Math.random() };
    this.go('home');
    if (this.mode === 'office') this.mode = 'blend';
  }

  selectedAgentId = $derived(this.selection?.kind === 'agent' ? this.selection.id : null);
  selectedTaskId = $derived(this.selection?.kind === 'task' ? this.selection.id : null);
  selectedPath = $derived(this.selection?.kind === 'file' ? this.selection.id : null);
  sheetOpen = $derived(this.selection != null);
}

export const ui = new Ui();
