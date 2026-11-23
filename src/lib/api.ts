// Typed client for the Codly core. Two transports, one interface:
// - Tauri (the real app): commands + one ordered Channel of event batches.
// - Mock (plain browser, `pnpm dev`): a scripted demo session for UI work and tests.
//
// CONTRACT: command names and argument keys match src-tauri/src/commands.rs.
import type { AccountInfo } from './gen/AccountInfo';
import type { Agent } from './gen/Agent';
import type { Rank } from './gen/Rank';
import type { RepoEntry } from './gen/RepoEntry';
import type { Digest } from './gen/Digest';
import type { DictationStatus } from './gen/DictationStatus';
import type { AgentGit } from './gen/AgentGit';
import type { AgentCheckpoints } from './gen/AgentCheckpoints';
import type { CheckpointRestore } from './gen/CheckpointRestore';
import type { LastTurn } from './gen/LastTurn';
import type { TurnSource } from './gen/TurnSource';
import type { Blocker } from './gen/Blocker';
import type { ApprovalDecision } from './gen/ApprovalDecision';
import type { ApprovalRule } from './gen/ApprovalRule';
import type { ApprovalRuleKind } from './gen/ApprovalRuleKind';
import type { ApprovalRuleOffer } from './gen/ApprovalRuleOffer';
import type { BreakerConfig } from './gen/BreakerConfig';
import type { BreakerStatus } from './gen/BreakerStatus';
import type { Spent } from './gen/Spent';
import type { ArchivedItem } from './gen/ArchivedItem';
import type { ArchivePolicy } from './gen/ArchivePolicy';
import type { Connection } from './gen/Connection';
import type { ConnectionSecret } from './gen/ConnectionSecret';
import type { SweepReport } from './gen/SweepReport';
import type { SetupChoice } from './gen/SetupChoice';
import type { WorktreeSetup } from './gen/WorktreeSetup';
import type { AgentCheck } from './gen/AgentCheck';
import type { CheckResult } from './gen/CheckResult';
import type { ChecksChoice } from './gen/ChecksChoice';
import type { RequiredChecks } from './gen/RequiredChecks';
import type { TrustChoice } from './gen/TrustChoice';
import type { ResumeOffer } from './gen/ResumeOffer';
import type { WorkspaceTrust } from './gen/WorkspaceTrust';
import type { StorageCleanup } from './gen/StorageCleanup';
import type { StoragePolicy } from './gen/StoragePolicy';
import type { StorageReport } from './gen/StorageReport';
import type { StartProblem } from './gen/StartProblem';
import type { RepoSync } from './gen/RepoSync';
import type { MergeState } from './gen/MergeState';
import type { MergeWarning } from './gen/MergeWarning';
import type { Intake } from './gen/Intake';
import type { IntakeKind } from './gen/IntakeKind';
import type { PrConfirm } from './gen/PrConfirm';
import type { PrDraft } from './gen/PrDraft';
import type { PrPushed } from './gen/PrPushed';
import type { PrSource } from './gen/PrSource';
import type { PullRequest } from './gen/PullRequest';
import type { Preview } from './gen/Preview';
import type { PreviewAction } from './gen/PreviewAction';
import type { PreviewSeen } from './gen/PreviewSeen';
import type { PreviewSource } from './gen/PreviewSource';
import type { ByokSource } from './gen/ByokSource';
import type { ByokStatus } from './gen/ByokStatus';
import type { Vendor } from './gen/Vendor';
import type { CliInfo } from './gen/CliInfo';
import type { ClosedRequest } from './gen/ClosedRequest';
import type { ClosedTranscript } from './gen/ClosedTranscript';
import type { InstallRun } from './gen/InstallRun';
import type { InstallRung } from './gen/InstallRung';
import type { ContextUsage } from './gen/ContextUsage';
import type { ControlState } from './gen/ControlState';
import type { PermissionMode } from './gen/PermissionMode';
import type { ModelCatalog } from './gen/ModelCatalog';
import type { Takeover } from './gen/Takeover';
import type { TakeoverState } from './gen/TakeoverState';
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
import type { Wallet } from './gen/Wallet';
import type { Lot } from './gen/Lot';
import type { LotSaved } from './gen/LotSaved';
import type { LotSlot } from './gen/LotSlot';
import type { HeldCheck } from './gen/HeldCheck';
import type { HireReview } from './gen/HireReview';
import type { LaunchPreview } from './gen/LaunchPreview';
import type { Memory } from './gen/Memory';
import type { ModelOffer } from './gen/ModelOffer';
import type { MemoryKind } from './gen/MemoryKind';
import type { MemoryScope } from './gen/MemoryScope';
import type { NewWebhook } from './gen/NewWebhook';
import type { Plan } from './gen/Plan';
import type { Provider } from './gen/Provider';
import type { RecentWorkspace } from './gen/RecentWorkspace';
import type { Office } from './gen/Office';
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

export type UiBatch = { events: EventRow[]; resync?: boolean };

/** What to attach to a bug report (src-tauri/src/diagnostics.rs). */
export interface Diagnostics {
  version: string;
  /** The log file, to show in Finder; null when the app could not open one. */
  log_file: string | null;
  /** Plain text: version, system, agent CLIs and the last log lines. Tokens, keys, emails and paths under the home folder are taken out; repository and agent names can remain. */
  report: string;
}

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

/** `codly serve`: the same commands over HTTP, the bus and terminals as server-sent events. */
class WebTransport implements Transport {
  private events: EventSource | null = null;
  subscribe(onBatch: (b: UiBatch) => void): Promise<Snapshot> {
    this.events?.close();
    const es = (this.events = new EventSource('/api/events'));
    const data = (e: Event) => JSON.parse((e as MessageEvent<string>).data);
    return new Promise<Snapshot>((resolve, reject) => {
      let open = false;
      es.addEventListener('batch', (e) => onBatch(data(e)));
      es.addEventListener('snapshot', (e) => {
        open = true;
        resolve(data(e));
      });
      es.addEventListener('failed', (e) => {
        es.close();
        reject(new Error(data(e)));
      });
      es.onerror = () => {
        es.close();
        // Codly stopped or restarted: take a fresh snapshot (the store retries via resync).
        if (open) setTimeout(() => onBatch({ events: [], resync: true }), 1000);
        else reject(new Error('Can’t reach Codly. Is `codly serve` still running?'));
      };
    });
  }
  call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    return post<T>(cmd, JSON.stringify(args ?? {}), { 'content-type': 'application/json' });
  }
  upload(cmd: string, name: string, bytes: ArrayBuffer): Promise<string> {
    return post<string>(cmd, bytes, { 'content-type': 'application/octet-stream', 'x-name': encodeURIComponent(name) });
  }
  attachPty(ptyId: string, onData: (bytes: Uint8Array) => void): Promise<void> {
    // ponytail: one connection per terminal; browsers allow 6 per host over HTTP/1.1.
    const es = new EventSource(`/api/pty/${encodeURIComponent(ptyId)}`);
    es.addEventListener('data', (e) => onData(Uint8Array.from(atob((e as MessageEvent<string>).data), (c) => c.charCodeAt(0))));
    return new Promise((resolve, reject) => {
      es.onopen = () => resolve();
      es.onerror = () => {
        es.close(); // the terminal closed, or Codly went away
        reject(new Error('no such terminal'));
      };
    });
  }
}

/** A command's value, or what it rejected with (as Tauri's `invoke` does). */
async function post<T>(cmd: string, body: BodyInit, headers: Record<string, string>): Promise<T> {
  const r = await fetch(`/api/cmd/${cmd}`, { method: 'POST', body, headers });
  const type = r.headers.get('content-type') ?? '';
  const value = type.startsWith('application/json') ? await r.json() : type.startsWith('application/octet-stream') ? await r.arrayBuffer() : await r.text();
  if (!r.ok) throw value;
  return value as T;
}

export const isTauriApp = (): boolean =>
  typeof window !== 'undefined' && !!(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;

/** The UI served by `codly serve` to a browser (index.html sets the flag). */
export const isWebApp = (): boolean => typeof window !== 'undefined' && (window as unknown as { __CODLY_WEB__?: boolean }).__CODLY_WEB__ === true;

/** A real Codly behind the UI: the app's window, or a browser on `codly serve`. */
export const hasBackend = (): boolean => isTauriApp() || isWebApp();

/** The public web demo (`pnpm build:site` → /demo/): the scripted fake repo, opened at once. */
export const isSiteDemo = (): boolean => import.meta.env.VITE_SITE_DEMO === '1' && !isTauriApp();

// The promise, not the instance: two callers during the mock's import must share one transport
// (or the store listens to one scripted office while commands go to another).
let transport: Promise<Transport> | null = null;

export function getTransport(): Promise<Transport> {
  return (transport ??= (async () => {
    if (isTauriApp()) return new TauriTransport();
    if (isWebApp()) return new WebTransport();
    return new (await import('./mock')).MockTransport();
  })());
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return (await getTransport()).call<T>(cmd, args);
}

export const api = {
  /** "Try again" on the start-up error screen: start the core again if its start failed. Subscribe again after it. */
  retryCore: () => call<void>('retry_core'),
  /** The start-up error screen: what to say and offer when the start failed because the database cannot be used. null = another reason. */
  startProblem: () => call<StartProblem | null>('start_problem'),
  /** Start from `backup` (the path of one `startProblem` offered) or with no data (null). The app moves the database aside, never deletes it, and starts the core again. Subscribe again after it. */
  recoverDatabase: (backup: string | null) => call<void>('recover_database', { backup }),
  setVisible: (visible: boolean) => call<void>('ui_set_visible', { visible }),
  detectClis: () => call<CliInfo[]>('detect_clis'),
  /** Every sign-in of every installed CLI, with quota windows. `refresh` re-probes sign-in state. */
  listAccounts: (refresh = false) => call<AccountInfo[]>('list_accounts', { refresh }),
  recentWorkspaces: () => call<RecentWorkspace[]>('recent_workspaces'),
  /** The git repositories under the home folder, newest first: the browser's "Open repository" picker. */
  listRepos: () => call<RepoEntry[]>('list_repos'),
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
  /** The requests you closed in the open repo, newest first. */
  closedRequests: () => call<ClosedRequest[]>('closed_requests'),
  /** One closed request's messages, to read again (`thread` from `closedRequests`). */
  closedRequest: (thread: string) => call<ClosedTranscript>('closed_request', { thread }),
  resolveApproval: (approvalId: string, decision: ApprovalDecision) =>
    call<void>('resolve_approval', { approvalId, decision }),
  /** "Always allow in this repo": what a waiting request can be saved as (nothing for what Codly always asks about). */
  approvalRuleOffer: (approvalId: string) => call<ApprovalRuleOffer>('approval_rule_offer', { approvalId }),
  /** Save the rule and answer the waiting requests it covers. The rule's text comes from the request. */
  addApprovalRule: (approvalId: string, kind: ApprovalRuleKind) => call<ApprovalRule>('add_approval_rule', { approvalId, kind }),
  /** The open repo's saved rules (Settings → Approvals). */
  approvalRules: () => call<ApprovalRule[]>('approval_rules'),
  removeApprovalRule: (ruleId: string) => call<void>('remove_approval_rule', { ruleId }),
  /** Circuit breaker (Settings → Autonomy & Budgets): limits, live levels, and a fresh start. */
  breakerConfig: () => call<BreakerConfig>('breaker_config'),
  setBreakerConfig: (config: BreakerConfig) => call<BreakerConfig>('set_breaker_config', { config }),
  breakerStatus: () => call<BreakerStatus>('breaker_status'),
  resetBreaker: () => call<void>('reset_breaker'),
  /** What a plan's tasks used so far: work tokens and the cost the CLIs report, over every worker and reviewer run. */
  planSpent: (planId: string) => call<Spent>('plan_spent', { planId }),
  interruptAgent: (agentId: string) => call<void>('interrupt_agent', { agentId }),
  stopAgent: (agentId: string) => call<void>('stop_agent', { agentId }),
  /** Live controls (agent panel). Each returns the agent's controls after the change. */
  setAgentBlockTools: (agentId: string, on: boolean) => call<ControlState>('set_agent_block_tools', { agentId, on }),
  stopAgentAfterStep: (agentId: string) => call<ControlState>('stop_agent_after_step', { agentId }),
  resumeAgent: (agentId: string) => call<ControlState>('resume_agent', { agentId }),
  steerAgent: (agentId: string, text: string) => call<ControlState>('steer_agent', { agentId, text }),
  setAgentPermissionMode: (agentId: string, mode: PermissionMode) => call<Agent>('set_agent_permission_mode', { agentId, mode }),
  /** Ask a running Claude or Codex agent to compact its conversation now. Resolves when the CLI took the request. */
  compactAgent: (agentId: string) => call<void>('compact_agent', { agentId }),
  agentContextUsage: (agentId: string) => call<ContextUsage>('agent_context_usage', { agentId }),
  /** Try again to connect one MCP server of a live Claude agent. Rejects with Claude Code's own reason when it still does not connect. */
  reconnectAgentMcp: (agentId: string, server: string) => call<void>('reconnect_agent_mcp', { agentId, server }),
  /** Turn one MCP server of a live Claude agent off or on, for this run of the agent. Its next `mcp_server` event says where it stands. */
  setAgentMcpEnabled: (agentId: string, server: string, enabled: boolean) => call<void>('set_agent_mcp_enabled', { agentId, server, enabled }),
  /** Stop a command a live Claude agent left running in the background. Its next `background_tasks` event says that it was stopped. */
  stopAgentTask: (agentId: string, taskId: string) => call<void>('stop_agent_task', { agentId, taskId }),
  /** "Open in my terminal" (agent sheet). Is the agent in your terminal, and if not, may it go there now? */
  agentTakeover: (agentId: string) => call<TakeoverState>('agent_takeover', { agentId }),
  /** Every agent that is in your terminal: what the window marks them with. */
  listTakeovers: () => call<Takeover[]>('list_takeovers'),
  /** Codly stops its own process for the agent's session and starts none until `giveBackAgent`. */
  takeOverAgent: (agentId: string) => call<TakeoverState>('take_over_agent', { agentId }),
  /** A terminal (attach with `ptyAttach`) in the worktree of an agent that is in your terminal, with the CLI's resume command typed, not run. */
  openTakeoverTerminal: (agentId: string) => call<string>('open_takeover_terminal', { agentId }),
  /** Closes that terminal; the agent's next message or task resumes the same session in Codly. */
  giveBackAgent: (agentId: string) => call<TakeoverState>('give_back_agent', { agentId }),
  /** A CLI's models with the thinking efforts each one takes, and where the list comes from. */
  modelCatalog: (provider: Provider) => call<ModelCatalog>('model_catalog', { provider }),
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
  /** The open repo's office as a `codly/office@1` file (the lot and the crew; no code, paths or prompts). */
  exportOffice: () => call<string>('export_office'),
  /** Set how much you trust an agent (`TRUST` in util/career), whatever its rank; 'intern' takes it all back. */
  trustAgent: (agentId: string, rank: Rank) => call<Agent>('trust_agent', { agentId, rank }),
  /** The office sends an agent home to rest until `until` (ms), or calls it back (null). The
   *  Lead gives it no new work meanwhile; its conversation is untouched. */
  setAgentHome: (agentId: string, until: number | null) => call<Agent>('set_agent_home', { agentId, until }),
  /** The office reports a shift worked and rested from, or a sickness (its stamina). */
  noteRest: (agentId: string, sick: boolean) => call<Agent>('note_rest', { agentId, sick }),
  noteHangout: (a: string, b: string) => call<void>('note_hangout', { a, b }),
  /** The office reports an agent tired (energy under 30%) or rested again: the Lead and the
   *  scheduler prefer the rested (a hint, never a refusal). */
  setAgentTired: (agentId: string, tired: boolean) => call<Agent>('set_agent_tired', { agentId, tired }),
  /** The model of its CLI with the biggest context window, when bigger than its own (offered
   *  to a new Legend; null = nothing bigger). */
  biggerModel: (agentId: string) => call<ModelOffer | null>('bigger_model', { agentId }),
  /** What the office did since `since` (ms): the daily edition and the morning standup. */
  officeDigest: (since: number) => call<Digest>('office_digest', { since }),
  /** Workers let go in the open repo, who keep their career (the office's ghosts). */
  ghosts: () => call<Agent[]>('ghosts', {}),
  /** Bring a ghost back as a regular worker (same name, look, career and memory). */
  bringBack: (agentId: string) => call<Agent>('bring_back', { agentId }),
  /** Let go an agent you hired: it becomes a ghost you can bring back. Refused mid-task, for the Lead and team members. */
  letGo: (agentId: string) => call<Agent>('let_go', { agentId }),
  /** Your other offices (repos opened before, the open one aside), with their crews and tasks in flight. */
  offices: () => call<Office[]>('offices', {}),
  /** Transfer an agent to another of your offices: its career, traits, stamina, look and memory
   *  go with it; trust, friends, mentor and its session stay. Refused mid-task and for the Lead. */
  transferAgent: (agentId: string, toWorkspaceId: string) => call<Agent>('transfer_agent', { agentId, toWorkspaceId }),
  // Build/Buy and Build mode name their office (`ws`): a save can land after you switched to
  // another one, which must not get this one's walls and furniture.
  /** Build/Buy: the wallet (coins, across every repo) and the furniture in office `ws`. */
  officeShop: (ws: string) => call<Shop>('office_shop', { workspaceId: ws }),
  /** Buy a piece of furniture and put it down, or move one already bought (same uid). */
  officePlace: (ws: string, piece: Placed) => call<Shop>('office_place', { workspaceId: ws, piece }),
  /** Sell a piece of furniture back for the full price. */
  officeSell: (ws: string, uid: string) => call<Shop>('office_sell', { workspaceId: ws, uid }),
  /** The player's coins (🪙): every repo's XP earned, plus grants, less what is owned. */
  wallet: () => call<Wallet>('wallet', {}),
  /** Developer mode only: give coins (flags the wallet as cheated, for good). */
  devGrant: (amount: number) => call<Wallet>('dev_grant', { amount }),
  /** Developer mode only: give an agent XP; its rank follows (flags the wallet too). */
  devGiveXp: (agentId: string, xp: number) => call<Agent>('dev_give_xp', { agentId, xp }),
  /** Developer mode only: this repo's lot back to a gift Garage (what stood there sells back). */
  devResetOffice: () => call<Shop>('dev_reset_office', {}),
  /** Build mode: office `ws`'s lot (walls, floors, furniture), made on the first read. */
  officeLot: (ws: string) => call<Lot>('office_lot', { workspaceId: ws }),
  /** Build mode: save the lot, checked and priced by the core (what changed is paid for or sold
   *  back; refused if the wallet can't pay): the lot as stored, the wallet, and the cost. */
  officeSaveLot: (ws: string, lot: Lot) => call<LotSaved>('office_save_lot', { workspaceId: ws, lot }),
  /** The neighbourhood: every plot, yours and for sale, with the repo on each. */
  lots: () => call<LotSlot[]>('lots', {}),
  /** A plot's lot, for drawing it from outside (a bare plot if it isn't yours). */
  lotOf: (slotId: string) => call<Lot>('lot_of', { slotId }),
  /** Buy a plot, bare (null) or with a ready-made design on it, at `price.ts::readyMadePrice`;
   *  or put a design on a bare plot of yours. */
  buyLot: (slotId: string, template: string | null) => call<Wallet>('buy_lot', { slotId, template }),
  /** Sell a plot with no office on it: 80% of the land, the contents at their sell-back rates. */
  sellLot: (slotId: string) => call<Wallet>('sell_lot', { slotId }),
  /** Move a repo's office to another plot of yours with none on it: the furniture comes along
   *  (what doesn't fit goes to storage), the building stays. Returns the neighbourhood. */
  moveOffice: (workspaceId: string, slotId: string) => call<LotSlot[]>('move_office', { workspaceId, slotId }),
  /** Close a repo's office: it leaves Codly and its plot is freed. Nothing is deleted; opening the repo again brings it back. */
  closeOffice: (workspaceId: string) => call<LotSlot[]>('close_office', { workspaceId }),
  /** The open repo's changes to the global team (members switched off, its own members). */
  setWorkspaceTeam: (team: WorkspaceTeam) => call<WorkspaceTeam>('set_workspace_team', { team }),
  /** A request's own mode, permissions and Lead model (null fields = the Settings defaults). */
  setRequestPrefs: (thread: string, prefs: RequestPrefs) => call<RequestPrefs>('set_request_prefs', { thread, prefs }),
  setAgentModel: (agentId: string, model: string | null) => call<Agent>('set_agent_model', { agentId, model }),
  setAgentLook: (agentId: string, lookSeed: string) => call<Agent>('set_agent_look', { agentId, lookSeed }),
  createTask: (title: string, goal: string) => call<Task>('create_task', { title, goal }),
  moveTask: (taskId: string, status: TaskStatus) => call<Task>('move_task', { taskId, status }),
  /** Cancel a task together with every task that waits for it (they could never start). Returns them, the task first. */
  cancelTaskTree: (taskId: string) => call<Task[]>('cancel_task_tree', { taskId }),
  /** Keep what is merged: a running plan finishes with its merged tasks; its other tasks are cancelled. */
  finishPlan: (planId: string) => call<Plan>('finish_plan', { planId }),
  /** Run a blocked or failed task's checks again, with no worker turn. `allow` names the risky commands you OK'd.
   *  Returns the commands that still need your OK (then nothing ran); none = the checks are running, or the task
   *  is Ready again for a team member that is on other work in the same worktree. */
  recheckTask: (taskId: string, allow: string[]) => call<HeldCheck[]>('recheck_task', { taskId, allow }),
  /** Merge anyway: a task that its checks stopped goes on without them. Your choice stays in its record. */
  acceptTask: (taskId: string) => call<Task>('accept_task', { taskId }),
  assignTask: (taskId: string, agentId: string | null) => call<Task>('assign_task', { taskId, agentId }),
  taskDiff: (taskId: string) => call<string>('task_diff', { taskId }),
  planDiff: (planId: string) => call<string>('plan_diff', { planId }),
  /** Send back: the review comments on a task's diff, as one message, go to the agent that did the work.
   *  Refused for a done task (its work is merged: the changes are a new request). */
  sendBackTask: (taskId: string, comments: string) => call<Task>('send_back_task', { taskId, comments }),
  /** Merge a plan into your checked-out branch. `into`: the branch you confirmed after `mergePlanWarning`.
   *  When your branch has commits the plan was never checked with, the plan's checks run on the merge
   *  result first and a failure stops the merge; `unchecked`: you merge without them. */
  mergePlan: (planId: string, into: string | null = null, unchecked = false) => call<string>('merge_plan', { planId, into, unchecked }),
  /** What stands between a plan's branch (or a solo agent's) and your branch: the files that conflict,
   *  and the plan's checks that failed on the merge result. Reads only. */
  mergeState: (source: PrSource) => call<MergeState>('merge_state', { source }),
  /** Have an agent resolve the conflict with your branch, in Codly's own worktree (your checkout is not touched).
   *  `into`: the branch you confirmed after `mergePlanWarning`, for a plan that did not start from it. */
  resolveMergeConflict: (source: PrSource, into: string | null = null) => call<MergeState>('resolve_merge_conflict', { source, into }),
  /** Stop a solo agent's conflict resolution: the merge is taken out of its worktree, its branch is as before. */
  stopResolvingMerge: (agentId: string) => call<MergeState>('stop_resolving_merge', { agentId }),
  /** What to confirm before merging a plan: your checkout is not on the branch it started from (null: nothing). */
  mergePlanWarning: (planId: string) => call<MergeWarning | null>('merge_plan_warning', { planId }),
  /** Undo the last merge Codly made into your branch (`sha` from `repoSync().last_merge`). */
  undoMerge: (sha: string) => call<RepoSync>('undo_merge', { sha }),
  /** Merge a solo agent's branch into your checked-out branch. Returns the new HEAD sha. */
  mergeAgent: (agentId: string) => call<string>('merge_agent', { agentId }),
  /** Your checkout against its remote: ahead/behind and uncommitted tracked files. */
  repoSync: () => call<RepoSync>('repo_sync'),
  /** Commit your changes to tracked files (your hooks run). Returns the new sha. */
  commitCheckout: (message: string) => call<string>('commit_checkout', { message }),
  /** Push your checked-out branch to its remote. */
  pushCheckout: () => call<RepoSync>('push_checkout'),
  /** The pull request Codly opened for a plan or a solo agent, as last read (null: none). No network. */
  pullRequest: (source: PrSource) => call<PullRequest | null>('pull_request', { source }),
  /** What Create pull request would send: remote, branch, base, a drafted title and description. Sends nothing. */
  draftPullRequest: (source: PrSource) => call<PrDraft>('draft_pull_request', { source }),
  /** Push the Codly branch you confirmed and open a pull request with your own `gh`. */
  createPullRequest: (source: PrSource, confirm: PrConfirm, title: string, body: string) =>
    call<PullRequest>('create_pull_request', { source, confirm, title, body }),
  /** Push the Codly branch you confirmed, without a pull request. */
  pushPrBranch: (source: PrSource, confirm: PrConfirm) => call<PrPushed>('push_pr_branch', { source, confirm }),
  /** Read the pull request's checks again: one `gh pr checks` call. */
  refreshPullRequest: (source: PrSource) => call<PullRequest>('refresh_pull_request', { source }),
  /** "From issue…" / "From pull request…": read it with your own `gh` and start work on it in the request `thread`. Writes nothing to GitHub. */
  startFromGithub: (thread: string, kind: IntakeKind, reference: string, note: string) => call<Intake>('start_from_github', { thread, kind, reference, note }),
  /** The preview of a plan's or an agent's worktree: the repo's command and the run there, with its output after `seen`. `action` starts your command or stops it first. */
  preview: (source: PreviewSource, action: PreviewAction | null = null, seen: PreviewSeen | null = null) =>
    call<Preview>('preview', { source, action, seen }),
  keepPlan: (planId: string) => call<Plan>('keep_plan', { planId }),
  events: (agentId: string | null, before: number | null, limit = 500) =>
    call<EventRow[]>('get_events', { agentId, before, limit }),
  /** One page of a conversation's history before event `before`, newest first: the events of the
   *  agent it belongs to (`agentId`), its messages, and its tasks' approvals and results. */
  threadEvents: (thread: string, agentId: string | null, before: number, limit: number) =>
    call<EventRow[]>('get_events', { agentId, thread, before, limit }),
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
  /** An agent's checkpoints: the files of its worktree before each of its turns, newest first. */
  agentCheckpoints: (agentId: string) => call<AgentCheckpoints>('agent_checkpoints', { agentId }),
  /** Put the files of the agent's worktree back to a checkpoint, or with `path` that one file
   *  alone (one of the checkpoint's `files`). Files only; it can be undone. */
  restoreCheckpoint: (agentId: string, checkpoint: number, path: string | null = null) =>
    call<CheckpointRestore>('restore_checkpoint', { agentId, checkpoint, path }),
  /** What the last turn of an agent, or the last turn in a task's worktree, changed in the files.
   *  Null: Codly kept no turn there. */
  lastTurnDiff: (source: TurnSource) => call<LastTurn | null>('last_turn_diff', { source }),
  /** Schedules and network mode, with the log of what triggers did. */
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
  /** One endpoint's two switches: on or off, and whether a plan made from its messages may run without your Run plan click. */
  inboundSetWebhook: (id: string, enabled: boolean, autoRun: boolean) => call<InboundState>('inbound_set_webhook', { id, enabled, autoRun }),
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
  /** What a new worktree of the open repo gets; pass `set` to turn its setup script on (for the text you read) or off. */
  worktreeSetup: (set: SetupChoice | null = null) => call<WorktreeSetup>('worktree_setup', { set }),
  /** The open repo's required checks (`.codly/checks`); pass `set` to turn them on (for the text you read) or off. */
  requiredChecks: (set: ChecksChoice | null = null) => call<RequiredChecks>('required_checks', { set }),
  /** Run the required checks in the worktree of an agent you chat with; each command's result. */
  checkAgent: (agentId: string) => call<CheckResult[]>('check_agent', { agentId }),
  /** What "Check the work" has for an agent now: whether its checks run, and the last result while its files are still the checked ones. */
  agentCheckState: (agentId: string) => call<AgentCheck>('agent_check_state', { agentId }),
  /** What the open repo makes the agent CLIs load by themselves, and your choice; pass `set` to trust the files you read or to open the repo without them. */
  workspaceTrust: (set: TrustChoice | null = null) => call<WorkspaceTrust>('workspace_trust', { set }),
  /** The work that was under way when Codly last closed and waits for your Resume (null: nothing waits); pass `go` to let it go on. */
  resumeWork: (go = false) => call<ResumeOffer | null>('resume_work', { go }),
  /** Default MCP servers for new agents, with their switches and whether each key is stored. */
  listConnections: () => call<Connection[]>('list_connections'),
  /** Switch a server and/or store its key (Keychain; `value: null` removes it). Applies to the next spawn. */
  updateConnection: (id: string, enabled: boolean | null, secret: ConnectionSecret | null = null) =>
    call<Connection[]>('update_connection', { id, enabled, secret }),
  /** What Codly keeps on disk and in each repository: worktrees, `codly/…` branches, run logs. Measures every worktree, so it can take a while. */
  storageReport: () => call<StorageReport>('storage_report'),
  /** How long run logs are kept and whether Codly cleans up after finished plans; pass `policy` to change them. */
  storagePolicy: (policy: StoragePolicy | null = null) => call<StoragePolicy>('storage_policy', { policy }),
  /** Remove what has nothing to lose, now: merged, clean worktrees and merged branches of finished work, old run logs. */
  storageClean: () => call<StorageCleanup>('storage_clean'),
  /** Delete one of Codly's worktrees (`head`: the commit the list showed). A dirty or unmerged one needs `force` and keeps its branch unless `deleteBranch`. */
  storageDeleteWorktree: (officeId: string, path: string, head: string | null, force: boolean, deleteBranch: boolean) =>
    call<null>('storage_delete_worktree', { officeId, path, head, force, deleteBranch }),
  /** Delete a `codly/…` branch that no worktree has checked out. Commits that no other branch has need `force`. */
  storageDeleteBranch: (officeId: string, branch: string, head: string, force: boolean) =>
    call<null>('storage_delete_branch', { officeId, branch, head, force }),
  /** Remove one backup of Codly's data that the storage report listed (`path`): a copy, or a folder of older copies that was moved aside. The database is never removed. */
  storageDeleteBackup: (path: string) => call<null>('storage_delete_backup', { path }),
  /** Onboarding: run the vendor's install or update line for a CLI in a visible terminal (attach with `ptyAttach`). The core picks the text from (provider, rung); nothing typed here is ever run. */
  installCli: (provider: Provider, rung: InstallRung) => call<InstallRun>('install_cli', { provider, rung }),
  /** Your own API credentials per vendor (opt-in): Anthropic → Claude, OpenAI → Codex, OpenRouter → OpenCode.
   *  A key goes in once and never comes back: every reply is a status. */
  byokStatus: () => call<ByokStatus>('byok_status'),
  byokConfigure: (vendor: Vendor, source: ByokSource) => call<ByokStatus>('byok_configure', { vendor, source }),
  byokSaveKey: (vendor: Vendor, key: string) => call<ByokStatus>('byok_save_key', { vendor, key }),
  byokClear: (vendor: Vendor) => call<ByokStatus>('byok_clear_key', { vendor }),
  /** The report for a bug report. It answers when the core did not start too; it probes the CLIs, so it takes a few seconds. */
  diagnostics: () => call<Diagnostics>('diagnostics'),
  /** Native folder picker (Tauri dialog plugin); in the browser a path prompt (repositories open through
   *  the repository picker instead, `openWorkspace`). Null if cancelled. */
  pickFolder: async (): Promise<string | null> => {
    if (!isTauriApp()) return window.prompt('Folder path') || null;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const r = await open({ directory: true, multiple: false, title: 'Open a git repository' });
    return typeof r === 'string' ? r : null;
  },
};
