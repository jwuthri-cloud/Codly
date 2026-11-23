// The browser demo's transport (mock.ts) from the outside, as the UI uses it: a URL parameter
// of docs/DEMO.md starts it in its state, and the commands of that screen are answered.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import resumeRs from '../../../crates/codly-core/src/orchestrator/resume.rs?raw';
import { demoDraft, reviewHire } from '../demo/hire';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { EventRow } from '../gen/EventRow';
import type { LotSlot } from '../gen/LotSlot';
import type { MergeState } from '../gen/MergeState';
import type { Office } from '../gen/Office';
import type { Plan } from '../gen/Plan';
import type { Preview } from '../gen/Preview';
import type { PullRequest } from '../gen/PullRequest';
import type { RepoSync } from '../gen/RepoSync';
import type { RequiredChecks } from '../gen/RequiredChecks';
import type { ResumeOffer } from '../gen/ResumeOffer';
import type { Snapshot } from '../gen/Snapshot';
import type { Task } from '../gen/Task';
import type { Workspace } from '../gen/Workspace';
import { HELD } from '../demo/story';
import { MockTransport } from '../mock';
import { intakeUi } from '../state/intake.svelte';
import { ofLiveRun, withMcpServer, type McpServer } from '../state/session';

/** The demo, as the page with this query string starts it. */
function open(search: string): MockTransport {
  vi.stubGlobal('location', { search });
  return new MockTransport();
}

/** One command. The mock answers after a short wait, as a backend does. */
async function call<T>(t: MockTransport, cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const answer = t.call<T>(cmd, args);
  answer.catch(() => {}); // a refusal is read by the test, after the wait
  await vi.advanceTimersByTimeAsync(200);
  return answer;
}

async function snapshot(t: MockTransport): Promise<Snapshot> {
  const s = t.subscribe(() => {});
  await vi.advanceTimersByTimeAsync(50);
  return s;
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date', 'performance'] });
  vi.spyOn(Math, 'random').mockReturnValue(0.5);
  // The story draws one picture on a canvas; a test has none.
  vi.stubGlobal('OffscreenCanvas', class { getContext() { return null; } });
});
afterEach(() => {
  intakeUi.dialog = null;
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('the demo transport', () => {
  it('without a parameter waits for you to open the repo, with nothing turned on', async () => {
    const t = open('');
    expect((await snapshot(t)).workspace).toBeNull();
    expect(intakeUi.dialog).toBeNull();
    expect((await call<RequiredChecks>(t, 'required_checks', { set: null })).on).toBe(false);
  });

  it('`?intake=pr&checks=on`: the dialog is open on the demo repo, your checks are on, and no scripted request starts', async () => {
    const t = open('?intake=pr&checks=on');
    expect(intakeUi.dialog).toBe('pull_request');
    await vi.advanceTimersByTimeAsync(10_000);
    const s = await snapshot(t);
    expect(s.workspace?.name).toBe('acme/web');
    expect(s.chats).toEqual([]);
    expect((await call<RequiredChecks>(t, 'required_checks', { set: null })).on).toBe(true);
    // "Run plans at once" is on there, so the plan of a request from GitHub is one that waits with its reason.
    expect(s.settings.auto_run_plans).toBe(true);
    expect((await snapshot(open(''))).settings.auto_run_plans).toBe(false);
  });

  it('`?merge=conflict&preview=running`: a finished plan is not merged until an agent resolved the conflict, and its preview runs', async () => {
    const t = open('?merge=conflict&preview=running&autoplay=1');
    let plan: Plan | undefined;
    for (let i = 0; i < 300 && !plan; i++) {
      await vi.advanceTimersByTimeAsync(1000);
      plan = (await snapshot(t)).plans.find((p) => p.status === 'done');
    }
    expect(plan, 'a plan finished').toBeDefined();
    const source = { kind: 'plan', id: plan!.id };
    const merge = await call<MergeState>(t, 'merge_state', { source });
    expect(merge.conflicts).toHaveLength(1);
    await expect(call(t, 'merge_plan', { planId: plan!.id, into: null, unchecked: false })).rejects.toThrow(`merging into main conflicts in ${merge.conflicts[0]}`);
    expect((await call<Preview>(t, 'preview', { source, action: null, seen: null })).run).toMatchObject({ command: 'pnpm dev --port $CODLY_PORT_BASE', ended: null });

    let now = await call<MergeState>(t, 'resolve_merge_conflict', { source, into: null });
    expect(now.resolving).toBeTruthy();
    for (let i = 0; i < 120 && now.conflicts.length; i++) {
      await vi.advanceTimersByTimeAsync(1000);
      now = await call<MergeState>(t, 'merge_state', { source });
    }
    expect(now).toMatchObject({ conflicts: [], resolving: null });
    // The plan is done again a moment after its last task.
    await vi.advanceTimersByTimeAsync(5000);
    expect(await call<string>(t, 'merge_plan', { planId: plan!.id, into: null, unchecked: false })).toMatch(/^[0-9a-f]{40}$/);
  });

  it('has two offices, and Close office is refused while one has agents at work or unfinished tasks', async () => {
    const t = open('?ws=1');
    const lots = await call<LotSlot[]>(t, 'lots');
    // No third office: a tab that opened nothing is gone.
    expect(lots.filter((l) => l.owned).map((l) => l.name).sort()).toEqual(['acme/web', 'billing-api']);
    expect((await call<Office[]>(t, 'offices')).map((o) => o.name)).toEqual(['billing-api']);
    await expect(call(t, 'close_office', { workspaceId: 'demo-office-billing' })).rejects.toThrow('is in the middle of a turn in billing-api');
    const home = lots.find((l) => l.name === 'acme/web')!;
    await expect(call(t, 'close_office', { workspaceId: home.workspace_id })).rejects.toThrow('acme/web still has unfinished work (a1 "Upgrade ESLint to v10"): finish or cancel it first');
  });

  describe('`?resume=ask`: work was under way when Codly last closed', () => {
    const offer = (t: MockTransport, go = false) => call<ResumeOffer | null>(t, 'resume_work', { go });
    /** The first plan's tasks, by key. */
    const tasks = async (t: MockTransport) => {
      const s = await snapshot(t);
      const plan = s.plans.find((p) => p.thread === 'lead')!;
      return Object.fromEntries(s.tasks.filter((x) => x.plan_id === plan.id).map((x) => [x.key, x])) as Record<string, Task>;
    };

    it('holds two tasks and their plan when the page loads, and "Resume work" starts them', async () => {
      const t = open('?resume=ask');
      expect(await offer(t)).toEqual({ tasks: 2 });
      // Nothing starts by itself, however long the question is on screen.
      await vi.advanceTimersByTimeAsync(10_000);
      const s = await snapshot(t);
      expect(s.workspace?.name).toBe('acme/web');
      expect(s.plans.map((p) => p.status)).toEqual(['running']);
      const held = await tasks(t);
      // Each says why it waits, in the core's own words.
      expect(HELD).toBe(/pub const HELD: &str = "([^"]+)";/.exec(resumeRs)?.[1]);
      expect([held.t1, held.t2].map((x) => [x.status, x.status_reason, x.attempt, x.branch])).toEqual([
        ['ready', HELD, 1, 'codly/dark-mode/t1'],
        ['ready', HELD, 1, 'codly/dark-mode/t2'],
      ]);
      expect(held.t3.status).toBe('pending');
      // Each keeps its worker of the last run: offline (its process ended with Codly), in the task's worktree.
      const workers = (await snapshot(t)).agents.filter((x) => x.role === 'worker');
      const of = (task: Task) => workers.find((x) => x.id === task.assignee);
      expect([of(held.t1), of(held.t2)].map((x) => [x?.name, x?.status, x?.run_id, x?.task_id, x?.worktree])).toEqual([
        ['Ada', 'offline', null, held.t1.id, held.t1.worktree],
        ['Bo', 'offline', null, held.t2.id, held.t2.worktree],
      ]);

      // Your Resume: nothing waits any more, the tasks go on with the same workers and attempt, and their history says so.
      expect(await offer(t, true)).toBeNull();
      await vi.advanceTimersByTimeAsync(3000);
      const now = await tasks(t);
      expect([now.t1, now.t2].map((x) => [x.status, x.assignee, x.attempt, x.worktree])).toEqual([
        ['running', held.t1.assignee, 1, held.t1.worktree],
        ['running', held.t2.assignee, 1, held.t2.worktree],
      ]);
      const after = await snapshot(t);
      expect(after.agents.filter((x) => x.role === 'worker').map((x) => x.name).sort()).toEqual(['Ada', 'Bo']);
      // No new route: the worker goes on where it stopped.
      expect(after.events.filter((e) => e.kind.type === 'decision' && e.kind.decision_type === 'route' && [held.t1.id, held.t2.id].includes(e.task_id ?? ''))).toEqual([]);
      const resumed = (await snapshot(t)).events.filter((e) => e.kind.type === 'decision' && e.kind.decision_type === 'retry');
      expect(resumed.map((e) => [e.task_id, e.kind.type === 'decision' && e.kind.summary])).toEqual([
        [held.t1.id, 'Resumed after restart: its worker goes on where it stopped'],
        [held.t2.id, 'Resumed after restart: its worker goes on where it stopped'],
      ]);
      expect(await offer(t)).toBeNull();
    });

    it('keeps the hold after "Not now": nothing starts, also not a plan you run meanwhile', async () => {
      // ("Not now" sends nothing: the bar keeps its button, and the core is only read)
      const t = open('?resume=ask');
      expect(await offer(t)).toEqual({ tasks: 2 });
      // The second request is planned meanwhile (the script opens it): run its plan.
      let second: Plan | undefined;
      for (let i = 0; i < 120 && !second; i++) {
        await vi.advanceTimersByTimeAsync(1000);
        second = (await snapshot(t)).plans.find((p) => p.thread !== 'lead' && p.status === 'awaiting_approval');
      }
      expect(second, 'the second plan is ready').toBeDefined();
      await call(t, 'run_plan', { planId: second!.id });
      await vi.advanceTimersByTimeAsync(20_000);
      const s = await snapshot(t);
      expect(s.tasks.filter((x) => ['running', 'verifying', 'merging', 'done'].includes(x.status) && x.plan_id)).toEqual([]);
      expect(s.agents.filter((a) => a.role === 'worker' && a.status !== 'offline')).toEqual([]);
      // Its first task says why it waits, in the core's words; the count is still of the two that were in work.
      expect(s.tasks.find((x) => x.plan_id === second!.id && x.key === 't1')).toMatchObject({ status: 'ready', status_reason: HELD });
      expect(await offer(t)).toEqual({ tasks: 2 });

      expect(await offer(t, true)).toBeNull();
      await vi.advanceTimersByTimeAsync(5000);
      const after = await snapshot(t);
      expect(after.tasks.filter((x) => x.status === 'running').map((x) => x.key).sort()).toEqual(['t1', 't1', 't2']);
    });

    /** The tasks of the first plan that were started since the page loaded. */
    const started = async (t: MockTransport) => Object.values(await tasks(t)).filter((x) => !['pending', 'ready', 'cancelled'].includes(x.status)).map((x) => x.key);
    /** The workers at work now (the held tasks' workers of the last run are offline). */
    const working = async (t: MockTransport) => (await snapshot(t)).agents.filter((x) => x.role === 'worker' && x.status !== 'offline').map((x) => x.name);

    it('counts one less when you cancel a waiting task, and ends by itself when none is left', async () => {
      const t = open('?resume=ask');
      // You read the question first: the plan's coordinator waits for your answer by then.
      await vi.advanceTimersByTimeAsync(2000);
      const held = await tasks(t);
      // t3 waits for t1: it is cancelled with it. One task still waits.
      await call(t, 'cancel_task_tree', { taskId: held.t1.id });
      expect(await offer(t)).toEqual({ tasks: 1 });
      await call(t, 'move_task', { taskId: held.t2.id, status: 'cancelled' });
      expect(await offer(t)).toBeNull();
      // The hold is over, and what you cancelled stays cancelled: no worker starts on it.
      await vi.advanceTimersByTimeAsync(5000);
      expect(Object.values(await tasks(t)).map((x) => x.status)).toEqual(['cancelled', 'cancelled', 'cancelled']);
      expect(await started(t)).toEqual([]);
      expect(await working(t)).toEqual([]);
      // No hold is left: a task you start from here on needs no Resume.
      const task = await call<Task>(t, 'create_task', { title: 'Sort the imports', goal: '' });
      await call(t, 'move_task', { taskId: task.id, status: 'ready' });
      await vi.advanceTimersByTimeAsync(3000);
      expect((await snapshot(t)).tasks.find((x) => x.id === task.id)?.status).toBe('running');
    });

    it('does not start a waiting task that you cancelled before your Resume', async () => {
      const t = open('?resume=ask');
      await vi.advanceTimersByTimeAsync(2000);
      const held = await tasks(t);
      await call(t, 'move_task', { taskId: held.t2.id, status: 'cancelled' });
      expect(await offer(t)).toEqual({ tasks: 1 });
      expect(await offer(t, true)).toBeNull();
      await vi.advanceTimersByTimeAsync(5000);
      // The other one goes on; t3 needs both, so it waits on.
      const now = await tasks(t);
      expect([now.t1.status, now.t2.status, now.t3.status]).toEqual(['running', 'cancelled', 'pending']);
      expect(await started(t)).toEqual(['t1']);
      expect(await working(t)).toEqual(['Ada']);
    });

    it('holds nothing without the parameter', async () => {
      expect(await offer(open('?ws=1'))).toBeNull();
    });
  });

  it('opens billing-api as an office of its own, and acme/web is as you left it when you come back', async () => {
    const t = open('?ws=1');
    await vi.advanceTimersByTimeAsync(8000);
    const a1 = await snapshot(t);
    expect(a1.workspace?.name).toBe('acme/web');
    expect(a1.chats.length).toBeGreaterThan(0);
    // What the tab of the other office says before you open it.
    const tab = (await call<Office[]>(t, 'offices')).find((o) => o.name === 'billing-api')!;
    expect(tab).toMatchObject({ id: 'demo-office-billing', repo_path: '~/code/billing-api', crew: 3, working: 2, needs_you: 1 });

    // Open it: the snapshot is that office's, with what its tab said, and nothing of acme/web.
    const ws = await call<Workspace>(t, 'open_workspace', { repoPath: tab.repo_path });
    expect(ws).toMatchObject({ id: tab.id, name: 'billing-api' });
    const b = await snapshot(t);
    expect(b.workspace).toEqual(ws);
    expect(b.agents.filter((a) => a.role !== 'lead').map((a) => a.name).sort()).toEqual(['Otto', 'Pia', 'Rex']);
    expect([...new Set(b.chats.map((c) => c.thread))]).toEqual(['lead']);
    expect(b.tasks.map((x) => x.status)).toEqual(['running', 'running']);
    const waiting = b.approvals.filter((x) => x.status === 'pending');
    expect(waiting.map((x) => x.subject)).toEqual([expect.objectContaining({ kind: 'command', command: 'docker compose up -d stripe-mock' })]);
    const ours = new Set(a1.agents.map((x) => x.id));
    expect(b.agents.some((x) => ours.has(x.id))).toBe(false);
    // From there, acme/web is the other office: counted as it stands, and on its plot of the map.
    const others = await call<Office[]>(t, 'offices');
    expect(others.map((o) => o.name)).not.toContain('billing-api');
    expect(others.find((o) => o.id === a1.workspace!.id)).toMatchObject({ name: 'acme/web', repo_path: a1.workspace!.repo_path });
    const plots = await call<LotSlot[]>(t, 'lots');
    expect(plots.find((l) => l.workspace_id === a1.workspace!.id)).toMatchObject({ name: 'acme/web', template: 'studio' });
    expect(plots.find((l) => l.workspace_id === tab.id)).toMatchObject({ name: 'billing-api', crew: 3 });
    // Commands are this office's: the request that waits here is answered here.
    await call(t, 'resolve_approval', { approvalId: waiting[0].id, decision: { choice: 'allow_once', message: null, answers: null } });
    // A repo the demo has no office for opens nothing: you stay where you are.
    expect((await call<Workspace>(t, 'open_workspace', { repoPath: '~/code/docs-site' })).id).toBe(tab.id);

    // Back: the first office with what it had, and what it did meanwhile.
    const back = await call<Workspace>(t, 'open_workspace', { repoPath: a1.workspace!.repo_path });
    expect(back.id).toBe(a1.workspace!.id);
    const seen: EventRow[] = [];
    const again = t.subscribe((batch) => seen.push(...batch.events));
    await vi.advanceTimersByTimeAsync(50);
    const a2 = await again;
    expect(a2.workspace).toEqual(a1.workspace);
    expect(a2.as_of).toBeGreaterThan(a1.as_of);
    const kept = new Map(a2.events.map((e) => [e.id, e]));
    for (const e of a1.events) expect(kept.get(e.id)).toEqual(e);
    expect(a2.chats.slice(0, a1.chats.length)).toEqual(a1.chats);
    expect(a2.approvals.some((x) => x.id === waiting[0].id)).toBe(false);
    // billing-api keeps working: its tab counts it as it stands, and its events come with its id.
    await vi.advanceTimersByTimeAsync(5000);
    expect((await call<Office[]>(t, 'offices')).find((o) => o.id === tab.id)).toMatchObject({ crew: 3, needs_you: 0 });
    const elsewhere = seen.filter((e) => e.workspace_id);
    expect(elsewhere.length).toBeGreaterThan(0);
    expect(elsewhere.every((e) => e.workspace_id === tab.id && e.id !== 0)).toBe(true);
    expect(seen.some((e) => !e.workspace_id)).toBe(true);
  });

  it('`?approval=permissions`: the repo is open, a request of that kind waits, and your answer closes it', async () => {
    const t = open('?approval=permissions');
    let waiting: Approval | undefined;
    for (let i = 0; i < 20 && !waiting; i++) {
      await vi.advanceTimersByTimeAsync(1000);
      waiting = (await snapshot(t)).approvals.find((x) => x.subject.kind === 'permissions');
    }
    expect(waiting).toMatchObject({ status: 'pending', choices: ['allow_once', 'allow_session', 'deny'] });
    await call(t, 'resolve_approval', { approvalId: waiting!.id, decision: { choice: 'deny', message: null, answers: null } });
    expect((await snapshot(t)).approvals.find((x) => x.id === waiting!.id)).toMatchObject({ status: 'denied', decided_by: 'user' });
  });

  it('shows the MCP servers of the Lead, and Reconnect connects the one that did not start after a second', async () => {
    const t = open('?ws=1');
    await vi.advanceTimersByTimeAsync(4000);
    const lead = (await snapshot(t)).agents.find((x) => x.role === 'lead')!;
    /** The agent sheet's list (state/session.ts), from the events of a snapshot. */
    const servers = async () => {
      let list: McpServer[] = [];
      for (const e of (await snapshot(t)).events) if (e.agent_id === lead.id && e.kind.type === 'mcp_server') list = withMcpServer(list, e.kind, e.run_id, e.ts);
      return ofLiveRun(list, lead.run_id).map((s) => [s.name, s.status]);
    };
    expect(await servers()).toEqual([['codly', 'connected'], ['linear', 'needs_sign_in'], ['postgres', 'failed']]);

    /** Reconnect answers when Claude Code's try is over: a second of the demo's time. */
    const reconnect = async (server: string) => {
      const answer = t.call('reconnect_agent_mcp', { agentId: lead.id, server });
      answer.catch(() => {});
      await vi.advanceTimersByTimeAsync(600);
      expect((await servers()).find(([name]) => name === server)![1]).not.toBe('connected');
      await vi.advanceTimersByTimeAsync(900);
      return answer;
    };
    expect(await reconnect('postgres')).toBeNull();
    await expect(reconnect('linear')).rejects.toThrow('linear still needs a sign-in');
    expect(await servers()).toEqual([['codly', 'connected'], ['linear', 'needs_sign_in'], ['postgres', 'connected']]);

    // "Turn off" and "Turn on": the server's state follows. Codly's own server stays on.
    expect(await call(t, 'set_agent_mcp_enabled', { agentId: lead.id, server: 'postgres', enabled: false })).toBeNull();
    expect(await servers()).toEqual([['codly', 'connected'], ['linear', 'needs_sign_in'], ['postgres', 'disabled']]);
    expect(await call(t, 'set_agent_mcp_enabled', { agentId: lead.id, server: 'postgres', enabled: true })).toBeNull();
    expect(await servers()).toEqual([['codly', 'connected'], ['linear', 'needs_sign_in'], ['postgres', 'connected']]);
    await expect(call(t, 'set_agent_mcp_enabled', { agentId: lead.id, server: 'codly', enabled: false })).rejects.toThrow("codly is Codly's own server");
    expect(await servers()).toEqual([['codly', 'connected'], ['linear', 'needs_sign_in'], ['postgres', 'connected']]);
  });

  it('`?hour=23.995`: the last minute of today, not the first of tomorrow', () => {
    const before = Date;
    const today = new Date().getDate();
    try {
      open('?hour=23.995');
      const now = new Date();
      expect([now.getDate(), now.getHours(), now.getMinutes()]).toEqual([today, 23, 59]);
    } finally {
      globalThis.Date = before;
    }
  });

  it('`?hour=13.5`: the demo’s clock starts at that hour of the day, and goes on from there', async () => {
    const before = Date;
    try {
      open('?hour=13.5');
      const now = new Date();
      expect([now.getHours(), now.getMinutes(), now.getSeconds()]).toEqual([13, 30, 0]);
      expect(now).toBeInstanceOf(Date);
      expect(Date.now()).toBe(now.getTime());
      // A date made from a value is that value.
      expect(new Date(86_400_000).getTime()).toBe(86_400_000);
      await vi.advanceTimersByTimeAsync(60_000);
      expect([new Date().getHours(), new Date().getMinutes()]).toEqual([13, 31]);
    } finally {
      globalThis.Date = before;
    }
  });

  /** Let the scripted story play until its first plan is done (`?autoplay=1`). */
  async function finished(t: MockTransport): Promise<Plan> {
    let plan: Plan | undefined;
    for (let i = 0; i < 300 && !plan; i++) {
      await vi.advanceTimersByTimeAsync(1000);
      plan = (await snapshot(t)).plans.find((p) => p.thread === 'lead' && p.status === 'done');
    }
    expect(plan, 'a plan finished').toBeDefined();
    return plan!;
  }

  it('counts the commits not on origin yet from what you did: a merge adds them, an undo and a push take them away', async () => {
    const t = open('?autoplay=1&ws=1');
    const plan = await finished(t);
    const ahead = async () => (await call<RepoSync>(t, 'repo_sync')).ahead;
    /** What `git status` says in the terminal. */
    const status = async () => {
      const id = await call<string>(t, 'pty_open', { cwd: null });
      let out = '';
      await t.attachPty(id, (bytes) => (out += new TextDecoder().decode(bytes)));
      await call(t, 'pty_write', { ptyId: id, data: 'git status\r' });
      return out.split('\n').find((l) => l.startsWith('Your branch'))?.trim();
    };
    expect(await ahead()).toBe(0);
    expect(await status()).toBe("Your branch is up to date with 'origin/main'.");

    // The commit of each of its three tasks, and the merge commit.
    const sha = await call<string>(t, 'merge_plan', { planId: plan.id, into: null, unchecked: false });
    expect(await ahead()).toBe(4);
    expect(await status()).toBe("Your branch is ahead of 'origin/main' by 4 commits.");
    expect((await call<RepoSync>(t, 'undo_merge', { sha })).ahead).toBe(0);

    await call(t, 'merge_plan', { planId: plan.id, into: null, unchecked: false });
    expect((await call<RepoSync>(t, 'push_checkout')).ahead).toBe(0);
    expect(await status()).toBe("Your branch is up to date with 'origin/main'.");
  });

  it('"Keep the branch" answers with the kept plan', async () => {
    const t = open('?autoplay=1&ws=1');
    const plan = await finished(t);
    expect(plan.kept).toBe(false);
    expect(await call<Plan>(t, 'keep_plan', { planId: plan.id })).toMatchObject({ id: plan.id, kept: true, status: 'done' });
    expect((await snapshot(t)).plans.find((p) => p.id === plan.id)!.kept).toBe(true);
    await expect(call(t, 'keep_plan', { planId: 'p-nobody' })).rejects.toThrow('Unknown plan');
  });

  it('`?pr=open`: the finished plan has an open pull request, and Refresh reads its checks running, then passed', async () => {
    const t = open('?pr=open&autoplay=1');
    const plan = await finished(t);
    const source = { kind: 'plan', id: plan.id };
    const pr = await call<PullRequest>(t, 'pull_request', { source });
    expect(pr).toMatchObject({ state: 'open', checks: null, branch: plan.integration_branch, base: 'main' });
    // The same one each time it is read, also from the dialog.
    expect((await call<PullRequest>(t, 'pull_request', { source })).url).toBe(pr.url);
    expect((await call<{ pr_state: string | null }>(t, 'draft_pull_request', { source })).pr_state).toBe('open');
    expect((await call<PullRequest>(t, 'refresh_pull_request', { source })).checks).toMatchObject({ passed: 1, pending: 2, failed: 0 });
    expect((await call<PullRequest>(t, 'refresh_pull_request', { source })).checks).toMatchObject({ passed: 3, pending: 0, failed: 0 });
    // Without the parameter no pull request is open before you open one.
    const plain = open('?autoplay=1&ws=1');
    const done = await finished(plain);
    expect(await call(plain, 'pull_request', { source: { kind: 'plan', id: done.id } })).toBeNull();
  });

  it('`?search=fail`: a code search fails with a reason; without it, it answers', async () => {
    await expect(call(open('?search=fail'), 'search_code', { query: 'theme' })).rejects.toThrow('the code index could not be read');
    expect((await call<{ query: string }>(open('?ws=1'), 'search_code', { query: 'theme' })).query).toBe('theme');
  });

  // The last test of this file: the demo's neighbourhood is one for the page, and an office that closed stays closed.
  it('answers Close office: the open office closes once its tasks are over, and its plot is free', async () => {
    // (`?intake=issue` opens the repo and starts no scripted request: no agent is in a turn)
    const t = open('?intake=issue');
    const s = await snapshot(t);
    const home = (await call<LotSlot[]>(t, 'lots')).find((l) => l.name === 'acme/web')!;
    expect(home.workspace_id).toBe(s.workspace!.id);
    for (const task of s.tasks.filter((x) => x.status !== 'done')) await call(t, 'move_task', { taskId: task.id, status: 'cancelled' });
    // An agent you hired is at rest, with its run.
    const dot = await call<Agent>(t, 'hire_from_manifest', { manifest: reviewHire(demoDraft('Keep the docs up to date')).manifest, confirmShell: false });
    expect(dot.run_id).toBeTruthy();
    const after = await call<LotSlot[]>(t, 'close_office', { workspaceId: home.workspace_id });
    expect(after.find((l) => l.id === home.id)).toMatchObject({ owned: true, workspace_id: null });
    // Its agents stopped, and no run is left: their processes ended with the office.
    expect((await snapshot(t)).agents.map((x) => [x.name, x.status, x.run_id])).toEqual([['Dot', 'offline', null]]);
    expect(after.find((l) => l.name === 'billing-api')).toMatchObject({ owned: true, workspace_id: 'demo-office-billing' });
  });
});
