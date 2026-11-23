// The browser demo's transport (mock.ts) from the outside, as the UI uses it: a URL parameter
// of docs/DEMO.md starts it in its state, and the commands of that screen are answered.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Approval } from '../gen/Approval';
import type { EventRow } from '../gen/EventRow';
import type { LotSlot } from '../gen/LotSlot';
import type { MergeState } from '../gen/MergeState';
import type { Office } from '../gen/Office';
import type { Plan } from '../gen/Plan';
import type { Preview } from '../gen/Preview';
import type { RequiredChecks } from '../gen/RequiredChecks';
import type { Snapshot } from '../gen/Snapshot';
import type { Workspace } from '../gen/Workspace';
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

  it('answers Close office: a quiet office closes and its plot is free, one with agents at work stays', async () => {
    const t = open('?ws=1');
    const docs = (await call<LotSlot[]>(t, 'lots')).find((l) => l.name === 'docs-site')!;
    await expect(call(t, 'close_office', { workspaceId: 'demo-office-billing' })).rejects.toThrow('is in the middle of a turn in billing-api');
    const after = await call<LotSlot[]>(t, 'close_office', { workspaceId: docs.workspace_id });
    expect(after.find((l) => l.id === docs.id)).toMatchObject({ owned: true, workspace_id: null, name: null });
    expect((await call<Office[]>(t, 'offices')).map((o) => o.name)).toEqual(['billing-api']);
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
});
