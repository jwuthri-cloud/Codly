// MockTransport: the browser-only backend used by `pnpm dev` (no Tauri). A scripted session on
// a virtual clock (demo/story.ts) that also answers every command, so each panel is demonstrable
// without the Rust core. Never loaded inside the Tauri app (see api.ts).
//
// ponytail: one scripted story (dark-mode toggle on acme/web); any repo path replays it.
// URL knobs (dev only): ?speed=2 (script speed), ?skip=90 (fast-forward N seconds of script),
// ?autoplay=1 (auto "Run plan" and auto-approve), ?ws=1 (open acme/web immediately; always on in
// the public site demo).
// Also answers `repo_map` (a ~300-file acme/web) and `read_artifact` (the bytes agents produced).
import { isSiteDemo, type Transport, type UiBatch } from './api';
import type { BreakerConfig } from './gen/BreakerConfig';
import type { CliInfo } from './gen/CliInfo';
import type { Placed } from './gen/Placed';
import type { Rank } from './gen/Rank';
import type { InstallHint } from './gen/InstallHint';
import type { InstallRung } from './gen/InstallRung';
import type { ByokStatus } from './gen/ByokStatus';
import type { MemoryKind } from './gen/MemoryKind';
import type { MemoryScope } from './gen/MemoryScope';
import type { Provider } from './gen/Provider';
import type { SearchResult } from './gen/SearchResult';
import type { Settings } from './gen/Settings';
import type { Snapshot } from './gen/Snapshot';
import type { TaskStatus } from './gen/TaskStatus';
import type { TeamMember } from './gen/TeamMember';
import type { WorkspaceTeam } from './gen/WorkspaceTeam';
import type { RequestPrefs } from './gen/RequestPrefs';
import { mockAgentGit } from './demo/agentgit';
import { MockAskMe } from './demo/askme';
import type { HireManifest } from './gen/HireManifest';
import { demoDraft, exportDemoAgent, hireFromManifest, launchPreview, reviewHire } from './demo/hire';
import { makeApplicants } from './util/career';
import { demoBiggerModel, demoBringBack, demoBuyLot, demoLetGo, demoDigest, demoExportOffice, demoGhosts, demoGiveXp, demoGrant, demoHangout, demoHome, demoLook, demoLot, demoLotOf, demoLots, demoMoveOffice, demoOffices, demoPlace, demoResetOffice, demoRest, demoSaveLot, demoSell, demoSellLot, demoShop, demoTired, demoTransfer, demoTrust, demoWallet } from './demo/office';
import type { Lot } from './gen/Lot';
import { HousekeepingDemo } from './demo/housekeeping';
import { REPO_LS } from './demo/repo';
import { MockBreaker } from './demo/breaker';
import { repoMap } from './demo/repomap';
import { HOME, REPO, Sim } from './demo/sim';
import { MockControls } from './demo/controls';
import { Story } from './demo/story';
import { saveTriggers, seedTriggers } from './demo/triggers';
import type { TriggersConfig } from './gen/TriggersConfig';
import { skillsMock } from './demo/skillsMock';
import { mockIde } from '../ide/mockFs';
import { buildMessageGraph, exactSearch } from './demo/memgraph';
import type { MemoryFile } from './gen/MemoryFile';

/** The demo's per-agent memory files (the core keeps them in <data>/memory/<agent id>.md). */
const memoryFiles = new Map<string, MemoryFile>();
import { inboundMock } from './demo/inbound';

/** The core's trusted install table (crates/codly-core/src/install.rs), for the browser demo. */
const INSTALL: Record<Provider, InstallHint> = {
  claude: { npm: 'npm install -g @anthropic-ai/claude-code', native: 'curl -fsSL https://claude.ai/install.sh | bash', login: 'claude auth login', docs: 'https://docs.claude.com/en/docs/claude-code/setup' },
  codex: { npm: 'npm install -g @openai/codex', native: null, login: 'codex login', docs: 'https://github.com/openai/codex' },
  gemini: { npm: 'npm install -g @google/gemini-cli', native: null, login: 'gemini', docs: 'https://github.com/google-gemini/gemini-cli' },
  opencode: { npm: 'npm install -g opencode-ai@latest', native: 'curl -fsSL https://opencode.ai/install | bash', login: null, docs: 'https://opencode.ai/docs' },
  cursor: { npm: null, native: 'curl https://cursor.com/install -fsS | bash', login: null, docs: 'https://cursor.com/cli' },
  copilot: { npm: 'npm install -g @github/copilot', native: null, login: null, docs: 'https://github.com/github/copilot-cli' },
};

/** `?onboard=1`: a machine with none of the CLIs installed, so the quest has something to do. */
const fresh = (c: CliInfo): CliInfo => ({ ...c, path: null, version: null, logged_in: null, billing: 'unknown', plan: null, problem: `Not found on PATH: install \`${c.provider}\`.` });

const CLIS: CliInfo[] = [
  { provider: 'claude', path: '/opt/homebrew/bin/claude', version: '2.1.309', logged_in: true, billing: 'subscription', plan: 'Team', problem: null, transport: 'stream-json', install: INSTALL.claude, default_model: null },
  { provider: 'codex', path: '/opt/homebrew/bin/codex', version: '0.161.0', logged_in: true, billing: 'subscription', plan: 'Plus', problem: null, transport: 'app-server', install: INSTALL.codex, default_model: null },
  { provider: 'gemini', path: '/opt/homebrew/bin/gemini', version: '0.62.0', logged_in: true, billing: 'subscription', plan: 'Google AI Pro', problem: null, transport: 'acp', install: INSTALL.gemini, default_model: null },
  { provider: 'opencode', path: `${HOME}/.opencode/bin/opencode`, version: '1.18.31', logged_in: null, billing: 'third_party', plan: 'OpenRouter', problem: null, transport: 'acp', install: INSTALL.opencode, default_model: null },
  { provider: 'copilot', path: '/opt/homebrew/bin/copilot', version: '1.4.2', logged_in: false, billing: 'unknown', plan: null, problem: 'Not signed in: run `copilot` once and use /login.', transport: 'acp', install: INSTALL.copilot, default_model: null },
  { provider: 'cursor', path: null, version: null, logged_in: null, billing: 'unknown', plan: null, problem: 'Not found on PATH. Install Cursor’s CLI (`cursor-agent`) to use it here.', transport: 'acp', install: INSTALL.cursor, default_model: null },
];

const SETTINGS: Settings = {
  lead_provider: 'claude',
  lead_model: null,
  worker_providers: ['claude', 'codex', 'gemini', 'opencode'],
  max_parallel: 3,
  max_per_provider: 2,
  // Two Claude subscriptions (your terminal's and a work one) and a Codex one not signed in yet.
  accounts: [
    { provider: 'claude', home: null, label: null, enabled: true },
    { provider: 'claude', home: '~/.claude-work', label: null, enabled: true },
    { provider: 'codex', home: null, label: null, enabled: true },
    { provider: 'codex', home: '~/.codex-perso', label: null, enabled: true },
  ],
  account_strategy: 'spread',
  lead_account: null,
  approval_mode: 'balanced',
  auto_run_plans: false,
  review_gate: false,
  lead_followups: true,
  use_my_claude_setup: true,
  paused: false,
  paused_offices: [],
  developer_mode: false,
  world_ambient: false,
  theme: 'system',
  team: [],
  byok_openrouter: false,
};

/** Like the core: a member without an id gets "<slug>-<4 hex>". */
const withIds = (team: TeamMember[]): TeamMember[] =>
  team.map((m) => (m.id ? m : { ...m, id: `${m.name.toLowerCase().replace(/[^a-z0-9]+/g, '-').slice(0, 12)}-${Math.random().toString(16).slice(2, 6)}` }));

const params = () => new URLSearchParams(typeof location !== 'undefined' ? location.search : '');
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export class MockTransport implements Transport {
  async upload(_cmd: string, name: string): Promise<string> {
    return `/demo/acme-web/.codly/attachments/${Math.random().toString(36).slice(2, 8)}/${name}`;
  }
  private sim: Sim;
  private story: Story;
  private ctl: MockControls;
  private ptys = new Map<string, FakeShell>();
  private ptySeq = 0;
  /** Free Flow demo state: on with a key saved, so the composer's mic shows. */
  private dictation = { enabled: true, model: 'whisper-large-v3-turbo', models: ['whisper-large-v3-turbo', 'whisper-large-v3'], key_saved: true, secure_storage: true };
  /** BYOK demo state. The key is never stored, only whether one was "saved". */
  private byok = { key_saved: false };
  private triggers: TriggersConfig | null = null;
  private breaker: MockBreaker;
  private askMe!: MockAskMe;
  private hk: HousekeepingDemo | undefined;

  constructor() {
    const p = params();
    const theme = (() => {
      try {
        return localStorage.getItem('codly.theme') ?? 'system';
      } catch {
        return 'system';
      }
    })();
    this.sim = new Sim(
      { speed: Number(p.get('speed') ?? 1) || 1, skipMs: (Number(p.get('skip') ?? 0) || 0) * 1000, autoplay: p.get('autoplay') === '1' },
      { ...SETTINGS, theme },
    );
    this.story = new Story(this.sim);
    this.breaker = new MockBreaker(this.sim);
    this.ctl = new MockControls(this.sim);
    this.askMe = new MockAskMe(this.sim);
    const now = this.sim.now();
    this.sim.clis = p.get('onboard') === '1' ? CLIS.map(fresh) : CLIS;
    this.sim.recent = [
      { repo_path: REPO, name: 'acme/web', opened_at: now - 2 * 3_600_000 },
      { repo_path: `${HOME}/Code/billing-service`, name: 'billing-service', opened_at: now - 26 * 3_600_000 },
      { repo_path: `${HOME}/dotfiles`, name: 'dotfiles', opened_at: now - 6 * 86_400_000 },
    ];
    // Let the UI's relative times follow the virtual clock (dev only).
    (globalThis as { __codlyNow?: () => number }).__codlyNow = () => this.sim.now();
    if (p.get('ws') === '1' || p.get('skip') || isSiteDemo()) this.story.openWorkspace(REPO);
  }

  async subscribe(onBatch: (b: UiBatch) => void): Promise<Snapshot> {
    this.sim.flush();
    this.sim.sink = onBatch;
    await wait(40);
    return this.sim.snapshot();
  }

  async attachPty(ptyId: string, onData: (bytes: Uint8Array) => void): Promise<void> {
    const sh = this.ptys.get(ptyId);
    if (!sh) throw new Error(`No terminal ${ptyId}`);
    sh.attach(onData);
  }

  async call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    // A little latency makes loading states honest; terminals stay instant.
    if (!cmd.startsWith('pty_')) await wait(60 + Math.random() * 80);
    // Re-probing sign-ins runs each CLI's status command: it takes a moment.
    if (cmd === 'list_accounts' && args.refresh) await wait(900);
    const skills = await skillsMock(cmd, args);
    if (skills !== undefined) return skills as T;
    return this.dispatch(cmd, args) as T;
  }

  private byokStatus(): ByokStatus {
    return { enabled: this.sim.settings.byok_openrouter, key_saved: this.byok.key_saved };
  }

  private dispatch(cmd: string, a: Record<string, unknown>): unknown {
    const sim = this.sim;
    const story = this.story;
    switch (cmd) {
      case 'ui_set_visible':
        return null;
      case 'detect_clis':
        // Like the core: OpenCode's badge says so while your own OpenRouter key is in use.
        return sim.clis.map((c) => (c.provider === 'opencode' && sim.settings.byok_openrouter && this.byok.key_saved ? { ...c, billing: 'third_party' as const, plan: 'OpenRouter · your key' } : c));
      case 'list_accounts':
        return sim.signIns.list(sim.settings, sim.clis);
      case 'recent_workspaces':
        return sim.recent;
      case 'list_repos':
        return sim.recent.map((r) => ({ name: r.name, path: r.repo_path, modified_ms: r.opened_at }));
      case 'open_workspace':
        return story.openWorkspace(String(a.repoPath ?? REPO));
      case 'update_settings': {
        const next = sim.signIns.normalize(a.settings as Settings);
        next.max_parallel = Math.min(8, Math.max(1, next.max_parallel));
        next.max_per_provider = Math.min(next.max_parallel, Math.max(1, next.max_per_provider));
        next.team = withIds(next.team ?? []);
        sim.settings = next;
        story.syncTeam();
        return sim.settings;
      }
      case 'set_request_prefs':
        sim.requestPrefs[String(a.thread)] = a.prefs as RequestPrefs;
        return a.prefs;
      case 'set_workspace_team': {
        const t = a.team as WorkspaceTeam;
        sim.workspaceTeam = { off: t.off, members: withIds(t.members) };
        story.syncTeam();
        return sim.workspaceTeam;
      }
      case 'send_chat': {
        if (!sim.workspace) throw new Error('Open a repository first.');
        story.userChat(String(a.thread), String(a.text));
        return null;
      }
      case 'run_plan':
        story.runPlan(String(a.planId));
        return null;
      case 'cancel_plan':
        story.cancelPlan(String(a.planId));
        return null;
      case 'close_thread':
        story.closeThread(String(a.thread));
        return null;
      case 'merge_plan':
        return story.mergePlan(String(a.planId));
      case 'keep_plan':
      case 'pin_memory':
        return null; // ponytail: the mock keeps these as UI state only
      case 'resolve_approval': {
        const d = a.decision as { choice: string; message: string | null; answers: unknown };
        story.resolveApproval(String(a.approvalId), d.choice, d.message, d.answers);
        return null;
      }
      case 'breaker_config':
        return this.breaker.cfg;
      case 'set_breaker_config':
        return this.breaker.set(a.config as BreakerConfig);
      case 'breaker_status':
        return this.breaker.status();
      case 'reset_breaker':
        this.breaker.reset();
        return null;
      case 'interrupt_agent':
        story.interrupt(String(a.agentId));
        return null;
      case 'stop_agent':
        story.interrupt(String(a.agentId), true);
        this.ctl.clear(String(a.agentId));
        return null;
      case 'agent_control':
        return this.ctl.state(String(a.agentId));
      case 'set_agent_block_tools':
        return this.ctl.setBlockTools(String(a.agentId), !!a.on);
      case 'stop_agent_after_step':
        return this.ctl.stopAfterStep(String(a.agentId));
      case 'resume_agent':
        return this.ctl.resume(String(a.agentId));
      case 'steer_agent':
        return this.ctl.steer(String(a.agentId), String(a.text ?? ''));
      case 'hire_agent':
        return story.hire(a.provider as Provider, (a.model as string | null) ?? null, (a.title as string | null) ?? null);
      case 'list_models':
        return a.provider === 'claude' ? ['sonnet', 'opus', 'haiku'] : a.provider === 'codex' ? ['gpt-5.5-codex', 'gpt-5.5-mini'] : [];
      case 'hire_review':
        return reviewHire(String(a.raw));
      case 'hire_launch_preview':
        return launchPreview(String(a.provider), (a.model as string | null) ?? null, (a.command as string | null) ?? null);
      case 'hire_draft':
        return reviewHire(demoDraft(String(a.brief)));
      case 'hire_from_manifest':
        return hireFromManifest(sim, story, a.manifest as HireManifest, Boolean(a.confirmShell));
      case 'hire_applicants': {
        const all = [...sim.agents.values()].filter((x) => !x.archived);
        return makeApplicants(Number(a.n ?? 3), all.map((x) => x.name), all.map((x) => x.color));
      }
      case 'pick_applicant':
        return null;
      case 'export_agent':
        return exportDemoAgent(sim, String(a.agentId));
      case 'export_office':
        return demoExportOffice(sim);
      case 'office_digest':
        return demoDigest(sim, Number(a.since));
      case 'set_agent_look':
        return demoLook(sim, String(a.agentId), String(a.lookSeed));
      case 'trust_agent':
        return demoTrust(sim, String(a.agentId), a.rank as Rank);
      case 'set_agent_home':
        return demoHome(sim, String(a.agentId), (a.until as number | null) ?? null);
      case 'note_rest':
        return demoRest(sim, String(a.agentId), Boolean(a.sick));
      case 'note_hangout':
        return demoHangout(sim, String(a.a), String(a.b));
      case 'set_agent_tired':
        return demoTired(sim, String(a.agentId), Boolean(a.tired));
      case 'bigger_model':
        return demoBiggerModel(sim, String(a.agentId));
      case 'ghosts':
        return demoGhosts(sim);
      case 'bring_back':
        return demoBringBack(sim, String(a.agentId));
      case 'let_go':
        return demoLetGo(sim, String(a.agentId));
      case 'offices':
        return demoOffices();
      case 'transfer_agent':
        return demoTransfer(sim, String(a.agentId), String(a.toWorkspaceId));
      case 'office_shop':
        return demoShop(sim);
      case 'office_place':
        return demoPlace(sim, a.piece as Placed);
      case 'office_sell':
        return demoSell(sim, String(a.uid));
      case 'wallet':
        return demoWallet(sim);
      case 'dev_grant':
        return demoGrant(sim, Number(a.amount));
      case 'dev_give_xp':
        return demoGiveXp(sim, String(a.agentId), Number(a.xp));
      case 'dev_reset_office':
        return demoResetOffice(sim);
      case 'office_lot':
        return demoLot();
      case 'office_save_lot':
        return demoSaveLot(sim, a.lot as Lot);
      case 'lots':
        return demoLots(sim);
      case 'lot_of':
        return demoLotOf(String(a.slotId));
      case 'buy_lot':
        return demoBuyLot(sim, String(a.slotId), (a.template as string | null) ?? null);
      case 'sell_lot':
        return demoSellLot(sim, String(a.slotId));
      case 'move_office':
        return demoMoveOffice(sim, String(a.workspaceId), String(a.slotId));
      case 'set_agent_model':
        return null;
      case 'create_task':
        return story.createTask(String(a.title), String(a.goal ?? ''));
      case 'move_task':
        return story.moveTask(String(a.taskId), a.status as TaskStatus);
      case 'assign_task':
        return story.assignTask(String(a.taskId), (a.agentId as string | null) ?? null);
      case 'task_diff':
        return story.taskDiff(String(a.taskId));
      case 'plan_diff':
        return story.planDiff(String(a.planId));
      case 'get_events': {
        const agentId = a.agentId as string | null;
        const before = a.before as number | null;
        const limit = Number(a.limit ?? 500);
        return sim.log
          .filter((e) => (!agentId || e.agent_id === agentId) && (before == null || e.id < before))
          .slice(-limit)
          .reverse();
      }
      case 'search_code':
        return this.search(String(a.query ?? ''));
      case 'list_memories': {
        const q = (a.query as string | null)?.toLowerCase() ?? '';
        const scope = a.scope as MemoryScope | null;
        return [...sim.memories.values()].filter((m) => (!scope || m.scope === scope) && (!q || m.text.toLowerCase().includes(q)));
      }
      case 'add_memory':
        return Object.assign(sim.memory(String(a.text), a.scope as MemoryScope, a.kind as MemoryKind, 'user', { importance: 0.7 }), { title: String(a.title ?? ''), tags: (a.tags as string[] | null) ?? [] });
      case 'delete_memory':
        sim.memories.delete(String(a.id));
        return null;
      case 'repo_map':
        return sim.workspace ? repoMap(sim.committedKeys) : [];
      case 'read_artifact': {
        const bytes = sim.artifactBytes.get(String(a.artifactId));
        if (!bytes) throw new Error(`Unknown artifact ${String(a.artifactId)}`);
        return bytes.slice(0);
      }
      case 'exact_search':
        return exactSearch(String(a.query ?? ''), [...sim.tasks.values()], [...sim.memories.values()]);
      case 'message_graph':
        return buildMessageGraph([...sim.agents.values()], sim.chats, [...sim.memories.values()], [...sim.tasks.values()]);
      case 'memory_file':
      case 'save_memory_file': {
        const agent = sim.agents.get(String(a.agentId));
        if (!agent || !/^[A-Za-z0-9_-]{1,64}$/.test(String(a.agentId))) throw new Error('unknown agent');
        const prev = memoryFiles.get(agent.id);
        if (cmd === 'memory_file' && prev) return prev;
        const text = cmd === 'save_memory_file' ? String(a.text) : `# ${agent.name}: memory\n\nWhat ${agent.name} should always know. Codly adds this file to ${agent.name}'s brief at the start of every task.\n\n- \n`;
        const f: MemoryFile = { agent_id: agent.id, path: `memory/${agent.id}.md`, text, exists: cmd === 'save_memory_file', modified_at: cmd === 'save_memory_file' ? sim.now() : null };
        if (cmd === 'save_memory_file') memoryFiles.set(agent.id, f);
        return f;
      }
      case 'list_triples': {
        const e = a.entity as string | null;
        return sim.triples.filter((t) => !e || t.subject === e || t.object === e);
      }
      case 'pty_open': {
        const id = `pty-${++this.ptySeq}`;
        this.ptys.set(id, new FakeShell(String(a.cwd ?? sim.workspace?.repo_path ?? REPO), sim));
        return id;
      }
      case 'pty_write':
        this.ptys.get(String(a.ptyId))?.input(String(a.data));
        return null;
      case 'pty_resize':
        return null;
      case 'pty_close':
        this.ptys.get(String(a.ptyId))?.close();
        this.ptys.delete(String(a.ptyId));
        return null;
      case 'dictation_status':
        return { ...this.dictation };
      case 'dictation_configure':
        if (typeof a.enabled === 'boolean') this.dictation.enabled = a.enabled;
        if (typeof a.model === 'string') this.dictation.model = a.model;
        return { ...this.dictation };
      case 'dictation_save_key':
        if (!/^gsk_[\w-]{8,}$/.test(String(a.key).trim())) throw new Error('that does not look like a Groq API key (it starts with gsk_)');
        this.dictation.key_saved = true;
        return { ...this.dictation };
      case 'dictation_clear_key':
        this.dictation.key_saved = false;
        return { ...this.dictation };
      case 'dictation_transcribe':
        return 'Add retry with backoff to the sync worker, and cover it with a test.';
      case 'install_cli': {
        // Only the table's constants run, chosen by (provider, rung), as in the core.
        const hint = INSTALL[a.provider as Provider];
        const rung = a.rung as InstallRung;
        const command = rung === 'npm' || rung === 'native' ? hint?.[rung] : undefined;
        if (!command) throw new Error(`there is no ${String(a.rung)} installer for ${String(a.provider)}`);
        const id = `pty-${++this.ptySeq}`;
        this.ptys.set(id, new FakeShell(HOME, sim));
        // The demo "finishes" the install after a moment, then the sign-in a little later.
        const real = CLIS.find((c) => c.provider === a.provider);
        const idx = sim.clis.findIndex((c) => c.provider === a.provider);
        if (real && idx >= 0 && !sim.clis[idx].path) {
          setTimeout(() => (sim.clis = sim.clis.map((c) => (c.provider === a.provider ? { ...real, logged_in: false, billing: 'unknown', plan: null, problem: `Not signed in: run \`${a.provider}\` and sign in.` } : c))), 2500);
          setTimeout(() => (sim.clis = sim.clis.map((c) => (c.provider === a.provider ? real : c))), 8000);
        }
        return { pty_id: id, command };
      }
      case 'byok_status':
        return this.byokStatus();
      case 'byok_configure':
        sim.settings = { ...sim.settings, byok_openrouter: a.enabled === true };
        return this.byokStatus();
      case 'byok_save_key':
        if (!/^[\w.-]{1,256}$/.test(String(a.key).trim())) throw new Error('the key may only contain letters, digits, - _ and .');
        this.byok.key_saved = true;
        return this.byokStatus();
      case 'byok_clear_key':
        this.byok.key_saved = false;
        return this.byokStatus();
      case 'agent_git':
        return mockAgentGit(sim.agents.get(String(a.agentId)), sim.now());
      case 'get_triggers':
        return (this.triggers ??= seedTriggers());
      case 'set_triggers':
        return (this.triggers = saveTriggers((this.triggers ??= seedTriggers()), a.config as TriggersConfig));
      case 'ide_info':
      case 'ide_list':
      case 'ide_read':
      case 'ide_write':
      case 'ide_create':
      case 'ide_search':
      case 'ide_changes':
      case 'ide_diff':
        return mockIde(cmd, a);
      case 'list_blockers':
        return this.askMe.list();
      case 'answer_blocker':
        return this.askMe.close(String(a.blockerId), 'answered', String(a.answer ?? ''));
      case 'dismiss_blocker':
        return this.askMe.close(String(a.blockerId), 'dismissed', null);
      case 'inbound_state':
      case 'inbound_create_webhook':
      case 'inbound_set_webhook':
      case 'inbound_revoke_webhook':
      case 'inbound_set_settings':
      case 'inbound_set_slack':
        return inboundMock(cmd, a);
      case 'board_policy':
      case 'board_sweep':
      case 'board_archive':
      case 'list_connections':
      case 'update_connection':
        return (this.hk ??= new HousekeepingDemo(sim.now())).handle(cmd, a, sim.tasks.values(), sim.now());
      default:
        throw new Error(`Mock backend: unknown command “${cmd}”`);
    }
  }

  private search(query: string): SearchResult {
    const terms = query.toLowerCase().split(/[^a-z0-9_.-]+/).filter((t) => t.length > 1);
    const scored = this.story
      .searchIndex()
      .map((e) => {
        let score = 0;
        const why: string[] = [];
        const path = e.path.toLowerCase();
        const sym = (e.symbol ?? '').toLowerCase();
        for (const t of terms) {
          if (sym && sym === t) { score += 8; why.push(`symbol: ${e.symbol}`); }
          else if (sym.includes(t)) { score += 5; why.push(`symbol: ${e.symbol}`); }
          if (path.includes(t)) { score += 3; why.push(`path: ${t}`); }
          if (e.text.includes(t)) { score += 1.2; why.push(`text: ${t}`); }
        }
        return { e, score: Math.round(score * 10) / 10, why: [...new Set(why)].slice(0, 3) };
      })
      .filter((x) => x.score > 0)
      .sort((x, y) => y.score - x.score)
      .slice(0, 8);
    const top = scored[0]?.score ?? 0;
    const exact = scored.some((x) => x.e.symbol && terms.includes(x.e.symbol.toLowerCase()));
    const confidence = exact ? 'exact' : top >= 9 ? 'high' : top >= 5 ? 'medium' : 'low';
    return {
      query,
      confidence,
      citations: scored.map((x, i) => ({ id: `C${i + 1}`, path: x.e.path, start_line: x.e.start, end_line: x.e.end, symbol: x.e.symbol, why: x.why, score: x.score })),
      fallback_rg: confidence === 'low' && terms.length ? [`rg -n "${terms.join('|')}" src`] : [],
      took_ms: 4 + Math.round(Math.random() * 14),
    };
  }
}

// ───────────── a tiny fake shell for the Terminal panel ─────────────

const enc = new TextEncoder();
const C = { dim: '\x1b[2m', reset: '\x1b[0m', blue: '\x1b[38;5;110m', green: '\x1b[32m', red: '\x1b[31m', grey: '\x1b[38;5;245m', bold: '\x1b[1m' };

class FakeShell {
  private out: ((b: Uint8Array) => void) | null = null;
  private line = '';
  private pending = '';
  constructor(private cwd: string, private sim: Sim) {}

  private write(s: string) {
    if (this.out) this.out(enc.encode(s));
    else this.pending += s;
  }

  private get prompt() {
    const dir = this.cwd.replace(/^\/Users\/[^/]+/, '~');
    const branch = this.cwd.includes('.codly/worktrees') ? `codly/${this.cwd.split('/').pop()}` : 'main';
    return `${C.grey}${dir}${C.reset} ${C.blue}${branch}${C.reset} ❯ `;
  }

  attach(out: (b: Uint8Array) => void) {
    this.out = out;
    this.write(`${C.dim}Codly mock terminal — the browser has no shell. Try: ls, git status, git log, npm test, help${C.reset}\r\n\r\n`);
    if (this.pending) {
      const p = this.pending;
      this.pending = '';
      this.write(p);
    }
    this.write(this.prompt);
  }

  close() {
    this.out = null;
  }

  input(data: string) {
    for (const ch of data) {
      if (ch === '\r') {
        this.write('\r\n');
        this.run(this.line.trim());
        this.line = '';
      } else if (ch === '\x7f') {
        if (this.line) {
          this.line = this.line.slice(0, -1);
          this.write('\b \b');
        }
      } else if (ch === '\x03') {
        this.line = '';
        this.write('^C\r\n' + this.prompt);
      } else if (ch === '\x0c') {
        this.write('\x1b[2J\x1b[H' + this.prompt + this.line);
      } else if (ch >= ' ' && ch !== '\x1b' && ch.length === 1) {
        this.line += ch;
        this.write(ch);
      }
    }
  }

  private run(cmd: string) {
    const say = (s: string) => this.write(s.replace(/\n/g, '\r\n') + '\r\n');
    const [bin, ...rest] = cmd.split(/\s+/);
    const done = () => this.write(this.prompt);
    if (!cmd) return done();
    switch (bin) {
      case 'help':
        say('ls · pwd · whoami · git status · git log · git branch · npm test · clear · echo');
        break;
      case 'ls':
        say(REPO_LS.map((f) => (/\./.test(f) ? f : `${C.blue}${f}${C.reset}`)).join('  '));
        break;
      case 'pwd':
        say(this.cwd);
        break;
      case 'whoami':
        say('demo');
        break;
      case 'echo':
        say(rest.join(' '));
        break;
      case 'clear':
        this.write('\x1b[2J\x1b[H');
        break;
      case 'git': {
        const sub = rest[0];
        const tasks = [...this.sim.tasks.values()];
        if (sub === 'status') say(`On branch main\nYour branch is up to date with 'origin/main'.\n\nnothing to commit, working tree clean`);
        else if (sub === 'log')
          say(`${C.bold}4e1c9a2${C.reset} chore: bump next to 16.1.2\n${C.bold}b81d07e${C.reset} fix(checkout): retry stripe webhook once\n${C.bold}9a0c3f1${C.reset} feat(settings): weekly digest switch\n${C.bold}61e2d48${C.reset} refactor(auth): requireUser helper`);
        else if (sub === 'branch')
          say(['* main', ...tasks.filter((t) => t.branch).map((t) => `  ${t.branch}`), ...[...this.sim.plans.values()].filter((p) => p.integration_branch).map((p) => `  ${p.integration_branch}`)].join('\n'));
        else say(`${C.dim}(mock) git ${rest.join(' ')}${C.reset}`);
        break;
      }
      case 'npm':
      case 'pnpm':
        if (rest[0] === 'test' || rest[0] === 't') {
          say(`\n> acme-web@2.4.0 test\n> vitest run\n\n ${C.green}✓${C.reset} src/server/settings/route.test.ts (4 tests) 61ms\n ${C.green}✓${C.reset} src/theme/ThemeProvider.test.tsx (6 tests) 40ms\n\n ${C.green}Test Files  9 passed (9)${C.reset}\n      Tests  41 passed (41)\n   Duration  2.98s`);
        } else say(`${C.dim}(mock) ${cmd}${C.reset}`);
        break;
      case 'exit':
        say('logout');
        this.close();
        return;
      default:
        say(`zsh: command not found: ${bin}`);
    }
    done();
  }
}
