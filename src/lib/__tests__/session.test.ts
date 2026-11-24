// What runs beside a live agent's own calls: its MCP servers and what it left at work in the
// background (commands and helpers, as Claude Code lists them at every change). The store keeps
// them from the agent's events; the sheet shows the ones of its live run, with "Reconnect" and
// "Stop".
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { McpServerStatus } from '../gen/McpServerStatus';
import type { Snapshot } from '../gen/Snapshot';
import { HOLD_MS, KEEP_STOPPED_MS, MCP_ASK_WAIT_MS, MCP_BUSY, MCP_HINT, MCP_LABEL, STOP_WAIT_MS, backgroundKind, backgroundRows, backgroundSummary, canReconnect, canRestart, canStop, cliAnswer, mcpAnswered, mcpAskOnItsWay, mcpAsked, mcpButtonLabel, mcpRowLocked, mcpStartFailure, mcpSummary, mcpSwitch, mcpSwitchHint, ofLiveRun, reconnectAnswer, reconnectMcp, restartHint, restartMcp, rowOrder, showsMcpSection, signInSteps, startedWithoutFolderConfig, stopBackgroundTask, stopHint, stopLabel, stopOnItsWay, switchMcp, waitedHelperStopHint, type BackgroundWork } from '../state/session';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { mkAgent } from '../../office/demo/script';

const bo = { ...mkAgent('a1', 'Bo', 'claude', 'solo', 0, 'working'), run_id: 'r2' };
const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);

let id = 0;
/** A snapshot takes the place of everything the store holds. */
const snapshot = (o: Partial<Snapshot>) =>
  ({ as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents: [bo], plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [], ...o }) as unknown as Snapshot;
const fresh = (o: Partial<Snapshot> = {}) => (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snapshot(o));
/** A snapshot of the office that is open (the event stream fell behind, or connected again): it is merged into what the store holds. `missed`: the stored rows since. */
const merge = (o: Partial<Snapshot>, missed: EventRow[] = []) => (store as unknown as { mergeSnapshot(s: Snapshot, missed: EventRow[]): void }).mergeSnapshot(snapshot(o), missed);
beforeEach(() => {
  id = 0;
  fresh();
});
afterEach(() => vi.restoreAllMocks());

const ev = (ts: number, run: string, kind: EventKind): EventRow => ({ id: ++id, ts, agent_id: 'a1', task_id: null, run_id: run, kind });
const server = (ts: number, run: string, name: string, status: McpServerStatus, error: string | null = null) => ev(ts, run, { type: 'mcp_server', name, status, error });
const task = (id: string, kind = 'command') => ({ id, kind, description: kind === 'command' ? `sleep 127; echo ${id}` : `Look at ${id}` });
/** Claude Code's whole list of background tasks (commands, by id), with what left it since the list before: [id, Claude Code's word]. */
const list = (ts: number, run: string, tasks: string[], ended: [string, string][] = []) =>
  ev(ts, run, { type: 'background_tasks', tasks: tasks.map((id) => task(id)), ended: ended.map(([id, status]) => ({ ...task(id), status })) });
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

  it('a server that needs a sign-in says where to sign in, by its name, and what to do then', () => {
    expect(signInSteps('linear')).toBe(
      'Open Claude Code in a terminal and sign in to linear in Claude Code’s own MCP settings. Then press Reconnect here. If this server uses an API key, check the key in its settings instead.',
    );
    expect(MCP_HINT.needs_sign_in).toBe('The agent works without this server’s tools until you sign in to it in Claude Code’s own MCP settings. Then press Reconnect.');
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
    // Whatever the times: an answer about the server as it stands now is no line under one
    // that works, nor under one you switched off.
    expect(reconnectAnswer(tracker(), { ...late, seen: tracker().ts })).toBeNull();
    apply(server(90_000, 'r2', 'tracker', 'disabled'));
    expect(reconnectAnswer(tracker(), { ...late, seen: tracker().ts })).toBeNull();
  });

  it('a connected server can be turned off and a switched-off one on; Codly\'s own stays on', () => {
    apply(
      server(10, 'r2', 'codly', 'connected'),
      server(10, 'r2', 'docs', 'connected'),
      server(10, 'r2', 'old', 'disabled'),
      server(10, 'r2', 'slow', 'connecting'),
      server(11, 'r2', 'tracker', 'failed', 'Connection closed'),
      server(12, 'r2', 'linear', 'needs_sign_in', REFUSED),
    );
    const servers = ofLiveRun(live().mcp, bo.run_id);
    // Each row: its switch, and whether it has Reconnect. Never both.
    expect(servers.map((s) => [s.name, mcpSwitch(s)?.label ?? null, canReconnect(s)])).toEqual([
      ['codly', null, false],
      ['docs', 'Turn off', false],
      ['old', 'Turn on', false],
      ['slow', null, false],
      ['tracker', null, true],
      ['linear', null, true],
    ]);
    expect(mcpSwitch(servers[1])).toEqual({ label: 'Turn off', busy: 'Turning off…', enabled: false });
    expect(mcpSwitch(servers[2])).toEqual({ label: 'Turn on', busy: 'Turning on…', enabled: true });
    // Recorded (part 5, section 1, n=2): Claude Code saves the switch for the folder, and it
    // holds for every later Claude Code run there until "Turn on". Before, the hint said
    // "Claude Code may keep the switch after this run".
    expect(mcpSwitchHint('Bo')).toBe('Turns this server off for Claude Code in this folder: for Bo now and for every later run here, until you turn it on.');
    expect(MCP_HINT.disabled).toBe('This server is switched off for Claude Code in this folder: the agent works without its tools. Turn on starts it again.');
    expect(mcpSummary(servers)).toBe('2 connected · 1 connecting · 1 needs sign-in · 1 did not start · 1 switched off');
  });

  it('the switch goes to Claude Code, and a refusal is said with its reason', async () => {
    for (const t of toasts.list) toasts.dismiss(t.id);
    const flip = vi.spyOn(api, 'setAgentMcpEnabled').mockResolvedValueOnce(undefined).mockRejectedValueOnce('Claude Code answered: Connection closed');
    expect(await switchMcp('a1', 'docs', false)).toBe(true);
    expect(flip).toHaveBeenLastCalledWith('a1', 'docs', false);
    expect(toasts.list).toEqual([]);
    expect(await switchMcp('a1', 'docs', true)).toBe(false);
    expect(flip).toHaveBeenLastCalledWith('a1', 'docs', true);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['error', 'docs did not start: the connection closed. It stays switched on: Reconnect tries again.', undefined]]);
    for (const t of toasts.list) toasts.dismiss(t.id);
  });

  // Recorded (docs/research/cli-claude-stream-json-2.1.311-part4.md, section 7): a reconnect of a
  // connected server answers and starts it again. Before, a connected server had no such button.
  it('a connected server can be restarted; one that did not start keeps Reconnect; Codly\'s own has neither', () => {
    apply(server(10, 'r2', 'codly', 'connected'), server(10, 'r2', 'docs', 'connected'), server(11, 'r2', 'tracker', 'failed', 'Connection closed'), server(12, 'r2', 'linear', 'needs_sign_in', REFUSED), server(13, 'r2', 'old', 'disabled'), server(14, 'r2', 'slow', 'connecting'));
    const servers = ofLiveRun(live().mcp, bo.run_id);
    expect(servers.map((s) => [s.name, canRestart(s), canReconnect(s)])).toEqual([
      ['codly', false, false],
      ['docs', true, false],
      ['tracker', false, true],
      ['linear', false, true],
      ['old', false, false],
      ['slow', false, false],
    ]);
    expect(restartHint('Bo')).toBe('Starts this server again for this run of Bo.');
  });

  it('a restart is the request of Reconnect; one that does not take is said with Claude Code\'s reason', async () => {
    for (const t of toasts.list) toasts.dismiss(t.id);
    const again = vi.spyOn(api, 'reconnectAgentMcp').mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error('Claude Code answered: spawn npx ENOENT'));
    expect(await restartMcp('a1', 'docs')).toBe(true);
    expect(again).toHaveBeenLastCalledWith('a1', 'docs');
    expect(toasts.list).toEqual([]);
    expect(await restartMcp('a1', 'docs')).toBe(false);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['error', 'docs did not start: spawn npx ENOENT.', undefined]]);
    for (const t of toasts.list) toasts.dismiss(t.id);
    // A refusal of Codly's own reached no Claude Code: it is said as it is.
    again.mockRejectedValueOnce(new Error('Bo is not running'));
    expect(await restartMcp('a1', 'docs')).toBe(false);
    expect(toasts.list.map((t) => [t.title, t.body])).toEqual([['Couldn’t restart docs', 'Bo is not running']]);
    for (const t of toasts.list) toasts.dismiss(t.id);
  });

  // Recorded (part 5, section 9, `r7-mcp-cannot-start-*`, n=2): a "Turn on" or a reconnect of a
  // server that cannot start is answered with an error in Claude Code's words. Before, the toast
  // showed them as they came, "stdio" as if it were the missing program.
  it('a server that does not start says why in plain words, never "stdio" as its program', () => {
    expect(mcpStartFailure('no-such-command', 'Executable not found in $PATH: "stdio"', false)).toBe('no-such-command did not start: its program was not found on your PATH.');
    expect(mcpStartFailure('ping-flaky', 'Connection closed', false)).toBe('ping-flaky did not start: the connection closed.');
    expect(mcpStartFailure('docs', 'spawn npx ENOENT', false)).toBe('docs did not start: spawn npx ENOENT.');
    for (const words of ['Executable not found in $PATH: "stdio"', 'Connection closed']) expect(mcpStartFailure('x', words, false)).not.toContain('stdio');
    // A "Turn on" that failed still counts as on for the saved switch.
    expect(mcpStartFailure('ping-flaky', 'Connection closed', true)).toBe('ping-flaky did not start: the connection closed. It stays switched on: Reconnect tries again.');
  });

  it('a "Turn on" that fails says why in a toast; a "Turn off" that fails says it could not', async () => {
    for (const t of toasts.list) toasts.dismiss(t.id);
    vi.spyOn(api, 'setAgentMcpEnabled')
      .mockRejectedValueOnce('Claude Code answered: Executable not found in $PATH: "stdio"')
      .mockRejectedValueOnce('Bo is not running')
      .mockRejectedValueOnce('that agent is still starting; try again in a moment')
      .mockRejectedValueOnce('Claude Code did not answer within 10 s');
    expect(await switchMcp('a1', 'no-such-command', true)).toBe(false);
    expect(await switchMcp('a1', 'docs', false)).toBe(false);
    expect(toasts.list.map((t) => [t.title, t.body])).toEqual([
      ['no-such-command did not start: its program was not found on your PATH. It stays switched on: Reconnect tries again.', undefined],
      ['Couldn’t turn docs off', 'Bo is not running'],
    ]);
    for (const t of toasts.list) toasts.dismiss(t.id);
    // A refusal of Codly's own, or no answer: nothing reached Claude Code, nothing was switched on.
    expect(await switchMcp('a1', 'docs', true)).toBe(false);
    expect(await switchMcp('a1', 'docs', true)).toBe(false);
    expect(toasts.list.map((t) => [t.title, t.body])).toEqual([
      ['Couldn’t turn docs on', 'that agent is still starting; try again in a moment'],
      ['Couldn’t turn docs on', 'Claude Code did not answer within 10 s'],
    ]);
    expect(cliAnswer('Claude Code answered: Connection closed')).toBe('Connection closed');
    expect(cliAnswer('Bo is not running')).toBeNull();
    for (const t of toasts.list) toasts.dismiss(t.id);
  });

  it('a busy button says what it does to a screen reader too', () => {
    expect(mcpButtonLabel(MCP_BUSY.off, 'docs')).toBe('Turning off… docs');
    expect(mcpButtonLabel('Restart', 'docs')).toBe('Restart docs');
    // One place for the busy words: the switch reads them from there.
    expect(mcpSwitch({ name: 'docs', status: 'connected', error: null, runId: 'r2', ts: 1 })?.busy).toBe(MCP_BUSY.off);
  });

  // Before, the button was free again as soon as Claude Code answered: a second press sent a
  // second request and wrote a second record.
  it('a button stays busy until the server\'s next status row arrives, or 10 s pass after the answer', () => {
    apply(server(1_000, 'r2', 'docs', 'connected'));
    const docs = () => ofLiveRun(live().mcp, bo.run_id)[0];
    const ask = mcpAsked(docs(), 'off');
    expect(mcpAskOnItsWay(docs(), undefined, 1_000)).toBeNull();
    expect(mcpAskOnItsWay(docs(), ask, 1_500)).toEqual(ask);
    expect(MCP_BUSY[ask.what]).toBe('Turning off…');
    const back = mcpAnswered(ask, 2_000, false);
    expect(mcpAskOnItsWay(docs(), back, 2_000 + MCP_ASK_WAIT_MS - 1)).toEqual(back);
    expect(mcpAskOnItsWay(docs(), back, 2_000 + MCP_ASK_WAIT_MS)).toBeNull();
    // The server reports: the row shows where it stands, and the request is no longer on its way.
    apply(server(3_000, 'r2', 'docs', 'disabled'));
    expect(mcpAskOnItsWay(docs(), back, 3_000)).toBeNull();
    expect([MCP_BUSY.reconnect, MCP_BUSY.restart, MCP_BUSY.on]).toEqual(['Reconnecting…', 'Restarting…', 'Turning on…']);
  });

  // A server that hangs at start: Claude Code answers when its try is over (30 s and more), and
  // no status row comes meanwhile. A second press would send a second request.
  it('a request Claude Code has not answered stays on its way, also after 10 s', () => {
    apply(server(1_000, 'r2', 'tracker', 'failed', 'Connection closed'));
    const tracker = () => ofLiveRun(live().mcp, bo.run_id)[0];
    const ask = mcpAsked(tracker(), 'reconnect');
    expect(mcpAskOnItsWay(tracker(), ask, 1_000 + MCP_ASK_WAIT_MS + 30_000)).toEqual(ask);
    expect(mcpRowLocked(tracker(), ask, 1_000 + MCP_ASK_WAIT_MS + 30_000)).toBe(true);
  });

  // A restart leaves the server "connected": no status row follows, so its answer ends it.
  it('a restart ends with its answer, and the row stays off for a moment only', () => {
    apply(server(1_000, 'r2', 'docs', 'connected'));
    const docs = () => ofLiveRun(live().mcp, bo.run_id)[0];
    const done = mcpAnswered(mcpAsked(docs(), 'restart'), 1_200, true);
    expect(mcpAskOnItsWay(docs(), done, 1_200)).toBeNull();
    expect(mcpRowLocked(docs(), done, 1_200 + HOLD_MS - 1)).toBe(true);
    expect(mcpRowLocked(docs(), done, 1_200 + HOLD_MS)).toBe(false);
  });

  // A row's buttons change with its state: the one under the pointer must not take a press meant for the one before.
  it('the row\'s buttons stay off while its request is on its way and for a moment after its state changed', () => {
    apply(server(1_000, 'r2', 'tracker', 'failed', 'Connection closed'));
    const tracker = () => ofLiveRun(live().mcp, bo.run_id)[0];
    const ask = mcpAnswered(mcpAsked(tracker(), 'reconnect'), 1_500, false);
    expect(mcpRowLocked(tracker(), undefined, 1_000)).toBe(false);
    expect(mcpRowLocked(tracker(), ask, 2_000)).toBe(true);
    // It connected: "Restart" and "Turn off" are there, and off for HOLD_MS.
    apply(server(4_000, 'r2', 'tracker', 'connected'));
    expect(mcpRowLocked(tracker(), ask, 4_000 + HOLD_MS - 1)).toBe(true);
    expect(mcpRowLocked(tracker(), ask, 4_000 + HOLD_MS)).toBe(false);
  });

  it('says when the live run started without the folder\'s own CLI configuration', () => {
    const notice = (run: string, text: string) => ev(5, run, { type: 'notice', text });
    const TEXT = 'Started without this folder\'s Claude Code configuration (its settings, hooks and MCP servers): you have not trusted these files. See Settings → Safety.';
    expect(startedWithoutFolderConfig([notice('r2', TEXT)], 'a1', 'r2')).toBe(true);
    // A run started later loads what you trusted since: an earlier run's notice is not about it.
    expect(startedWithoutFolderConfig([notice('r1', TEXT)], 'a1', 'r2')).toBe(false);
    expect(startedWithoutFolderConfig([notice('r2', 'Goal: ship it')], 'a1', 'r2')).toBe(false);
    expect(startedWithoutFolderConfig([notice('r2', TEXT)], 'a1', null)).toBe(false);
  });

  // Before, the line lived in the section of the servers, which shows only when the run
  // reported one: a run without any never said that it started without the folder's own files.
  it('the MCP section shows for such a run also when it reported no server', () => {
    const docs = { name: 'docs', status: 'connected' as const, error: null, runId: 'r2', ts: 1 };
    expect(showsMcpSection([], true)).toBe(true);
    expect(showsMcpSection([docs], false)).toBe(true);
    expect(showsMcpSection([], false)).toBe(false);
  });
});

describe('a snapshot of a run that went on for long', () => {
  const said = (n: number): EventRow => ({ id: n, ts: 1000 + n, agent_id: 'a1', task_id: null, run_id: 'r2', kind: { type: 'message', item_id: `m${n}`, text: 'on it', phase: 'final' } as EventKind });
  /** The run's servers and its last list of background tasks, from before the snapshot's own events. */
  const beside = () => [server(10, 'r2', 'docs', 'connected'), server(11, 'r2', 'linear', 'needs_sign_in', REFUSED), list(12, 'r2', ['tid-1'])];

  it('still has the run\'s servers and what it runs in the background, though their events left the log', () => {
    fresh({ as_of: 900, events: [said(899), said(900)], session_events: beside() });
    expect(ofLiveRun(live().mcp, bo.run_id).map((s) => [s.name, s.status, s.error, s.ts])).toEqual([
      ['docs', 'connected', null, 10],
      ['linear', 'needs_sign_in', REFUSED, 11],
    ]);
    // (the list was read from the snapshot: when its work started is not known)
    expect(ofLiveRun(live().background, bo.run_id).map((w) => [w.id, w.status, w.startedAt])).toEqual([['tid-1', 'running', null]]);
    // They are state, not rows of the log: the log starts where the snapshot's events start.
    expect(store.events.map((e) => e.id)).toEqual([899, 900]);
    // What comes live lands on them.
    apply({ ...list(20_000, 'r2', [], [['tid-1', 'stopped']]), id: 901 }, { ...server(20_001, 'r2', 'linear', 'connected'), id: 902 });
    expect(live().mcp.map((s) => [s.name, s.status])).toEqual([['docs', 'connected'], ['linear', 'connected']]);
    expect(live().background.map((w) => [w.id, w.status, w.startedAt, w.endedAt])).toEqual([['tid-1', 'stopped', null, 20_000]]);
  });

  it('keeps them when the agent\'s earlier history is loaded and its state is built again', async () => {
    fresh({ as_of: 900, events: [said(899), said(900)], session_events: beside() });
    // One page of the thread's older rows (newest first): none of them is a server's or a list.
    vi.spyOn(api, 'threadEvents').mockResolvedValue([said(898), said(897)]);
    await store.loadEarlier('agent:a1');
    expect(live().messages).toHaveLength(4);
    expect(ofLiveRun(live().mcp, bo.run_id).map((s) => s.name)).toEqual(['docs', 'linear']);
    expect(ofLiveRun(live().background, bo.run_id).map((w) => w.id)).toEqual(['tid-1']);
  });

  // Every run reports each of its servers when it starts. One that stands where an earlier run
  // recorded it gets no new row in the log: its event has id 0, live and in a snapshot.
  describe('with servers that stand where an earlier run recorded them', () => {
    /** The earlier run's rows, and the live run's servers that changed nothing. */
    const earlier = () => [{ ...server(5, 'r1', 'docs', 'connected'), id: 898 }, said(899), said(900)];
    const unchanged = () => [{ ...server(50, 'r2', 'docs', 'connected'), id: 0 }, { ...server(50, 'r2', 'linear', 'needs_sign_in', REFUSED), id: 0 }];
    const servers = () => ofLiveRun(live().mcp, bo.run_id).map((s) => [s.name, s.status]);

    it('shows them for the live run, with no row in the log', () => {
      fresh({ as_of: 900, events: earlier(), session_events: unchanged() });
      // The earlier run's row is in the snapshot's events: it does not take their place.
      expect(servers()).toEqual([['docs', 'connected'], ['linear', 'needs_sign_in']]);
      expect(store.events.map((e) => e.id)).toEqual([898, 899, 900]);
    });

    it('takes one that comes live, and adds no row for it', () => {
      fresh({ as_of: 900, events: earlier() });
      expect(servers()).toEqual([]); // the earlier run's server is not the live run's
      apply(...unchanged());
      expect(servers()).toEqual([['docs', 'connected'], ['linear', 'needs_sign_in']]);
      expect(store.events.map((e) => e.id)).toEqual([898, 899, 900]);
      // A server that changes has its row, and lands on the same list.
      apply({ ...server(60, 'r2', 'docs', 'failed', 'Connection closed'), id: 901 });
      expect(servers()).toEqual([['docs', 'failed'], ['linear', 'needs_sign_in']]);
      expect(store.events.map((e) => e.id)).toEqual([898, 899, 900, 901]);
    });

    // The run started while the open office's event stream fell behind: its servers' live events
    // are lost, and no stored row holds them. The snapshot that catches up has them.
    it('shows them when their live events were lost and a snapshot is merged into the open office', () => {
      fresh({ as_of: 900, events: earlier() });
      expect(servers()).toEqual([]);
      merge({ as_of: 900, events: earlier(), session_events: unchanged() });
      expect(servers()).toEqual([['docs', 'connected'], ['linear', 'needs_sign_in']]);
      expect(store.events.map((e) => e.id)).toEqual([898, 899, 900]);
    });

    it('a merged snapshot gives a server that changed from its missed row, and the others beside it', () => {
      fresh({ as_of: 900, events: earlier() });
      // "docs" failed in the live run: that is a stored row. "linear" stands as recorded.
      const failed = { ...server(60, 'r2', 'docs', 'failed', 'Connection closed'), id: 901 };
      merge({ as_of: 901, events: [...earlier(), failed], session_events: [unchanged()[1]] }, [failed]);
      expect(servers()).toEqual([['docs', 'failed'], ['linear', 'needs_sign_in']]);
      expect(store.events.map((e) => e.id)).toEqual([898, 899, 900, 901]);
    });

    it('keeps them when the agent\'s earlier history is loaded and its state is built again', async () => {
      fresh({ as_of: 900, events: earlier(), session_events: unchanged() });
      vi.spyOn(api, 'threadEvents').mockResolvedValue([said(897), { ...server(4, 'r1', 'old', 'failed', 'Connection closed'), id: 896 }]);
      await store.loadEarlier('agent:a1');
      expect(live().messages).toHaveLength(3);
      expect(servers()).toEqual([['docs', 'connected'], ['linear', 'needs_sign_in']]);
    });
  });
});

describe('what an agent left at work in the background', () => {
  const rows = () => live().background.map((w) => [w.id, w.status, w.startedAt, w.endedAt]);

  it('work runs while Claude Code lists it, and ends in Claude Code\'s word when the list drops it', () => {
    apply(list(10, 'r2', ['tid-1']), list(20, 'r2', ['tid-1', 'tid-2']));
    expect(backgroundSummary(live().background)).toBe('2 running');
    // A row starts with the first list that names it, and keeps that time across the lists.
    expect(rows()).toEqual([['tid-1', 'running', 10, null], ['tid-2', 'running', 20, null]]);
    apply(list(18_000, 'r2', ['tid-2'], [['tid-1', 'completed']]));
    expect(rows()).toEqual([['tid-2', 'running', 20, null], ['tid-1', 'completed', 10, 18_000]]);
    apply(list(25_000, 'r2', [], [['tid-2', 'stopped']]));
    expect(rows()).toEqual([['tid-1', 'completed', 10, 18_000], ['tid-2', 'stopped', 20, 25_000]]);
    // Closed, the section says how the work ended, as its rows do (not "2 ended").
    expect(backgroundSummary(live().background)).toBe('1 completed · 1 stopped');
  });

  // Recorded (docs/research/cli-claude-stream-json-2.1.311-part3.md, section 1): `stop_task` stops a
  // helper of the list as it stops a command. Before, a helper had no "Stop".
  it('a helper is listed beside the commands; a command and a helper that run have "Stop", another kind of task has none', () => {
    const tasks = [task('tid-1'), task('tid-3', 'helper'), task('tid-4', 'remote')];
    apply(ev(10, 'r2', { type: 'background_tasks', tasks, ended: [] }));
    expect(live().background.map((w) => [w.id, backgroundKind(w), canStop(w)])).toEqual([['tid-1', null, true], ['tid-3', 'helper', true], ['tid-4', 'task', false]]);
    // What ended has no "Stop": the command, and the helper.
    apply(ev(20, 'r2', { type: 'background_tasks', tasks: [tasks[2]], ended: [{ ...tasks[0], status: 'completed' }, { ...tasks[1], status: 'stopped' }] }));
    expect(live().background.map((w) => [w.id, w.status, canStop(w)])).toEqual([['tid-4', 'running', false], ['tid-1', 'completed', false], ['tid-3', 'stopped', false]]);
  });

  // Recorded: with no turn running the agent takes a turn of its own (part 3, section 1); a stop
  // while a turn runs is told within that turn, and no turn follows (part 5, section 11, n=2).
  it('the "Stop" of a helper says what follows, by whether a turn of its agent runs', () => {
    apply(ev(10, 'r2', { type: 'background_tasks', tasks: [task('tid-1'), task('tid-3', 'helper')], ended: [] }));
    const [command, helper] = live().background;
    expect(stopHint(helper, 'Bo', false)).toBe('Stops this helper. Bo is told and goes on with a turn of its own.');
    expect(stopHint(helper, 'Bo', true)).toBe('Stops this helper. Bo is told in the turn that runs.');
    expect(stopHint(command, 'Bo', false)).toBeNull();
    expect(stopHint(command, 'Bo', true)).toBeNull();
    // A helper its agent still waits for: its turn goes on.
    expect(waitedHelperStopHint('Bo')).toBe('Stops this helper. Bo’s turn goes on.');
  });

  it('each "Stop" says what it stops, to a screen reader', () => {
    expect(stopLabel({ kind: 'command', description: 'npm run dev' })).toBe('Stop the command npm run dev');
    expect(stopLabel({ kind: 'helper', description: 'Run the tests' })).toBe('Stop the helper Run the tests');
    // While its stop is on its way, the name is what the button shows.
    expect(stopLabel({ kind: 'command', description: 'npm run dev' }, true)).toBe('Stopping… the command npm run dev');
  });

  // Before, a stopped row left the list at once, the next row's "Stop" took its place under the
  // pointer, and it was off for a moment.
  it('a row you stopped keeps its place for 2 s with how it ended, then the list closes up', () => {
    const w = (id: string, status: string, endedAt: number | null = null): BackgroundWork => ({ id, kind: 'command', description: id, status, runId: 'r2', startedAt: 1, endedAt });
    const before = [w('a', 'running'), w('b', 'running'), w('c', 'running')];
    const order = rowOrder([], before);
    expect(order).toEqual(['a', 'b', 'c']);
    // "a" was stopped at 10 s: Claude Code's next list puts the running ones first.
    const after = [w('b', 'running'), w('c', 'running'), w('a', 'stopped', 10_000)];
    const ids = (r: { running: BackgroundWork[]; ended: BackgroundWork[] }) => [r.running.map((x) => `${x.id}:${x.status}`), r.ended.map((x) => x.id)];
    expect(ids(backgroundRows(after, rowOrder(order, after), new Set(['a']), 10_000 + KEEP_STOPPED_MS - 1))).toEqual([['a:stopped', 'b:running', 'c:running'], []]);
    expect(ids(backgroundRows(after, rowOrder(order, after), new Set(['a']), 10_000 + KEEP_STOPPED_MS))).toEqual([['b:running', 'c:running'], ['a']]);
    // Work that ended by itself leaves its place at once; new work comes last.
    const more = [w('b', 'running'), w('d', 'running'), w('c', 'completed', 10_000)];
    expect(rowOrder(order, more)).toEqual(['b', 'c', 'd']);
    expect(ids(backgroundRows(more, rowOrder(order, more), new Set(), 10_000))).toEqual([['b:running', 'd:running'], ['c']]);
  });

  it('a row reads "Stopping…" from the stop Claude Code took until its list drops the work, for 10 s at most', () => {
    expect(stopOnItsWay(undefined, 50_000)).toBe(false); // no stop was asked
    expect(stopOnItsWay(50_000, 50_000)).toBe(true);
    expect(stopOnItsWay(50_000, 50_000 + STOP_WAIT_MS - 1)).toBe(true);
    // The row still runs after the wait: that request did not stop it, and "Stop" is there again.
    expect(stopOnItsWay(50_000, 50_000 + STOP_WAIT_MS)).toBe(false);
  });

  it('work that a list drops without a word is over too, and work that was never seen running has no start', () => {
    apply(list(10, 'r2', ['tid-1', 'tid-2']));
    // The list of its end is not at hand (the window missed it): the next list is still the whole list.
    apply(list(30, 'r2', ['tid-2']));
    expect(rows()).toEqual([['tid-2', 'running', 10, null], ['tid-1', 'ended', 10, 30]]);
    // The list at hand is the first one, and it says what ended before it.
    apply(list(40, 'r2', ['tid-2'], [['tid-9', 'completed']]));
    expect(rows()).toEqual([['tid-2', 'running', 10, null], ['tid-1', 'ended', 10, 30], ['tid-9', 'completed', null, 40]]);
  });

  it('work of an earlier run does not show as running: its process is gone', () => {
    apply(list(5, 'r1', ['tid-9']));
    expect(ofLiveRun(live().background, bo.run_id)).toEqual([]);
    apply(list(10, 'r2', ['tid-1']));
    expect(live().background.map((w) => w.id)).toEqual(['tid-1']);
  });

  it('keeps everything that runs and only the last 8 that ended', () => {
    apply(list(1, 'r2', ['long']));
    for (let i = 0; i < 12; i++) apply(list(10 + i, 'r2', ['long', `t${i}`]), list(20 + i, 'r2', ['long'], [[`t${i}`, 'completed']]));
    const kept = live().background;
    expect(kept.filter((w) => w.status === 'running').map((w) => [w.id, w.startedAt])).toEqual([['long', 1]]);
    expect(kept.filter((w) => w.status !== 'running').map((w) => w.id)).toEqual(['t4', 't5', 't6', 't7', 't8', 't9', 't10', 't11']);
  });

  it('has one row per id, also when a list names one twice', () => {
    apply(list(10, 'r2', ['tid-1', 'tid-1']));
    expect(rows()).toEqual([['tid-1', 'running', 10, null]]);
    apply(list(20, 'r2', [], [['tid-1', 'stopped']]), list(30, 'r2', [], [['tid-1', 'stopped']]));
    expect(rows()).toEqual([['tid-1', 'stopped', null, 30]]);
  });

  it('"Stop" names the work by Claude Code\'s id, and a refusal says why', async () => {
    for (const t of toasts.list) toasts.dismiss(t.id);
    const stop = vi.spyOn(api, 'stopAgentTask').mockResolvedValueOnce(undefined).mockRejectedValueOnce('Bo is not running').mockResolvedValueOnce(undefined).mockRejectedValueOnce('Bo is not running');
    expect(await stopBackgroundTask('a1', { id: 'tid-2', kind: 'command' })).toBe(true);
    expect(stop).toHaveBeenCalledWith('a1', 'tid-2');
    expect(toasts.list).toHaveLength(0);
    expect(await stopBackgroundTask('a1', { id: 'tid-2', kind: 'command' })).toBe(false);
    expect(toasts.list.map((t) => [t.title, t.body])).toEqual([['Couldn’t stop the command', 'Bo is not running']]);
    for (const t of toasts.list) toasts.dismiss(t.id);
    // A helper goes the same way, by its id of the list.
    expect(await stopBackgroundTask('a1', { id: 'tid-3', kind: 'helper' })).toBe(true);
    expect(stop).toHaveBeenLastCalledWith('a1', 'tid-3');
    expect(await stopBackgroundTask('a1', { id: 'tid-3', kind: 'helper' })).toBe(false);
    expect(toasts.list.map((t) => [t.title, t.body])).toEqual([['Couldn’t stop the helper', 'Bo is not running']]);
    for (const t of toasts.list) toasts.dismiss(t.id);
  });
});

// The event of a list has no start time: work starts with the list that names it first. After
// the window was loaded again, the first list at hand of a run can be a late one of a long run.
// Before, its work got that list's time, and the row's timer was too short.
describe('when background work started', () => {
  const said = (n: number): EventRow => ({ id: n, ts: 1000 + n, agent_id: 'a1', task_id: null, run_id: 'r2', kind: { type: 'message', item_id: `m${n}`, text: 'on it', phase: 'final' } as EventKind });
  const starts = () => live().background.map((w) => [w.id, w.status, w.startedAt]);

  it('is the time of the list that names it first, when the window saw that list come', () => {
    apply(list(10, 'r2', ['tid-1']), list(20, 'r2', ['tid-1', 'tid-2']));
    expect(starts()).toEqual([['tid-1', 'running', 10], ['tid-2', 'running', 20]]);
  });

  it('is not known for work that the first list of a snapshot names, in its events or beside them', () => {
    fresh({ as_of: 900, events: [{ ...list(500, 'r2', ['tid-1']), id: 890 }, said(899), said(900)] });
    expect(starts()).toEqual([['tid-1', 'running', null]]);
    fresh({ as_of: 900, events: [said(899), said(900)], session_events: [{ ...list(500, 'r2', ['tid-1']), id: 40 }] });
    expect(starts()).toEqual([['tid-1', 'running', null]]);
    // It ends: how long it ran is not known either.
    apply({ ...list(20_000, 'r2', [], [['tid-1', 'completed']]), id: 901 });
    expect(live().background.map((w) => [w.id, w.status, w.startedAt, w.endedAt])).toEqual([['tid-1', 'completed', null, 20_000]]);
  });

  it('is known for work that a later list of the snapshot adds, and for work that comes after the snapshot', () => {
    fresh({ as_of: 900, events: [{ ...list(500, 'r2', ['tid-1']), id: 890 }, { ...list(600, 'r2', ['tid-1', 'tid-2']), id: 895 }, said(900)] });
    expect(starts()).toEqual([['tid-1', 'running', null], ['tid-2', 'running', 600]]);
    apply({ ...list(20_000, 'r2', ['tid-1', 'tid-2', 'tid-3']), id: 901 });
    expect(starts()).toEqual([['tid-1', 'running', null], ['tid-2', 'running', 600], ['tid-3', 'running', 20_000]]);
  });

  it('a start the window saw stays known when the agent\'s earlier history is loaded and its state is built again', async () => {
    fresh({ as_of: 900, events: [said(899), said(900)] });
    apply({ ...list(20_000, 'r2', ['tid-1']), id: 901 });
    expect(starts()).toEqual([['tid-1', 'running', 20_000]]);
    vi.spyOn(api, 'threadEvents').mockResolvedValue([said(898), said(897)]);
    await store.loadEarlier('agent:a1');
    expect(live().messages).toHaveLength(4);
    expect(starts()).toEqual([['tid-1', 'running', 20_000]]);
  });

  // The Claude adapter gives each task the time of the first list that named it. Before, a row
  // read from the first list of a snapshot showed "running" with no timer for the rest of its
  // life (after you switched to another office and back).
  it('is the start the list carries, also in the first list of a snapshot; work without one is as before', () => {
    const carried = (ts: number, started: Record<string, number>, ids: string[]): EventRow =>
      ev(ts, 'r2', { type: 'background_tasks', tasks: ids.map((id) => ({ ...task(id), ...(started[id] != null ? { started_at: started[id] } : {}) })), ended: [] });
    fresh({ as_of: 900, events: [{ ...carried(500, { 'tid-1': 120 }, ['tid-1', 'tid-2']), id: 890 }, said(899), said(900)] });
    expect(starts()).toEqual([['tid-1', 'running', 120], ['tid-2', 'running', null]]);
    // A later list keeps it; one it ends with keeps it too.
    apply({ ...carried(20_000, { 'tid-1': 120 }, ['tid-1']), id: 901 });
    expect(starts()).toEqual([['tid-1', 'running', 120], ['tid-2', 'ended', null]]);
    apply({ ...list(30_000, 'r2', [], [['tid-1', 'stopped']]), id: 902 });
    expect(live().background.map((w) => [w.id, w.status, w.startedAt, w.endedAt])).toContainEqual(['tid-1', 'stopped', 120, 30_000]);
  });
});
