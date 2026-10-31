// Typed client for the Codly core. Two transports, one interface:
// - Tauri (the real app): commands + one ordered Channel of event batches.
// - Mock (plain browser, `pnpm dev`): a scripted demo session for UI work and tests.
//
// CONTRACT: command names and argument keys match src-tauri/src/commands.rs.
import type { AccountInfo } from './gen/AccountInfo';
import type { Agent } from './gen/Agent';
import type { Rank } from './gen/Rank';
import type { Digest } from './gen/Digest';
import type { DictationStatus } from './gen/DictationStatus';
import type { AgentGit } from './gen/AgentGit';
import type { Blocker } from './gen/Blocker';
import type { ApprovalDecision } from './gen/ApprovalDecision';
import type { BreakerConfig } from './gen/BreakerConfig';
import type { BreakerStatus } from './gen/BreakerStatus';
import type { ArchivedItem } from './gen/ArchivedItem';
import type { ArchivePolicy } from './gen/ArchivePolicy';
import type { Connection } from './gen/Connection';
import type { ConnectionSecret } from './gen/ConnectionSecret';
import type { SweepReport } from './gen/SweepReport';
import type { ByokStatus } from './gen/ByokStatus';
import type { CliInfo } from './gen/CliInfo';
import type { InstallRun } from './gen/InstallRun';
import type { InstallRung } from './gen/InstallRung';
import type { ControlState } from './gen/ControlState';
import type { EventRow } from './gen/EventRow';
import type { IdeChange } from './gen/IdeChange';
import type { IdeEntry } from './gen/IdeEntry';
import type { IdeFile } from './gen/IdeFile';
import type { IdeInfo } from './gen/IdeInfo';
import type { IdeSearch } from './gen/IdeSearch';
import type { InboundState } from './gen/InboundState';
import type { HireManifest } from './gen/HireManifest';
import type { Applicant } from './gen/Applicant';
import type { Placed } from './gen/Placed';
import type { Shop } from './gen/Shop';
import type { HireReview } from './gen/HireReview';
import type { LaunchPreview } from './gen/LaunchPreview';
import type { Memory } from './gen/Memory';
import type { MemoryKind } from './gen/MemoryKind';
import type { MemoryScope } from './gen/MemoryScope';
import type { NewWebhook } from './gen/NewWebhook';
import type { Plan } from './gen/Plan';
import type { Provider } from './gen/Provider';
import type { RecentWorkspace } from './gen/RecentWorkspace';
import type { RepoFile } from './gen/RepoFile';
import type { CatalogSkill } from './gen/CatalogSkill';
import type { Skill } from './gen/Skill';
import type { SearchResult } from './gen/SearchResult';
import type { Settings } from './gen/Settings';
import type { SlackUpdate } from './gen/SlackUpdate';
import type { Snapshot } from './gen/Snapshot';
import type { Task } from './gen/Task';
import type { TaskStatus } from './gen/TaskStatus';
import type { Triple } from './gen/Triple';
import type { TriggersConfig } from './gen/TriggersConfig';
import type { Workspace } from './gen/Workspace';
import type { WorkspaceTeam } from './gen/WorkspaceTeam';
import type { RequestPrefs } from './gen/RequestPrefs';
import type { ExactHit } from './gen/ExactHit';
import type { MemoryFile } from './gen/MemoryFile';
import type { MessageGraph } from './gen/MessageGraph';

export type UiBatch = { events: EventRow[] };

export interface Transport {
  /** Install the batch sink first, then resolve with a snapshot. Drop events with id <= snapshot.as_of (id 0 = ephemeral, always apply). */
  subscribe(onBatch: (b: UiBatch) => void): Promise<Snapshot>;
  call<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  /** Send raw bytes (a pasted or dropped file); resolves with the command's string result. */
  upload(cmd: string, name: string, bytes: ArrayBuffer): Promise<string>;
  /** Raw PTY bytes for one terminal. */
  attachPty(ptyId: string, onData: (bytes: Uint8Array) => void): Promise<void>;
}

class TauriTransport implements Transport {
  async subscribe(onBatch: (b: UiBatch) => void): Promise<Snapshot> {
    const { Channel, invoke } = await import('@tauri-apps/api/core');
    const ch = new Channel<UiBatch>(onBatch);
    return invoke<Snapshot>('ui_subscribe', { onBatch: ch });
  }
  async call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(cmd, args);
  }
  async upload(cmd: string, name: string, bytes: ArrayBuffer): Promise<string> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<string>(cmd, new Uint8Array(bytes), { headers: { 'x-name': encodeURIComponent(name) } });
  }
  async attachPty(ptyId: string, onData: (bytes: Uint8Array) => void): Promise<void> {
    const { Channel, invoke } = await import('@tauri-apps/api/core');
    const ch = new Channel<ArrayBuffer>((buf) => onData(new Uint8Array(buf)));
    await invoke('pty_attach', { ptyId, onData: ch });
  }
}

export const isTauriApp = (): boolean =>
  typeof window !== 'undefined' && !!(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

/** The public web demo (`pnpm build:site` → /demo/): the scripted fake repo, opened at once. */
export const isSiteDemo = (): boolean => import.meta.env.VITE_SITE_DEMO === '1' && !isTauriApp();

let transport: Transport | null = null;

export async function getTransport(): Promise<Transport> {
  if (transport) return transport;
  if (isTauriApp()) transport = new TauriTransport();
  else transport = new (await import('./mock')).MockTransport();
  return transport;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return (await getTransport()).call<T>(cmd, args);
}

export const api = {
  setVisible: (visible: boolean) => call<void>('ui_set_visible', { visible }),
  detectClis: () => call<CliInfo[]>('detect_clis'),
  /** Every sign-in of every installed CLI, with quota windows. `refresh` re-probes sign-in state. */
  listAccounts: (refresh = false) => call<AccountInfo[]>('list_accounts', { refresh }),
  recentWorkspaces: () => call<RecentWorkspace[]>('recent_workspaces'),
  openWorkspace: (repoPath: string) => call<Workspace>('open_workspace', { repoPath }),
  updateSettings: (settings: Settings) => call<Settings>('update_settings', { settings }),
  /** thread: "lead" or "agent:<id>" */
  /** Save a pasted or dropped file in the repo for the agents to read; resolves with its path. */
  saveAttachment: async (file: File) => (await getTransport()).upload('save_attachment', file.name, await file.arrayBuffer()),
  sendChat: (thread: string, text: string) => call<void>('send_chat', { thread, text }),
  runPlan: (planId: string) => call<void>('run_plan', { planId }),
  cancelPlan: (planId: string) => call<void>('cancel_plan', { planId }),
  /** Close a request: its Lead stops, its unfinished plans are cancelled, it leaves the tabs. */
  closeThread: (thread: string) => call<void>('close_thread', { thread }),
  resolveApproval: (approvalId: string, decision: ApprovalDecision) =>
    call<void>('resolve_approval', { approvalId, decision }),
  /** Circuit breaker (Settings → Autonomy & Budgets): limits, live levels, and a fresh start. */
  breakerConfig: () => call<BreakerConfig>('breaker_config'),
  setBreakerConfig: (config: BreakerConfig) => call<BreakerConfig>('set_breaker_config', { config }),
  breakerStatus: () => call<BreakerStatus>('breaker_status'),
  resetBreaker: () => call<void>('reset_breaker'),
  interruptAgent: (agentId: string) => call<void>('interrupt_agent', { agentId }),
  stopAgent: (agentId: string) => call<void>('stop_agent', { agentId }),
  /** Live controls (agent panel). Each returns the agent's controls after the change. */
  agentControl: (agentId: string) => call<ControlState>('agent_control', { agentId }),
  setAgentBlockTools: (agentId: string, on: boolean) => call<ControlState>('set_agent_block_tools', { agentId, on }),
  stopAgentAfterStep: (agentId: string) => call<ControlState>('stop_agent_after_step', { agentId }),
  resumeAgent: (agentId: string) => call<ControlState>('resume_agent', { agentId }),
  steerAgent: (agentId: string, text: string) => call<ControlState>('steer_agent', { agentId, text }),
  hireAgent: (provider: Provider, model: string | null = null, title: string | null = null) => call<Agent>('hire_agent', { provider, model, title }),
  listModels: (provider: Provider) => call<string[]>('list_models', { provider }),
  // Add-agent wizard: the core judges hire manifests (import, AI draft) and starts the confirmed one.
  hireReview: (raw: string) => call<HireReview>('hire_review', { raw }),
  hireLaunchPreview: (provider: string, model: string | null, command: string | null) => call<LaunchPreview>('hire_launch_preview', { provider, model, command }),
  hireDraft: (brief: string) => call<HireReview>('hire_draft', { brief }),
  hireFromManifest: (manifest: HireManifest, confirmShell: boolean) => call<Agent>('hire_from_manifest', { manifest, confirmShell }),
  /** Applicants for the wizard to pick from (nothing is started). */
  hireApplicants: (n = 3) => call<Applicant[]>('hire_applicants', { n }),
  /** Pick who gets the job a new consultant's offer (in the office) is for. */
  pickApplicant: (offerId: string, index: number) => call<void>('pick_applicant', { offerId, index }),
  /** An agent as a `codly/agent@1` file: manifest, traits, career and memory. */
  exportAgent: (agentId: string) => call<string>('export_agent', { agentId }),
  /** Trust an agent up to a rank's perks (capped at its rank); 'intern' takes them back. */
  trustAgent: (agentId: string, rank: Rank) => call<Agent>('trust_agent', { agentId, rank }),
  /** What the office did since `since` (ms): the daily edition and the morning standup. */
  officeDigest: (since: number) => call<Digest>('office_digest', { since }),
  /** Workers let go in the open repo, who keep their career (the office's ghosts). */
  ghosts: () => call<Agent[]>('ghosts', {}),
  /** Bring a ghost back as a regular worker (same name, look, career and memory). */
  bringBack: (agentId: string) => call<Agent>('bring_back', { agentId }),
  /** Build/Buy: the budget (XP the agents earned here), catalog and furniture owned. */
  officeShop: () => call<Shop>('office_shop', {}),
  /** Buy a piece of furniture and put it down, or move one already bought (same uid). */
  officePlace: (piece: Placed) => call<Shop>('office_place', { piece }),
  /** Sell a piece of furniture back for the full price. */
  officeSell: (uid: string) => call<Shop>('office_sell', { uid }),
  /** The open repo's changes to the global team (members switched off, its own members). */
  setWorkspaceTeam: (team: WorkspaceTeam) => call<WorkspaceTeam>('set_workspace_team', { team }),
  /** A request's own mode, permissions and Lead model (null fields = the Settings defaults). */
  setRequestPrefs: (thread: string, prefs: RequestPrefs) => call<RequestPrefs>('set_request_prefs', { thread, prefs }),
  setAgentModel: (agentId: string, model: string | null) => call<Agent>('set_agent_model', { agentId, model }),
  setAgentLook: (agentId: string, lookSeed: string) => call<Agent>('set_agent_look', { agentId, lookSeed }),
  createTask: (title: string, goal: string) => call<Task>('create_task', { title, goal }),
  moveTask: (taskId: string, status: TaskStatus) => call<Task>('move_task', { taskId, status }),
  assignTask: (taskId: string, agentId: string | null) => call<Task>('assign_task', { taskId, agentId }),
  taskDiff: (taskId: string) => call<string>('task_diff', { taskId }),
  planDiff: (planId: string) => call<string>('plan_diff', { planId }),
  mergePlan: (planId: string) => call<string>('merge_plan', { planId }),
  keepPlan: (planId: string) => call<Plan>('keep_plan', { planId }),
  events: (agentId: string | null, before: number | null, limit = 500) =>
    call<EventRow[]>('get_events', { agentId, before, limit }),
  search: (query: string) => call<SearchResult>('search_code', { query }),
  memories: (query: string | null = null, scope: MemoryScope | null = null) =>
    call<Memory[]>('list_memories', { query, scope }),
  addMemory: (text: string, scope: MemoryScope, kind: MemoryKind, title: string | null = null, tags: string[] | null = null) =>
    call<Memory>('add_memory', { text, scope, kind, title, tags }),
  deleteMemory: (id: string) => call<void>('delete_memory', { id }),
  pinMemory: (id: string, pinned: boolean) => call<void>('pin_memory', { id, pinned }),
  triples: (entity: string | null = null) => call<Triple[]>('list_triples', { entity }),
  /** Literal, case-insensitive text search across the whiteboard, tasks and memories. */
  exactSearch: (query: string) => call<ExactHit[]>('exact_search', { query }),
  /** Agents and the messages between them (edges by act), from the chats and notes Codly recorded. */
  messageGraph: () => call<MessageGraph>('message_graph'),
  /** An agent's memory file (a starter template until first saved); the core confines it to its memory folder. */
  memoryFile: (agentId: string) => call<MemoryFile>('memory_file', { agentId }),
  saveMemoryFile: (agentId: string, text: string) => call<MemoryFile>('save_memory_file', { agentId, text }),
  /** Files of the open workspace (paths, line counts, languages) for the 3D codebase city. */
  repoMap: () => call<RepoFile[]>('repo_map'),
  /** Bytes of a recorded artifact (`Artifact.id`); its MIME type is `Artifact.mime`. */
  readArtifact: (artifactId: string) => call<ArrayBuffer>('read_artifact', { artifactId }),
  ptyOpen: (cwd: string | null) => call<string>('pty_open', { cwd }),
  ptyAttach: async (ptyId: string, onData: (bytes: Uint8Array) => void) =>
    (await getTransport()).attachPty(ptyId, onData),
  ptyWrite: (ptyId: string, data: string) => call<void>('pty_write', { ptyId, data }),
  ptyResize: (ptyId: string, cols: number, rows: number) => call<void>('pty_resize', { ptyId, cols, rows }),
  ptyClose: (ptyId: string) => call<void>('pty_close', { ptyId }),
  // Free Flow dictation. The key goes in once and never comes back: every reply is a status.
  dictationStatus: () => call<DictationStatus>('dictation_status'),
  dictationConfigure: (enabled: boolean | null, model: string | null) => call<DictationStatus>('dictation_configure', { enabled, model }),
  dictationSaveKey: (key: string) => call<DictationStatus>('dictation_save_key', { key }),
  dictationClearKey: () => call<DictationStatus>('dictation_clear_key'),
  /** One recording (base64 + MIME type) → its transcript. Sent to Groq by the core, only now. */
  dictationTranscribe: (audio: string, mime: string) => call<string>('dictation_transcribe', { audio, mime }),
  /** Read-only git state (status, branches, recent log) of an agent's worktree. */
  agentGit: (agentId: string) => call<AgentGit>('agent_git', { agentId }),
  /** Schedules, context rules and network mode, with the log of what triggers did. */
  triggers: () => call<TriggersConfig>('get_triggers'),
  /** Save them; the core keeps its own clocks and log, and returns what it stored. */
  setTriggers: (config: TriggersConfig) => call<TriggersConfig>('set_triggers', { config }),
  /** Skills the agent CLIs can load: project, user, plugins, Codex and Codly's own. */
  listSkills: (project: string | null) => call<Skill[]>('list_skills', { project }),
  /** The bundled catalog of public skills, each marked installed or not. */
  skillCatalog: () => call<CatalogSkill[]>('skill_catalog'),
  /** `git clone --depth 1` a catalog skill into ~/.claude/skills/<id>. */
  installSkill: (id: string) => call<Skill>('install_skill', { id }),
  uninstallSkill: (path: string, project: string | null) => call<void>('uninstall_skill', { path, project }),
  openSkillFolder: (path: string, project: string | null) => call<void>('open_skill_folder', { path, project }),
  // ── workspace IDE: every path is relative to the agent's worktree and confined to it by the core ──
  ideInfo: (agentId: string) => call<IdeInfo>('ide_info', { agentId }),
  ideList: (agentId: string, path: string) => call<IdeEntry[]>('ide_list', { agentId, path }),
  ideRead: (agentId: string, path: string) => call<IdeFile>('ide_read', { agentId, path }),
  ideWrite: (agentId: string, path: string, content: string) => call<void>('ide_write', { agentId, path, content }),
  ideCreate: (agentId: string, path: string) => call<void>('ide_create', { agentId, path }),
  ideSearch: (agentId: string, query: string) => call<IdeSearch>('ide_search', { agentId, query }),
  ideChanges: (agentId: string) => call<IdeChange[]>('ide_changes', { agentId }),
  ideDiff: (agentId: string, path: string) => call<string>('ide_diff', { agentId, path }),
  /** "Ask me" cards (codly_ask_user) of the open repo; long-waiting ones come back flagged. */
  listBlockers: () => call<Blocker[]>('list_blockers'),
  /** Respond & unblock: answer a card and resume the agent that waits for it. */
  answerBlocker: (blockerId: string, answer: string) => call<Blocker>('answer_blocker', { blockerId, answer }),
  dismissBlocker: (blockerId: string) => call<Blocker>('dismiss_blocker', { blockerId }),
  // Inbound work sources (webhooks, Slack). Secrets: only `inboundCreateWebhook` ever returns one.
  inboundState: () => call<InboundState>('inbound_state'),
  /** A new endpoint, switched off, with its secret (shown to the user once). */
  inboundCreateWebhook: (name: string) => call<NewWebhook>('inbound_create_webhook', { name }),
  inboundSetWebhook: (id: string, enabled: boolean) => call<InboundState>('inbound_set_webhook', { id, enabled }),
  inboundRevokeWebhook: (id: string) => call<InboundState>('inbound_revoke_webhook', { id }),
  /** Who answers inbound messages ("lead" or "agent:<id>") and the listener's port. */
  inboundSetSettings: (answerer: string, port: number) => call<InboundState>('inbound_set_settings', { answerer, port }),
  inboundSetSlack: (update: SlackUpdate) => call<InboundState>('inbound_set_slack', { update }),
  /** Board archiving thresholds (days); pass `policy` to change them. */
  boardPolicy: (policy: ArchivePolicy | null = null) => call<ArchivePolicy>('board_policy', { policy }),
  /** Archive what is due; reports what is flagged and which tasks are archived. */
  boardSweep: () => call<SweepReport>('board_sweep'),
  /** Archived tasks whose text matches `query` (all when empty). */
  boardArchive: (query: string | null = null) => call<ArchivedItem[]>('board_archive', { query }),
  /** Default MCP servers for new agents, with their switches and whether each key is stored. */
  listConnections: () => call<Connection[]>('list_connections'),
  /** Switch a server and/or store its key (Keychain; `value: null` removes it). Applies to the next spawn. */
  updateConnection: (id: string, enabled: boolean | null, secret: ConnectionSecret | null = null) =>
    call<Connection[]>('update_connection', { id, enabled, secret }),
  /** Onboarding: run the vendor's install line for a CLI in a visible terminal (attach with `ptyAttach`). The core picks the text from (provider, rung); nothing typed here is ever run. */
  installCli: (provider: Provider, rung: InstallRung) => call<InstallRun>('install_cli', { provider, rung }),
  /** OpenRouter key for OpenCode (opt-in). The key goes in once and never comes back: every reply is a status. */
  byokStatus: () => call<ByokStatus>('byok_status'),
  byokConfigure: (enabled: boolean) => call<ByokStatus>('byok_configure', { enabled }),
  byokSaveKey: (key: string) => call<ByokStatus>('byok_save_key', { key }),
  byokClear: () => call<ByokStatus>('byok_clear_key'),
  /** Native folder picker (Tauri dialog plugin); null if cancelled or in the browser. */
  pickFolder: async (): Promise<string | null> => {
    if (!isTauriApp()) return window.prompt('Repository path') || null;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const r = await open({ directory: true, multiple: false, title: 'Open a git repository' });
    return typeof r === 'string' ? r : null;
  },
};
