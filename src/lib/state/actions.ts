// User intents → core commands. Every action reports failure as a toast that says how to get
// out, and applies command results the core does not echo as events (workspace, settings).
import { SvelteSet } from 'svelte/reactivity';
import { api, isTauriApp } from '../api';
import type { Agent } from '../gen/Agent';
import type { AccountConfig } from '../gen/AccountConfig';
import type { ApprovalChoice } from '../gen/ApprovalChoice';
import type { MemoryKind } from '../gen/MemoryKind';
import type { MemoryScope } from '../gen/MemoryScope';
import type { Provider } from '../gen/Provider';
import type { Settings } from '../gen/Settings';
import type { TaskStatus } from '../gen/TaskStatus';
import type { WorkspaceTeam } from '../gen/WorkspaceTeam';
import type { RequestPrefs } from '../gen/RequestPrefs';
import { PROVIDER_LABEL, TASK_STATUS_LABEL } from '../util/labels';
import { shortSha } from '../util/format';
import { accountId, applyConfigs, withProvider } from '../util/accounts';
import { withModel } from '../util/team';
import { accounts } from './accounts.svelte';
import { diffstats } from './diffstats.svelte';
import { requests } from './requests.svelte';
import { store } from './store.svelte';
import { toast, toasts } from './toasts.svelte';
import { ui, wallNow } from './ui.svelte';

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

async function attempt<T>(what: string, fn: () => Promise<T>, hint?: string): Promise<T | undefined> {
  try {
    return await fn();
  } catch (e) {
    toast({ kind: 'error', title: `Couldn’t ${what}`, body: hint ? `${msg(e)} — ${hint}` : msg(e) });
    return undefined;
  }
}

export async function openWorkspace(path?: string) {
  // A browser page cannot show a folder dialog: its picker lists the repositories and calls back here.
  if (!path && !isTauriApp()) {
    ui.repoPickerOpen = true;
    return;
  }
  const repo = path ?? (await api.pickFolder());
  if (!repo) return;
  const ws = await attempt('open the repository', () => api.openWorkspace(repo), 'Pick the folder that contains .git.');
  if (!ws) return;
  store.setWorkspace(ws);
  diffstats.reset();
  // A workspace switch is a reconnect: take a fresh snapshot for the new repo.
  await store.resync();
  if (!store.workspace) store.setWorkspace(ws);
  const [memories, triples] = await Promise.all([api.memories().catch(() => null), api.triples().catch(() => null)]);
  if (memories) store.setMemories(memories);
  if (triples) store.setTriples(triples);
  ui.selection = null;
  // Requests belong to a repository: start on its first one.
  ui.draftRequests = [];
  ui.planId = null;
  ui.openRequest('lead');
}

export async function updateSettings(patch: Partial<Settings>) {
  const prev = store.settings;
  const next = { ...prev, ...patch };
  store.setSettings(next);
  try {
    localStorage.setItem('codly.theme', next.theme);
  } catch {
    /* ignore */
  }
  const saved = await attempt('save settings', () => api.updateSettings(next));
  if (saved) store.setSettings(saved);
  else store.setSettings(prev);
}

const prefsSeq: Record<string, number> = {};
/** Change a request's mode, permissions or Lead CLI and model; shown at once, reverted if refused. */
export async function setRequestPrefs(thread: string, patch: Partial<RequestPrefs>) {
  const prev = store.requestPrefs[thread] ?? { mode: null, approval_mode: null, model: null, provider: null };
  const next = { ...prev, ...patch };
  const seq = (prefsSeq[thread] = (prefsSeq[thread] ?? 0) + 1);
  store.setRequestPrefs(thread, next);
  const saved = await attempt('change the request', () => api.setRequestPrefs(thread, next));
  // Picks in a row (the menu stays open): an older answer never undoes a newer pick.
  if (seq === prefsSeq[thread]) store.setRequestPrefs(thread, saved ?? prev);
}

/** Save the open repo's team changes; shown at once, reverted if the core refuses them. */
export async function saveWorkspaceTeam(next: WorkspaceTeam) {
  const prev = store.workspaceTeam;
  store.setWorkspaceTeam(next);
  const saved = await attempt('save the team', () => api.setWorkspaceTeam(next));
  store.setWorkspaceTeam(saved ?? prev);
}

/** Re-read every CLI's sign-ins and quota. `probe` re-checks who is signed in (a few seconds). */
export async function refreshAccounts(probe = false): Promise<boolean> {
  const list = await attempt('check your sign-ins', () => api.listAccounts(probe));
  if (list) accounts.set(list);
  return !!list;
}

/**
 * Save one CLI's sign-ins (order = preference). Shown at once, then replaced by what the core
 * detects; a refused save is reverted by `updateSettings` and the re-read.
 */
export async function saveAccounts(provider: Provider, next: AccountConfig[], probe = false) {
  accounts.set(applyConfigs(accounts.list, provider, next));
  const s = store.settings;
  const patch: Partial<Settings> = { accounts: withProvider(s.accounts, provider, next) };
  // A Lead pinned to a sign-in that is gone or switched off goes back to "most quota left".
  const leadHere = s.lead_account === provider || !!s.lead_account?.startsWith(`${provider}:`);
  if (leadHere && !next.some((c) => c.enabled && accountId(c.provider, c.home) === s.lead_account)) patch.lead_account = null;
  await updateSettings(patch);
  await refreshAccounts(probe);
}

/**
 * Never blocking: the message shows at once (optimistic) and the call runs in the background.
 * If the thread's agent is mid-turn, Codly queues it; the conversation shows it as Queued.
 */
export function sendChat(thread: string, text: string): boolean {
  const t = text.trim();
  if (!t) return false;
  const id = store.queueOutgoing(thread, t, wallNow());
  void deliver(id, thread, t);
  return true;
}

async function deliver(id: string, thread: string, text: string) {
  try {
    await api.sendChat(thread, text);
  } catch (e) {
    store.failOutgoing(id, msg(e));
    toast({ kind: 'error', title: 'Couldn’t send your message', body: `${msg(e)} — it stays in the conversation; retry it there.` });
  }
}

/** Retry a message that failed to send. */
export function resendChat(id: string) {
  const o = store.outbox.find((x) => x.id === id);
  if (!o) return;
  store.dropOutgoing(id);
  sendChat(o.thread, o.text);
}

/** Send now: stop the agent's current turn; Codly then runs your oldest queued message. */
export async function sendQueuedNow(agentId: string) {
  await attempt('interrupt the agent', () => api.interruptAgent(agentId), 'Your message stays queued and runs when the turn ends.');
}

// Starting a plan can take seconds (integration worktree, dependency install) and has four ways
// in (plan card, Plan room, ⌘↵, palette): a second press meanwhile is dropped, not an error.
// Reactive, so a Run plan button says "Starting…" whichever way the plan was started.
const starting = new SvelteSet<string>();
export const isStarting = (planId: string) => starting.has(planId);

export async function runPlan(planId: string) {
  if (starting.has(planId)) return;
  starting.add(planId);
  try {
    await attempt('run the plan', () => api.runPlan(planId));
  } finally {
    starting.delete(planId);
  }
}

export async function cancelPlan(planId: string) {
  await attempt('cancel the plan', () => api.cancelPlan(planId));
}

/** Close a request's tab: its Lead stops and its unfinished plans are cancelled (branches and
 *  history stay). You land on the tab beside it. */
export async function closeRequest(thread: string): Promise<boolean> {
  const list = requests.list;
  const i = list.findIndex((r) => r.thread === thread);
  const ok = await attempt('close the request', async () => {
    await api.closeThread(thread);
    return true;
  });
  if (!ok) return false;
  ui.draftRequests = ui.draftRequests.filter((d) => d.thread !== thread);
  if (ui.request === thread) ui.openRequest((list[i + 1] ?? list[i - 1])?.thread ?? 'lead');
  await store.resync(); // its messages moved out of the thread without events
  return true;
}

export async function mergePlan(planId: string) {
  const plan = store.planMap.get(planId);
  const target = store.workspace?.current_branch ?? store.workspace?.default_branch ?? 'your branch';
  const id = toast({ kind: 'progress', title: `Merging into ${target}…` });
  try {
    const sha = await api.mergePlan(planId);
    store.markPlanMerged(planId, sha);
    toastsUpdate(id, { kind: 'success', title: `Merged into ${target}`, body: `${plan?.summary ?? 'Plan'} · ${shortSha(sha)}` });
  } catch (e) {
    // The Lead that planned it explains in the request's chat, instead of a bare error.
    const asked = !!plan && sendChat(plan.thread, `Merging “${plan.summary}” into ${target} failed: ${msg(e)}\n\nWhy, and what's the safest way to get it merged?`);
    toastsUpdate(id, { kind: 'error', title: `Couldn’t merge into ${target}`, body: asked ? `${msg(e)} — the Lead is looking into it in the chat.` : msg(e) });
  }
}

export async function resolveApproval(approvalId: string, choice: ApprovalChoice, message: string | null = null, answers: unknown = null) {
  await attempt('send your answer', () =>
    api.resolveApproval(approvalId, { choice, message, answers: (answers ?? null) as never }),
  );
}

export async function moveTask(taskId: string, status: TaskStatus) {
  const t = await attempt(`move the task to ${TASK_STATUS_LABEL[status]}`, () => api.moveTask(taskId, status));
  if (t) store.upsertTask(t);
}

export async function assignTask(taskId: string, agentId: string | null) {
  const t = await attempt('assign the task', () => api.assignTask(taskId, agentId));
  if (t) store.upsertTask(t);
}

export async function createTask(title: string, goal = '') {
  const t = await attempt('create the task', () => api.createTask(title, goal || title));
  if (t) {
    store.upsertTask(t);
    ui.selectTask(t.id);
  }
  return t;
}

/** Returns whether the switch happened. The next message resumes the conversation on the new model. */
export async function setAgentModel(agent: Agent, model: string | null): Promise<boolean> {
  const label = model ?? 'the default model';
  const a = await attempt(`switch ${agent.name} to ${label}`, () => api.setAgentModel(agent.id, model), 'Stop its current turn first, then try again.');
  if (!a) return false;
  store.upsertAgent(a);
  toast({ kind: 'success', title: `${a.name} switched`, body: `${PROVIDER_LABEL[a.provider]} · ${label}` });
  return true;
}

/** Give an agent another model (a new Legend's bigger context window; you clicked it): a 1:1
 *  agent switches in place, a team member's line in the team changes, its effort kept. */
export async function giveModel(agent: Agent, model: string): Promise<boolean> {
  if (agent.role === 'solo') return setAgentModel(agent, model);
  const id = agent.member;
  const done = () => toast({ kind: 'success', title: `${agent.name} switched`, body: `${PROVIDER_LABEL[agent.provider]} · ${model}, from its next task` });
  const global = store.settings.team.find((m) => m.id === id);
  if (id && global) {
    await updateSettings({ team: store.settings.team.map((m) => (m.id === id ? { ...m, model: withModel(m.model, model) } : m)) });
    if (store.settings.team.find((m) => m.id === id)?.model === withModel(global.model, model)) done();
    return true;
  }
  const repo = store.workspaceTeam.members.find((m) => m.id === id);
  if (id && repo) {
    await saveWorkspaceTeam({ ...store.workspaceTeam, members: store.workspaceTeam.members.map((m) => (m.id === id ? { ...m, model: withModel(m.model, model) } : m)) });
    if (store.workspaceTeam.members.find((m) => m.id === id)?.model === withModel(repo.model, model)) done();
    return true;
  }
  toast({ kind: 'info', title: `${agent.name} follows its plan's model`, body: `Pick ${model} for it in Settings.` });
  return false;
}

/** Save how an agent looks in the office (the look editor's picks in its `look_seed`). */
export async function setAgentLook(agent: Agent, lookSeed: string): Promise<boolean> {
  const a = await attempt(`change ${agent.name}'s look`, () => api.setAgentLook(agent.id, lookSeed));
  if (!a) return false;
  store.upsertAgent(a);
  return true;
}

export async function interruptAgent(agentId: string) {
  await attempt('interrupt the agent', () => api.interruptAgent(agentId));
}

export async function stopAgent(agentId: string) {
  await attempt('stop the agent', () => api.stopAgent(agentId));
}

/** Let go an agent you hired: it joins the ghosts by the memory archive (you can bring it back). */
export async function letGo(agentId: string) {
  const a = await attempt('let them go', () => api.letGo(agentId));
  if (!a) return;
  if (ui.selection?.kind === 'agent' && ui.selection.id === agentId) ui.select(null);
  toast({ kind: 'success', title: `${a.name} was let go`, body: 'A ghost by the memory archive now, with the same career and memory. Bring them back from there.' });
}

export async function addMemory(text: string, scope: MemoryScope, kind: MemoryKind, title: string | null = null, tags: string[] | null = null) {
  const m = await attempt('save the memory', () => api.addMemory(text, scope, kind, title, tags));
  if (m) store.upsertMemory(m);
  return m;
}

export async function deleteMemory(id: string) {
  const ok = await attempt('delete the memory', () => api.deleteMemory(id));
  if (ok !== undefined) store.removeMemory(id);
}

export async function redetect() {
  const clis = await attempt('detect CLIs', () => api.detectClis());
  if (clis) store.setClis(clis);
}

export function keepBranch(planId: string) {
  const plan = store.planMap.get(planId);
  ui.keepPlan(planId);
  void api.keepPlan(planId).catch(() => {});
  toast({ kind: 'info', title: 'Branch kept', body: plan?.integration_branch ? `${plan.integration_branch} stays as it is; merge it whenever you like.` : 'The integration branch stays as it is.' });
}

/** Pin or unpin a memory (pinned memories always make the agents' memory pack). */
export function togglePinMemory(m: { id: string; pinned: boolean }) {
  const next = !(m.pinned || ui.pinned.includes(m.id));
  ui.togglePin(m.id);
  const cur = store.memories.find((x) => x.id === m.id);
  if (cur) store.memories = store.memories.map((x) => (x.id === m.id ? { ...x, pinned: next } : x));
  void api.pinMemory(m.id, next).catch(() => {});
}

export async function copyText(text: string, what = 'Copied') {
  try {
    await navigator.clipboard.writeText(text);
    toast({ kind: 'success', title: what, body: text.length > 90 ? text.slice(0, 89) + '…' : text });
  } catch {
    toast({ kind: 'error', title: 'Couldn’t copy', body: 'The clipboard is not available here.' });
  }
}

/** Reveal a file in Finder (Tauri opener plugin); in the browser demo, copy the path instead. */
export async function revealInFinder(path: string) {
  if (!isTauriApp()) {
    await copyText(path, 'Path copied (Finder is only available in the app)');
    return;
  }
  await attempt('reveal the file in Finder', async () => {
    const { revealItemInDir } = await import('@tauri-apps/plugin-opener');
    await revealItemInDir(path);
  }, 'The file may have been moved or deleted.');
}

function toastsUpdate(id: number, patch: Parameters<typeof toasts.update>[1]) {
  toasts.update(id, patch);
}
