// User intents → core commands. Every action reports failure as a toast that says how to get
// out, and applies command results the core does not echo as events (workspace, settings).
import type { RepoSync } from '../gen/RepoSync';
import type { LastMerge } from '../gen/LastMerge';
import { SvelteSet } from 'svelte/reactivity';
import { api, isTauriApp } from '../api';
import type { Agent } from '../gen/Agent';
import type { AccountConfig } from '../gen/AccountConfig';
import type { ApprovalChoice } from '../gen/ApprovalChoice';
import type { ApprovalRuleKind } from '../gen/ApprovalRuleKind';
import type { HeldCheck } from '../gen/HeldCheck';
import type { MemoryKind } from '../gen/MemoryKind';
import type { MemoryScope } from '../gen/MemoryScope';
import type { Provider } from '../gen/Provider';
import type { Rank } from '../gen/Rank';
import type { Settings } from '../gen/Settings';
import type { TaskStatus } from '../gen/TaskStatus';
import type { WorkspaceTeam } from '../gen/WorkspaceTeam';
import type { RequestPrefs } from '../gen/RequestPrefs';
import { PROVIDER_LABEL, TASK_STATUS_LABEL } from '../util/labels';
import { shortSha } from '../util/format';
import { accountId, applyConfigs, withProvider } from '../util/accounts';
import { trustOf } from '../util/career';
import { withModel } from '../util/team';
import { followUpRequest, reviewMessage } from '../util/review';
import { accounts } from './accounts.svelte';
import { diffstats } from './diffstats.svelte';
import { fileList, loadMergeState, resolveMergeConflict } from './mergeState.svelte';
import { flushClosing } from './closing';
import { busy } from './queue';
import { requests } from './requests.svelte';
import { review, reviewKey } from './review.svelte';
import { noteStop, stopping } from './stops';
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
  // Requests closed a moment ago are closed for good first, while their office is the open one.
  await flushClosing();
  const ws = await attempt('open the repository', () => api.openWorkspace(repo), 'Pick the folder that contains .git.');
  if (!ws) return;
  // Requests belong to a repository. Where you were in this one is remembered before the
  // office changes: from here on the tabs and the composer are the next one's.
  ui.leaveOffice(store.workspace?.id ?? null);
  store.setWorkspace(ws);
  diffstats.reset();
  // A workspace switch is a reconnect: take a fresh snapshot for the new repo.
  await store.resync();
  if (!store.workspace) store.setWorkspace(ws);
  const [memories, triples] = await Promise.all([api.memories().catch(() => null), api.triples().catch(() => null)]);
  if (!store.isOpen(ws.id)) return; // you switched again meanwhile: that switch owns the screen
  if (memories) store.setMemories(memories);
  if (triples) store.setTriples(triples);
  ui.selection = null;
  ui.planId = null;
  // You land on the request you last had open here, when it is still one; else on the first.
  const last = ui.enterOffice(ws.id);
  const list = requests.list;
  ui.openRequest((list.find((r) => r.thread === last) ?? list[0])?.thread ?? 'lead');
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
  const ws = store.workspace?.id;
  store.setRequestPrefs(thread, next);
  const saved = await attempt('change the request', () => api.setRequestPrefs(thread, next));
  // Picks in a row (the menu stays open): an older answer never undoes a newer pick. Thread
  // names repeat across offices: an answer for the office you left is not this one's.
  if (seq === prefsSeq[thread] && store.isOpen(ws)) store.setRequestPrefs(thread, saved ?? prev);
}

/** Save the open repo's team changes; shown at once, reverted if the core refuses them. */
export async function saveWorkspaceTeam(next: WorkspaceTeam) {
  const prev = store.workspaceTeam;
  const ws = store.workspace?.id;
  store.setWorkspaceTeam(next);
  const saved = await attempt('save the team', () => api.setWorkspaceTeam(next));
  if (store.isOpen(ws)) store.setWorkspaceTeam(saved ?? prev);
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
  // Review comments that waited in this request's composer are sent now: they leave their diff.
  review.sent(store.workspace?.id ?? null, thread);
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

/** Send now: stop the agent's current turn, so that it reads your oldest queued message at once. */
export async function sendQueuedNow(agentId: string) {
  const undo = noteStop(agentId);
  const ok = await attempt('interrupt the agent', async () => {
    await api.interruptAgent(agentId);
    return true;
  }, 'Your message stays queued.');
  if (!ok) undo();
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

/** Keep what is merged: the plan's unfinished tasks are cancelled and it finishes with the merged ones. */
export async function finishPlan(planId: string) {
  await attempt('finish the plan with what is merged', () => api.finishPlan(planId));
}

/** Close a request's tab: its Lead stops and its unfinished plans are cancelled (branches and
 *  history stay). You land on the tab beside it. */
export async function closeRequest(thread: string): Promise<boolean> {
  const list = requests.list;
  const i = list.findIndex((r) => r.thread === thread);
  const ws = store.workspace?.id ?? null;
  const ok = await attempt('close the request', async () => {
    await api.closeThread(thread);
    return true;
  });
  if (!ok) return false;
  ui.draftRequests = ui.draftRequests.filter((d) => d.thread !== thread);
  review.unhand(ws, thread); // review comments it held and did not send stay on their diff
  if (ui.request === thread) ui.openRequest((list[i + 1] ?? list[i - 1])?.thread ?? 'lead');
  await store.resync(); // its messages moved out of the thread without events
  return true;
}

/** Merge a done plan into your checked-out branch. When that is not the branch the plan started
 *  from, nothing is merged yet: a toast says what the merge would bring and offers to go on
 *  (`into` is then the branch you confirmed). A merge that stops says why and how to go on: a
 *  conflict can go to an agent, and checks that failed on the merge result can be skipped
 *  (`unchecked`). */
export async function mergePlan(planId: string, into: string | null = null, unchecked = false) {
  const plan = store.planMap.get(planId);
  if (!into) {
    const warning = await attempt('merge the plan', () => api.mergePlanWarning(planId));
    if (warning === undefined) return;
    if (warning) {
      const goOn = { label: 'Merge anyway', run: () => void mergePlan(planId, warning.target, unchecked) };
      toast({ kind: 'attention', title: `Merge into ${warning.target}?`, body: warning.message, action: goOn, duration: 0 });
      return;
    }
  }
  const target = into ?? store.workspace?.current_branch ?? store.workspace?.default_branch ?? 'your branch';
  const id = toast({ kind: 'progress', title: `Merging into ${target}…` });
  try {
    const sha = await api.mergePlan(planId, into, unchecked);
    store.markPlanMerged(planId, sha);
    toastsUpdate(id, { kind: 'success', title: `Merged into ${target}`, body: `${plan?.summary ?? 'Plan'} · ${shortSha(sha)}` });
  } catch (e) {
    // What stands in the way, as the core reads it now. Your branch did not move.
    const source = { kind: 'plan' as const, id: planId };
    const state = await loadMergeState(source);
    if (state?.conflicts.length) {
      const body = `${state.target} and this plan both changed ${fileList(state.conflicts)}. ${state.target} is unchanged. An agent can resolve it in Codly’s own worktree, or you can do it yourself: see the plan’s card.`;
      const resolve = { label: 'Have an agent resolve it', run: () => void resolveMergeConflict(source, null, into) };
      toastsUpdate(id, { kind: 'error', title: `Couldn’t merge into ${state.target}`, body, action: resolve, duration: 0 });
      return;
    }
    if (state?.failed_checks.length || state?.checks_not_run) {
      const goOn = { label: 'Merge anyway', run: () => void mergePlan(planId, into, true) };
      toastsUpdate(id, { kind: 'error', title: `Not merged into ${state.target}`, body: msg(e), action: goOn, duration: 0 });
      return;
    }
    // The Lead that planned it explains in the request's chat, instead of a bare error. Only in
    // the plan's own office: a merge can take minutes, and the chat goes to the open one.
    const asked = !!plan && store.isOpen(plan.workspace_id) && sendChat(plan.thread, `Merging “${plan.summary}” into ${target} failed: ${msg(e)}\n\nWhy, and what's the safest way to get it merged?`);
    toastsUpdate(id, { kind: 'error', title: `Couldn’t merge into ${target}`, body: asked ? `${msg(e)} — the Lead is looking into it in the chat.` : msg(e) });
  }
}

/** Merge a solo agent's branch into your branch; failures are explained in a toast. */
export async function mergeAgent(agentId: string): Promise<boolean> {
  const a = store.agentMap.get(agentId);
  const target = store.workspace?.current_branch ?? store.workspace?.default_branch ?? 'your branch';
  const id = toast({ kind: 'progress', title: `Merging ${a?.name ?? 'the agent'}’s work into ${target}…` });
  try {
    const sha = await api.mergeAgent(agentId);
    toastsUpdate(id, { kind: 'success', title: `Merged into ${target}`, body: `${a?.name ?? 'Agent'} · ${shortSha(sha)}` });
    return true;
  } catch (e) {
    // A conflict can go back to the agent: it resolves it in its own worktree. Not while it
    // is on it already: then the core's own words say what is left to do.
    const source = { kind: 'agent' as const, id: agentId };
    const state = await loadMergeState(source);
    if (state?.conflicts.length && !state.resolving) {
      const who = a?.name ?? 'the agent';
      const body = `${state.target} and ${who} both changed ${fileList(state.conflicts)}. ${state.target} is unchanged.`;
      const resolve = { label: `Have ${who} resolve it`, run: () => void resolveMergeConflict(source, a?.name ?? null) };
      toastsUpdate(id, { kind: 'error', title: `Couldn’t merge into ${state.target}`, body, action: resolve, duration: 0 });
      return false;
    }
    toastsUpdate(id, { kind: 'error', title: `Couldn’t merge into ${target}`, body: msg(e) });
    return false;
  }
}

/** Undo the last merge Codly made into your branch: the branch goes back to where it was, and
 *  the merged work stays on its own branch. A refusal says why (pushed, later commits…). */
export async function undoMerge(merge: LastMerge): Promise<RepoSync | undefined> {
  const id = toast({ kind: 'progress', title: `Undoing the merge into ${merge.branch}…` });
  try {
    const s = await api.undoMerge(merge.sha);
    if (merge.plan_id) store.unmarkPlanMerged(merge.plan_id);
    toastsUpdate(id, { kind: 'success', title: 'Merge undone', body: `${merge.branch} is back to where it was before the merge.` });
    return s;
  } catch (e) {
    toastsUpdate(id, { kind: 'error', title: 'Couldn’t undo the merge', body: msg(e) });
    return undefined;
  }
}

export async function commitCheckout(message: string): Promise<boolean> {
  const sha = await attempt('commit your changes', () => api.commitCheckout(message));
  if (sha) toast({ kind: 'success', title: 'Committed', body: shortSha(sha), duration: 3000 });
  return !!sha;
}

export async function pushCheckout(): Promise<RepoSync | undefined> {
  const branch = store.workspace?.current_branch ?? 'your branch';
  const id = toast({ kind: 'progress', title: `Pushing ${branch}…` });
  try {
    const s = await api.pushCheckout();
    toastsUpdate(id, { kind: 'success', title: `Pushed ${branch}`, body: s.upstream ? `${s.upstream} is up to date` : undefined });
    return s;
  } catch (e) {
    toastsUpdate(id, { kind: 'error', title: `Couldn’t push ${branch}`, body: msg(e) });
    return undefined;
  }
}

export async function resolveApproval(approvalId: string, choice: ApprovalChoice, message: string | null = null, answers: unknown = null) {
  await attempt('send your answer', () =>
    api.resolveApproval(approvalId, { choice, message, answers: (answers ?? null) as never }),
  );
}

/** One answer for the requests that share a card: the same command asked by several agents. */
export async function resolveApprovals(approvalIds: string[], choice: ApprovalChoice, message: string | null = null, answers: unknown = null) {
  await Promise.all(approvalIds.map((id) => resolveApproval(id, choice, message, answers)));
}

/** "Always allow in this repo": save the rule a card offers. The core answers the requests it covers. */
export async function saveApprovalRule(approvalId: string, kind: ApprovalRuleKind) {
  const rule = await attempt('save the rule', () => api.addApprovalRule(approvalId, kind));
  if (!rule) return;
  const what = rule.kind === 'prefix' ? `commands that start with ${rule.command}` : rule.command;
  toast({ kind: 'success', title: 'Codly will not ask about this again', body: `In this repo: ${what}. Remove the rule in Settings → Safety.` });
}

// What a command answers belongs to the office it was sent from: after a switch it is dropped
// here (that office's next snapshot has it), never added to the one you are looking at now.

export async function moveTask(taskId: string, status: TaskStatus) {
  const ws = store.workspace?.id;
  const t = await attempt(`move the task to ${TASK_STATUS_LABEL[status]}`, () => api.moveTask(taskId, status));
  if (t && store.isOpen(ws)) store.upsertTask(t);
}

/** Cancel a task together with the tasks that wait for it. */
export async function cancelTaskTree(taskId: string) {
  const ws = store.workspace?.id;
  const tasks = await attempt('cancel the tasks', () => api.cancelTaskTree(taskId));
  if (tasks && store.isOpen(ws)) for (const t of tasks) store.upsertTask(t);
}

/** Re-run a stopped task's checks. Answers the commands that need your OK first (none = the checks are running). */
export async function recheckTask(taskId: string, allow: string[] = []): Promise<HeldCheck[]> {
  return (await attempt('re-run the checks', () => api.recheckTask(taskId, allow))) ?? [];
}

/** Merge anyway: the task goes on without its checks. */
export async function acceptTask(taskId: string) {
  const ws = store.workspace?.id;
  const t = await attempt('merge the task', () => api.acceptTask(taskId));
  if (t && store.isOpen(ws)) store.upsertTask(t);
}

/** The request whose composer holds a diff's comments since Send back (`reviewKey`), while it is still a tab. */
export function waitingRequest(key: string): string | null {
  const to = review.handedTo(key);
  return to && store.isOpen(to.office) && requests.get(to.thread) ? to.thread : null;
}

/**
 * Send back: every comment you wrote on a diff, as one message (file, line, the quoted line, the
 * comment). A task that is not done gets them through the core: its worker reads them and Codly
 * checks the work again. Merged or finished work (a done task, a whole plan) opens a new request
 * with the comments ready to send: they stay on the diff until you send that message, and Send
 * back again goes to the same request (with the comments as they are now). `paths`: the diff's
 * files, in order. True: the comments went to the worker, or are in a request's composer; false:
 * nothing happened (nothing to send, or the core refused and a toast says why).
 */
export async function sendBack(target: { kind: 'plan' | 'task'; id: string }, paths: string[]): Promise<boolean> {
  const key = reviewKey(target);
  const list = review.comments(key);
  const task = target.kind === 'task' ? (store.taskMap.get(target.id) ?? null) : null;
  const planId = target.kind === 'plan' ? target.id : task?.plan_id;
  const plan = (planId && store.planMap.get(planId)) || null;
  if (!list.length || (!task && !plan)) return false;
  if (task && task.status !== 'done') {
    const ws = store.workspace?.id;
    const t = await attempt('send your comments back', () => api.sendBackTask(task.id, reviewMessage(list, paths)), 'your comments stay on the diff.');
    if (!t) return false;
    if (store.isOpen(ws)) store.upsertTask(t);
    review.clear(key);
    const who = (t.assignee && store.agentMap.get(t.assignee)?.name) || 'its worker';
    toast({ kind: 'success', title: `${t.key} sent back to ${who}`, body: list.length === 1 ? '1 comment' : `${list.length} comments` });
    return true;
  }
  const text = followUpRequest(task, plan, list, paths);
  const waiting = waitingRequest(key);
  const was = review.handedTo(key)?.text ?? null;
  if (!waiting) {
    ui.newRequest();
    ui.seedComposer(ui.request, text);
  } else if (was === text) ui.openRequest(waiting);
  else ui.seedComposer(waiting, text, false, was);
  review.hand(key, { office: store.workspace?.id ?? null, thread: ui.request, text });
  return true;
}

export async function assignTask(taskId: string, agentId: string | null) {
  const ws = store.workspace?.id;
  const t = await attempt('assign the task', () => api.assignTask(taskId, agentId));
  if (t && store.isOpen(ws)) store.upsertTask(t);
}

export async function createTask(title: string, goal = '') {
  const ws = store.workspace?.id;
  const t = await attempt('create the task', () => api.createTask(title, goal || title));
  if (t && store.isOpen(ws)) {
    store.upsertTask(t);
    ui.selectTask(t.id);
  }
  return t;
}

/** Returns whether the switch happened. A live Claude agent switches in place; any other
 *  resumes the conversation on the new model with its next message. The core's refusal says
 *  what to do (stop its turn first, wait for its task, pick a model the CLI knows). */
export async function setAgentModel(agent: Agent, model: string | null): Promise<boolean> {
  const label = model ?? 'the default model';
  const ws = store.workspace?.id;
  const a = await attempt(`switch ${agent.name} to ${label}`, () => api.setAgentModel(agent.id, model));
  if (!a) return false;
  if (store.isOpen(ws)) store.upsertAgent(a);
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
  const ws = store.workspace?.id;
  const a = await attempt(`change ${agent.name}'s look`, () => api.setAgentLook(agent.id, lookSeed));
  if (!a) return false;
  if (store.isOpen(ws)) store.upsertAgent(a);
  return true;
}

/** Whether the core took the interrupt (a button that says "Interrupting…" needs to know). */
export async function interruptAgent(agentId: string): Promise<boolean> {
  const undo = noteStop(agentId);
  const ok = await attempt('interrupt the agent', async () => {
    await api.interruptAgent(agentId);
    return true;
  });
  if (!ok) undo();
  return !!ok;
}

/** Stop the running turn of a thread's agent (the Stop button, Esc). False: no turn was running. */
export function stopThread(thread: string): boolean {
  const agent = store.threadAgent(thread);
  if (!agent || !busy(thread)) return false;
  // A second press while the CLI lets go asks nothing new.
  if (!stopping(agent.id)) void interruptAgent(agent.id);
  return true;
}

export async function stopAgent(agentId: string) {
  const undo = noteStop(agentId);
  const ok = await attempt('stop the agent', async () => {
    await api.stopAgent(agentId);
    return true;
  });
  if (!ok) undo();
}

/** Let go an agent you hired: it joins the ghosts by the memory archive (you can bring it back). */
export async function letGo(agentId: string) {
  const a = await attempt('let them go', () => api.letGo(agentId));
  if (!a) return;
  if (ui.selection?.kind === 'agent' && ui.selection.id === agentId) ui.select(null);
  toast({ kind: 'success', title: `${a.name} was let go`, body: 'A ghost by the memory archive now, with the same career and memory. Bring them back from there.' });
}

/** Set how much you trust an agent (`TRUST`): the core saves it and records the change as a decision. */
export async function setTrust(agentId: string, level: Rank): Promise<boolean> {
  const a = await attempt('change its trust', () => api.trustAgent(agentId, level));
  if (!a) return false;
  const t = trustOf(a.career.trust);
  toast({ kind: a.career.trust === 'intern' ? 'info' : 'success', title: `${a.name}: ${t.label.toLowerCase()}`, body: t.allows });
  return true;
}

export async function addMemory(text: string, scope: MemoryScope, kind: MemoryKind, title: string | null = null, tags: string[] | null = null) {
  const ws = store.workspace?.id;
  const m = await attempt('save the memory', () => api.addMemory(text, scope, kind, title, tags));
  if (m && (!m.workspace_id || store.isOpen(ws))) store.upsertMemory(m); // a global memory is every office's
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

/** Keep a finished plan's branch instead of merging it. The merge prompt goes once the core has recorded it. */
export async function keepBranch(planId: string): Promise<boolean> {
  const plan = store.planMap.get(planId);
  const ok = await attempt('keep the branch', async () => {
    await api.keepPlan(planId);
    return true;
  });
  if (!ok) return false;
  ui.keepPlan(planId);
  toast({ kind: 'info', title: 'Branch kept', body: plan?.integration_branch ? `${plan.integration_branch} stays as it is; merge it whenever you like.` : 'The integration branch stays as it is.' });
  return true;
}

/** Pin or unpin a memory (pinned memories always make the agents' memory pack). The pin shows once the core has saved it. */
export async function togglePinMemory(m: { id: string; pinned: boolean }): Promise<boolean> {
  const next = !(m.pinned || ui.pinned.includes(m.id));
  const ok = await attempt(next ? 'pin the memory' : 'unpin the memory', async () => {
    await api.pinMemory(m.id, next);
    return true;
  });
  if (!ok) return false;
  ui.setPin(m.id, next);
  store.memories = store.memories.map((x) => (x.id === m.id ? { ...x, pinned: next } : x));
  return true;
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
