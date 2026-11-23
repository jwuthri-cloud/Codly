// What runs beside a live agent's own calls: its MCP servers and the commands it left running
// in the background. The store keeps them from the agent's events; the sheet shows the ones of
// its live run, with "Reconnect" and "Stop".
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { McpServerStatus } from '../gen/McpServerStatus';
import type { Snapshot } from '../gen/Snapshot';
import { MCP_LABEL, backgroundSummary, canReconnect, mcpSummary, ofLiveRun, reconnectMcp, stopBackgroundCommand } from '../state/session';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { mkAgent } from '../../office/demo/script';

const bo = { ...mkAgent('a1', 'Bo', 'claude', 'solo', 0, 'working'), run_id: 'r2' };
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

let id = 0;
beforeEach(() => {
  id = 0;
  const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [bo], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
  (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
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
