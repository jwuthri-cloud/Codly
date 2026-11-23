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
import { MCP_HINT, MCP_LABEL, STOP_WAIT_MS, backgroundKind, backgroundSummary, canReconnect, canStop, mcpSummary, mcpSwitch, mcpSwitchHint, ofLiveRun, reconnectAnswer, reconnectMcp, signInSteps, stopBackgroundTask, stopHint, stopOnItsWay, switchMcp } from '../state/session';
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
    // No recording says whether Claude Code saves the switch: the hint promises nothing
    // about the agent's next run.
    expect(mcpSwitchHint('Bo')).toBe('Switches this server for Bo. Claude Code may keep the switch after this run.');
    expect(MCP_HINT.disabled).toBe('This server is switched off: the agent works without its tools. Turn on starts it again.');
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
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['error', 'Couldn’t turn docs on', 'Claude Code answered: Connection closed']]);
    for (const t of toasts.list) toasts.dismiss(t.id);
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
    expect(backgroundSummary(live().background)).toBe('2 ended');
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

  it('the "Stop" of a helper says what follows: its agent is told and takes a turn', () => {
    apply(ev(10, 'r2', { type: 'background_tasks', tasks: [task('tid-1'), task('tid-3', 'helper')], ended: [] }));
    const [command, helper] = live().background;
    expect(stopHint(helper, 'Bo')).toBe('Stops this helper. Bo is told and goes on with a turn of its own.');
    expect(stopHint(command, 'Bo')).toBeNull();
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
});
