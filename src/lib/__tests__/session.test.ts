// What runs beside a live agent's own calls: its MCP servers and the commands it left running
// in the background. The store keeps them from the agent's events; the sheet shows the ones of
// its live run, with "Reconnect" and "Stop".
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { McpServerStatus } from '../gen/McpServerStatus';
import type { Snapshot } from '../gen/Snapshot';
import { MCP_LABEL, backgroundSummary, canReconnect, mcpSummary, ofLiveRun, reconnectAnswer, reconnectMcp, stopBackgroundCommand } from '../state/session';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { mkAgent } from '../../office/demo/script';

const bo = { ...mkAgent('a1', 'Bo', 'claude', 'solo', 0, 'working'), run_id: 'r2' };
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

let id = 0;
/** A snapshot takes the place of everything the store holds. */
const fresh = (o: Partial<Snapshot> = {}) => {
  const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [bo], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [], ...o };
  (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
};
beforeEach(() => {
  id = 0;
  fresh();
});
afterEach(() => vi.restoreAllMocks());

const ev = (ts: number, run: string, kind: EventKind): EventRow => ({ id: ++id, ts, agent_id: 'a1', task_id: null, run_id: run, kind });
const server = (ts: number, run: string, name: string, status: McpServerStatus, error: string | null = null) => ev(ts, run, { type: 'mcp_server', name, status, error });
const command = (ts: number, run: string, task: string, status: string) =>
  ev(ts, run, { type: 'background_command', task_id: task, call_id: `toolu-${task}`, description: `sleep 127; echo ${task}`, status });
const live = () => store.live.a1;
const REFUSED = 'Dynamic Client Registration rejected (HTTP 401): {"error":"unauthorized"}';

describe('the MCP servers of an agent', () => {
  it('each server keeps its place and shows its last state, with the reason Claude Code gave', () => {
    apply(server(10, 'r2', 'docs', 'connected'), server(10, 'r2', 'linear', 'connecting'), server(11, 'r2', 'tracker', 'failed', 'Connection closed'));
    apply(server(12, 'r2', 'linear', 'needs_sign_in', REFUSED));
    const servers = ofLiveRun(live().mcp, bo.run_id);
    expect(servers.map((s) => [s.name, s.status, s.error])).toEqual([
      ['docs', 'connected', null],
      ['linear', 'needs_sign_in', REFUSED],
      ['tracker', 'failed', 'Connection closed'],
    ]);
    expect(mcpSummary(servers)).toBe('1 connected · 1 needs sign-in · 1 did not start');
    // A server that needs a sign-in says so: it is not one that "did not start".
    expect(MCP_LABEL.needs_sign_in).toBe('Needs sign-in');
    expect(servers.map(canReconnect)).toEqual([false, true, true]);
  });

  it('belong to one run: an earlier run\'s servers are gone with its process', () => {
    apply(server(5, 'r1', 'old', 'connected'), server(6, 'r1', 'docs', 'failed', 'Connection closed'));
    expect(ofLiveRun(live().mcp, bo.run_id)).toEqual([]); // nothing of the live run yet
    apply(server(10, 'r2', 'docs', 'connected'));
    expect(live().mcp.map((s) => [s.name, s.status, s.runId])).toEqual([['docs', 'connected', 'r2']]);
    // The agent rests (no live run): no server is shown as connected.
    expect(ofLiveRun(live().mcp, null)).toEqual([]);
  });

  it('a reconnect that does not connect gives Claude Code\'s own answer; one that connects gives none', async () => {
    const reconnect = vi.spyOn(api, 'reconnectAgentMcp').mockRejectedValueOnce(`Claude Code answered: ${REFUSED}`).mockResolvedValueOnce(undefined);
    expect(await reconnectMcp('a1', 'linear')).toBe(`Claude Code answered: ${REFUSED}`);
    expect(await reconnectMcp('a1', 'linear')).toBeNull();
    expect(reconnect).toHaveBeenCalledWith('a1', 'linear');
  });

  it('the answer to a reconnect shows beside the server\'s reason, until the server reports again', () => {
    apply(server(10, 'r2', 'tracker', 'failed', 'Connection closed'));
    const tracker = () => live().mcp[0];
    // You asked about the server as it stood at 10. No answer in time: both lines show.
    const late = { text: 'Claude Code did not answer within 60 s', seen: tracker().ts };
    expect(reconnectAnswer(tracker(), undefined)).toBeNull();
    expect(reconnectAnswer(tracker(), late)).toBe('Claude Code did not answer within 60 s');
    // An answer that is the server's reason again is not said twice.
    expect(reconnectAnswer(tracker(), { text: 'Claude Code answered: Connection closed', seen: 10 })).toBeNull();
    // The try ended after all and the server reports: its own reason is what shows.
    apply(server(75_000, 'r2', 'tracker', 'failed', 'spawn npx ENOENT'));
    expect(tracker().error).toBe('spawn npx ENOENT');
    expect(reconnectAnswer(tracker(), late)).toBeNull();
    // A server that connected has nothing to say.
    apply(server(80_000, 'r2', 'tracker', 'connected'));
    expect(reconnectAnswer(tracker(), { text: 'x', seen: 80_000 })).toBeNull();
  });
});

describe('a snapshot of a run that went on for long', () => {
  const said = (n: number): EventRow => ({ id: n, ts: 1000 + n, agent_id: 'a1', task_id: null, run_id: 'r2', kind: { type: 'message', item_id: `m${n}`, text: 'on it', phase: 'final' } as EventKind });
  /** The run's servers and its running command, from before the snapshot's own events. */
  const beside = () => [server(10, 'r2', 'docs', 'connected'), server(11, 'r2', 'linear', 'needs_sign_in', REFUSED), command(12, 'r2', 'tid-1', 'running')];

  it('still has the run\'s servers and its running commands, though their events left the log', () => {
    fresh({ as_of: 900, events: [said(899), said(900)], session_events: beside() });
    expect(ofLiveRun(live().mcp, bo.run_id).map((s) => [s.name, s.status, s.error, s.ts])).toEqual([
      ['docs', 'connected', null, 10],
      ['linear', 'needs_sign_in', REFUSED, 11],
    ]);
    expect(ofLiveRun(live().background, bo.run_id).map((c) => [c.taskId, c.status, c.startedAt])).toEqual([['tid-1', 'running', 12]]);
    // They are state, not rows of the log: the log starts where the snapshot's events start.
    expect(store.events.map((e) => e.id)).toEqual([899, 900]);
    // What comes live lands on them.
    apply({ ...command(20_000, 'r2', 'tid-1', 'stopped'), id: 901 }, { ...server(20_001, 'r2', 'linear', 'connected'), id: 902 });
    expect(live().mcp.map((s) => [s.name, s.status])).toEqual([['docs', 'connected'], ['linear', 'connected']]);
    expect(live().background.map((c) => [c.taskId, c.status, c.startedAt])).toEqual([['tid-1', 'stopped', 12]]);
  });

  it('keeps them when the agent\'s earlier history is loaded and its state is built again', async () => {
    fresh({ as_of: 900, events: [said(899), said(900)], session_events: beside() });
    // One page of the thread's older rows (newest first): none of them is a server's or a command's.
    vi.spyOn(api, 'threadEvents').mockResolvedValue([said(898), said(897)]);
    await store.loadEarlier('agent:a1');
    expect(live().messages).toHaveLength(4);
    expect(ofLiveRun(live().mcp, bo.run_id).map((s) => s.name)).toEqual(['docs', 'linear']);
    expect(ofLiveRun(live().background, bo.run_id).map((c) => c.taskId)).toEqual(['tid-1']);
  });
});

describe('the commands an agent left running in the background', () => {
  it('a command runs until its own event says it is over, in Claude Code\'s word', () => {
    apply(command(10, 'r2', 'tid-1', 'running'), command(20, 'r2', 'tid-2', 'running'));
    expect(backgroundSummary(live().background)).toBe('2 running');
    apply(command(18_000, 'r2', 'tid-1', 'completed'));
    apply(command(25_000, 'r2', 'tid-2', 'stopped'));
    expect(live().background.map((c) => [c.taskId, c.callId, c.status, c.startedAt, c.ts])).toEqual([
      ['tid-1', 'toolu-tid-1', 'completed', 10, 18_000],
      ['tid-2', 'toolu-tid-2', 'stopped', 20, 25_000],
    ]);
    expect(backgroundSummary(live().background)).toBe('2 commands ended');
  });

  it('a command of an earlier run does not show as running: its process is gone', () => {
    apply(command(5, 'r1', 'tid-9', 'running'));
    expect(ofLiveRun(live().background, bo.run_id)).toEqual([]);
    apply(command(10, 'r2', 'tid-1', 'running'));
    expect(live().background.map((c) => c.taskId)).toEqual(['tid-1']);
  });

  it('keeps every running command and only the last ended ones', () => {
    apply(command(1, 'r2', 'long', 'running'));
    for (let i = 0; i < 12; i++) apply(command(10 + i, 'r2', `t${i}`, 'running'), command(20 + i, 'r2', `t${i}`, 'completed'));
    const kept = live().background;
    expect(kept.filter((c) => c.status === 'running').map((c) => c.taskId)).toEqual(['long']);
    expect(kept.filter((c) => c.status !== 'running').map((c) => c.taskId)).toEqual(['t4', 't5', 't6', 't7', 't8', 't9', 't10', 't11']);
  });

  it('"Stop" names the command by Claude Code\'s id, and a refusal says why', async () => {
    for (const t of toasts.list) toasts.dismiss(t.id);
    const stop = vi.spyOn(api, 'stopAgentTask').mockResolvedValueOnce(undefined).mockRejectedValueOnce('Bo is not running');
    expect(await stopBackgroundCommand('a1', 'tid-2')).toBe(true);
    expect(stop).toHaveBeenCalledWith('a1', 'tid-2');
    expect(toasts.list).toHaveLength(0);
    expect(await stopBackgroundCommand('a1', 'tid-2')).toBe(false);
    expect(toasts.list.map((t) => [t.title, t.body])).toEqual([['Couldn’t stop the command', 'Bo is not running']]);
    for (const t of toasts.list) toasts.dismiss(t.id);
  });
});
