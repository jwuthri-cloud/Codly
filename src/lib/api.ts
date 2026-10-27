// Typed client for the Codly core. Two transports, one interface:
// - Tauri (the real app): commands + one ordered Channel of event batches.
// - Mock (plain browser, `pnpm dev`): a scripted demo session for UI work and tests.
//
// CONTRACT: command names and argument keys match src-tauri/src/commands.rs.
import type { AccountInfo } from './gen/AccountInfo';
import type { Agent } from './gen/Agent';
import type { ApprovalDecision } from './gen/ApprovalDecision';
import type { CliInfo } from './gen/CliInfo';
import type { EventRow } from './gen/EventRow';
import type { Memory } from './gen/Memory';
import type { MemoryKind } from './gen/MemoryKind';
import type { MemoryScope } from './gen/MemoryScope';
import type { Plan } from './gen/Plan';
import type { Provider } from './gen/Provider';
import type { RecentWorkspace } from './gen/RecentWorkspace';
import type { RepoFile } from './gen/RepoFile';
import type { SearchResult } from './gen/SearchResult';
import type { Settings } from './gen/Settings';
import type { Snapshot } from './gen/Snapshot';
import type { Task } from './gen/Task';
import type { TaskStatus } from './gen/TaskStatus';
import type { Triple } from './gen/Triple';
import type { Workspace } from './gen/Workspace';
import type { WorkspaceTeam } from './gen/WorkspaceTeam';

export type UiBatch = { events: EventRow[] };

export interface Transport {
  /** Install the batch sink first, then resolve with a snapshot. Drop events with id <= snapshot.as_of (id 0 = ephemeral, always apply). */
  subscribe(onBatch: (b: UiBatch) => void): Promise<Snapshot>;
  call<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
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
  sendChat: (thread: string, text: string) => call<void>('send_chat', { thread, text }),
  runPlan: (planId: string) => call<void>('run_plan', { planId }),
  cancelPlan: (planId: string) => call<void>('cancel_plan', { planId }),
  /** Close a request: its Lead stops, its unfinished plans are cancelled, it leaves the tabs. */
  closeThread: (thread: string) => call<void>('close_thread', { thread }),
  resolveApproval: (approvalId: string, decision: ApprovalDecision) =>
    call<void>('resolve_approval', { approvalId, decision }),
  interruptAgent: (agentId: string) => call<void>('interrupt_agent', { agentId }),
  stopAgent: (agentId: string) => call<void>('stop_agent', { agentId }),
  hireAgent: (provider: Provider, model: string | null = null) => call<Agent>('hire_agent', { provider, model }),
  listModels: (provider: Provider) => call<string[]>('list_models', { provider }),
  /** The open repo's changes to the global team (members switched off, its own members). */
  setWorkspaceTeam: (team: WorkspaceTeam) => call<WorkspaceTeam>('set_workspace_team', { team }),
  setAgentModel: (agentId: string, model: string | null) => call<Agent>('set_agent_model', { agentId, model }),
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
  addMemory: (text: string, scope: MemoryScope, kind: MemoryKind) => call<Memory>('add_memory', { text, scope, kind }),
  deleteMemory: (id: string) => call<void>('delete_memory', { id }),
  pinMemory: (id: string, pinned: boolean) => call<void>('pin_memory', { id, pinned }),
  triples: (entity: string | null = null) => call<Triple[]>('list_triples', { entity }),
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
  /** Native folder picker (Tauri dialog plugin); null if cancelled or in the browser. */
  pickFolder: async (): Promise<string | null> => {
    if (!isTauriApp()) return window.prompt('Repository path') || null;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const r = await open({ directory: true, multiple: false, title: 'Open a git repository' });
    return typeof r === 'string' ? r : null;
  },
};
