// What runs beside a live agent's own calls, as its CLI reports it: its MCP servers
// (`mcp_server` events) and what it left at work in the background: commands and helpers
// (`background_tasks` events, the CLI's whole list at every change, with what left the list).
// Both belong to one run of the agent: the process that holds the servers and runs the
// commands. The store keeps them per agent (store.svelte.ts feeds these functions); the agent
// sheet shows the ones of the agent's live run, with "Reconnect", "Turn off", "Turn on" and
// "Stop" (requests to the live Claude session).
import { api } from '../api';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { McpServerStatus } from '../gen/McpServerStatus';
import { clean } from '../util/sanitize';
import { toast } from './toasts.svelte';

/** One MCP server of an agent's run, as the CLI last gave it. `error` is the CLI's own reason. */
export type McpServer = { name: string; status: McpServerStatus; error: string | null; runId: string | null; ts: number };

/**
 * One piece of work of an agent's run in the background: a command it left running ("command":
 * "Stop" names it by `id`, the CLI's id of it), a helper ("helper"), or another task of the CLI.
 * `status` is "running", then the CLI's word for how it ended ("completed", "stopped"; "ended"
 * when the CLI did not say). `startedAt`: when the list in which it started came (null: that
 * list is not known: the work was never seen running, or the first list at hand that names it
 * was read from a snapshot and may not be the first one that did). `endedAt`: when the list
 * that no longer names it came.
 */
export type BackgroundWork = { id: string; kind: string; description: string; status: string; runId: string | null; startedAt: number | null; endedAt: number | null };

type McpEvent = Extract<EventKind, { type: 'mcp_server' }>;
type TasksEvent = Extract<EventKind, { type: 'background_tasks' }>;

/** Ended work kept per agent, beside what still runs. */
const ENDED_KEPT = 8;

/**
 * The servers after one more `mcp_server` event. A server keeps its place in the list. The
 * servers of an earlier run are gone with that run's process: the first event of a new run
 * drops them.
 */
export function withMcpServer(list: McpServer[], k: McpEvent, runId: string | null, ts: number): McpServer[] {
  const next: McpServer = { name: k.name, status: k.status, error: k.error, runId, ts };
  const same = list.filter((s) => s.runId === runId);
  return same.some((s) => s.name === k.name) ? same.map((s) => (s.name === k.name ? next : s)) : [...same, next];
}

/**
 * The background work after one more `background_tasks` event: what the CLI lists now runs
 * (each keeps the start time it had), and what left the list is over, with the CLI's word for
 * its end. The list is the whole list each time, so work that ran and is in neither part of
 * the event is over too (the event of its end is not at hand: "ended"). The work of an earlier
 * run is gone with that run's process. Of the ended work the last ones are kept.
 *
 * A task's start is the `started_at` its adapter gives it (Claude Code's: when the first list
 * that named it came). Without one (another CLI, a row written before it was kept), work starts
 * with the list that names it first. `replayed`: the list was read from a snapshot or from
 * stored rows (the store says so), not as it came. Such a list with nothing of its run before
 * it (the window was loaded again) can be a late list of a long run: the work it names with no
 * `started_at` started at a time nobody knows, and its row shows no timer.
 */
export function withBackgroundTasks(list: BackgroundWork[], k: TasksEvent, runId: string | null, ts: number, replayed = false): BackgroundWork[] {
  const same = list.filter((w) => w.runId === runId);
  const ran = new Map(same.filter((w) => w.status === 'running').map((w) => [w.id, w]));
  const startsHere = !replayed || same.length > 0;
  // One row per id, in each part: the rows are keyed by it.
  const running = new Map<string, BackgroundWork>();
  for (const t of k.tasks) {
    const was = ran.get(t.id);
    const startedAt = t.started_at ?? (was ? was.startedAt : startsHere ? ts : null);
    running.set(t.id, { id: t.id, kind: t.kind, description: t.description, status: 'running', runId, startedAt, endedAt: null });
  }
  const said = new Set([...k.tasks, ...k.ended].map((t) => t.id));
  const lost = [...ran.values()].filter((w) => !said.has(w.id)).map((w): BackgroundWork => ({ ...w, status: 'ended', endedAt: ts }));
  const ended = k.ended.map((e): BackgroundWork => ({ id: e.id, kind: e.kind, description: e.description, status: e.status, runId, startedAt: ran.get(e.id)?.startedAt ?? null, endedAt: ts }));
  const over = new Map<string, BackgroundWork>();
  for (const w of [...same.filter((w) => w.status !== 'running'), ...lost, ...ended]) {
    over.delete(w.id); // (an end that is said again moves to the end of the list, with its last word)
    over.set(w.id, w);
  }
  return [...running.values(), ...[...over.values()].slice(-ENDED_KEPT)];
}

/**
 * Background work that was built again from an agent's stored rows (`built`), with the start
 * times the window knew before (`held`): a start it saw is not in doubt because the rows were
 * read again.
 */
export function withKnownStarts(built: BackgroundWork[], held: BackgroundWork[]): BackgroundWork[] {
  const key = (w: BackgroundWork) => `${w.runId ?? ''} ${w.id}`;
  const known = new Map(held.map((w) => [key(w), w.startedAt]));
  return built.map((w) => (w.startedAt != null ? w : { ...w, startedAt: known.get(key(w)) ?? null }));
}

/** What belongs to the agent's live run (nothing when it has none: its process is gone). */
export function ofLiveRun<T extends { runId: string | null }>(list: T[], runId: string | null): T[] {
  return runId == null ? [] : list.filter((x) => x.runId === runId);
}

export const MCP_LABEL: Record<McpServerStatus, string> = {
  connected: 'Connected',
  connecting: 'Connecting…',
  failed: 'Did not start',
  needs_sign_in: 'Needs sign-in',
  disabled: 'Switched off',
};

/** What the status means for the agent, and what you can do about it. */
export const MCP_HINT: Record<McpServerStatus, string> = {
  connected: 'The agent can use this server’s tools.',
  connecting: 'Claude Code is still connecting to this server.',
  failed: 'The agent works without this server’s tools. Reconnect tries again.',
  needs_sign_in: 'The agent works without this server’s tools until you sign in to it in Claude Code’s own MCP settings. Then press Reconnect.',
  disabled: 'This server is switched off: the agent works without its tools. Turn on starts it again.',
};

/** How to sign in to a server that needs it, under its row. Codly cannot do it for you: the
 *  sign-in is Claude Code's own (no recording in docs/research names the command for it, so
 *  the line names the place, not a command). */
export const signInSteps = (name: string): string =>
  `Open Claude Code in a terminal and sign in to ${name} in Claude Code’s own MCP settings. Then press Reconnect here. If this server uses an API key, check the key in its settings instead.`;

/** A server Codly can ask Claude Code to connect again. */
export const canReconnect = (s: McpServer) => s.status === 'failed' || s.status === 'needs_sign_in';

/** Codly's own MCP server in every run: the agent reports to Codly through it, so it stays on. */
const CODLY_SERVER = 'codly';

/**
 * A connected server Codly can ask Claude Code to start again: the same request as Reconnect
 * (`mcp_reconnect`), which on a connected server answers and starts it again (recorded:
 * docs/research/cli-claude-stream-json-2.1.311-part4.md, section 7). Not Codly's own server.
 * A server whose process died still reads "connected" (Claude Code starts it again at its next
 * call), so this is how you start one again that you think is stuck.
 */
export const canRestart = (s: McpServer) => s.status === 'connected' && s.name !== CODLY_SERVER;

/** What "Restart" does, on its button (`agent`: the name of the server's agent). */
export const restartHint = (agent: string): string => `Starts this server again for this run of ${agent}.`;

/**
 * A request about one server, from the press of its button until the server's next status row:
 * what it asks, the state of the server it is about (`McpServer.ts` when you pressed), and when
 * Claude Code answered it (null: not yet). `done`: the answer itself says how it ended (a
 * restart: the server stays "connected", so no status row follows).
 */
export type McpAsk = { what: 'reconnect' | 'restart' | 'off' | 'on'; seen: number; answeredAt: number | null; done: boolean };

/** What a server's button reads while its request is on its way. */
export const MCP_BUSY: Record<McpAsk['what'], string> = { reconnect: 'Reconnecting…', restart: 'Restarting…', off: 'Turning off…', on: 'Turning on…' };

/** How long a button waits, after Claude Code answered, for the server's next status row before it can be pressed again: Claude Code may answer and report nothing new. */
export const MCP_ASK_WAIT_MS = 10_000;

/** The request just pressed: not answered yet. */
export const mcpAsked = (s: McpServer, what: McpAsk['what']): McpAsk => ({ what, seen: s.ts, answeredAt: null, done: false });

/** The request once Claude Code answered it at `now`; `done`: the answer says how it ended. */
export const mcpAnswered = (ask: McpAsk, now: number, done: boolean): McpAsk => ({ ...ask, answeredAt: now, done });

/**
 * The request still on its way for a server, if any: while Claude Code has not answered it (a
 * server that hangs at start can take 30 s and more), then until the server's next status row
 * arrives (the row then shows where it stands) or `MCP_ASK_WAIT_MS` pass after the answer. A
 * second press meanwhile would send a second request and write a second record.
 */
export const mcpAskOnItsWay = (s: McpServer, ask: McpAsk | undefined, now: number): McpAsk | null => {
  if (!ask || ask.done) return null;
  if (ask.answeredAt == null) return ask;
  return ask.seen === s.ts && now - ask.answeredAt < MCP_ASK_WAIT_MS ? ask : null;
};

/**
 * Whether a server's buttons are off: while its request is on its way, and for `HOLD_MS` after
 * its row changed because of it or after an answer that ended it. The row's buttons change with
 * its state ("Reconnect" gives way to "Restart" and "Turn off"): one press too many never lands
 * on the new button.
 */
export const mcpRowLocked = (s: McpServer, ask: McpAsk | undefined, now: number): boolean =>
  !!ask &&
  (mcpAskOnItsWay(s, ask, now) != null ||
    (ask.seen !== s.ts && now - s.ts < HOLD_MS) ||
    (ask.done && ask.answeredAt != null && now - ask.answeredAt < HOLD_MS));

/** The start of the core's notice that a run started without the folder's own CLI configuration (orchestrator/runs.rs). */
const WITHOUT_FOLDER_CONFIG = 'Started without this folder';

/**
 * Whether the agent's live run started without the folder's own CLI configuration (its MCP
 * servers among it), because you had not trusted those files: the core says so once per run,
 * in a notice. A run keeps what it started with: trusting the folder later changes the next run.
 */
export const startedWithoutFolderConfig = (events: readonly EventRow[], agentId: string, runId: string | null): boolean =>
  runId != null && events.some((e) => e.agent_id === agentId && e.run_id === runId && e.kind.type === 'notice' && e.kind.text.startsWith(WITHOUT_FOLDER_CONFIG));

/** The line of the MCP section for such a run. */
export const WITHOUT_FOLDER_CONFIG_LINE = 'This run started without the folder’s own MCP servers and settings: you had not trusted them. Settings → Safety shows them.';

/** The switch of a server's row: its button, the button while the request is on its way, and what the request turns the server to. */
export type McpSwitch = { label: string; busy: string; enabled: boolean };

/**
 * The switch a server's row offers: "Turn off" on one that is connected, "Turn on" on one
 * that is switched off, none in any other state (a server that did not start has Reconnect)
 * and none on Codly's own server.
 */
export function mcpSwitch(s: McpServer): McpSwitch | null {
  if (s.name === CODLY_SERVER) return null;
  if (s.status === 'connected') return { label: 'Turn off', busy: 'Turning off…', enabled: false };
  if (s.status === 'disabled') return { label: 'Turn on', busy: 'Turning on…', enabled: true };
  return null;
}

/**
 * What the switch is for, on its button. Not recorded: whether Claude Code saves the switch
 * (docs/research has one process that turns a server off and on, and no process started
 * after an "off"). So the line promises nothing about the agent's next run, and says that
 * the switch can stay.
 */
export const mcpSwitchHint = (agent: string): string => `Switches this server for ${agent}. Claude Code may keep the switch after this run.`;

/**
 * Turn an MCP server of the agent's live run off or on. Returns whether Claude Code took
 * the request; the server's next `mcp_server` event says where it stands. A refusal (Codly's
 * own server, a server that does not start again) is said in a toast, with the reason.
 */
export async function switchMcp(agentId: string, server: string, enabled: boolean): Promise<boolean> {
  try {
    await api.setAgentMcpEnabled(agentId, server, enabled);
    return true;
  } catch (e) {
    toast({ kind: 'error', title: `Couldn’t turn ${clean(server)} ${enabled ? 'on' : 'off'}`, body: msg(e) });
    return false;
  }
}

/**
 * What a reconnect that did not connect the server came back with: Claude Code's own answer,
 * or why it gave none. `seen` is the state of the server it is about (`McpServer.ts` when you
 * asked).
 */
export type ReconnectAnswer = { text: string; seen: number };

/**
 * The answer to show under a server, beside the server's own reason. An event of the server
 * that came since you asked says where it stands now (Codly asks Claude Code after every
 * reconnect), so the answer gives way to it. One that only repeats the reason is not said twice.
 */
export function reconnectAnswer(s: McpServer, answer: ReconnectAnswer | undefined): string | null {
  if (!answer || answer.seen !== s.ts || !canReconnect(s)) return null;
  return s.error && answer.text.includes(s.error) ? null : answer.text;
}

/** The section's line when it is closed: "2 connected · 1 needs sign-in". */
export function mcpSummary(list: McpServer[]): string {
  const n = (status: McpServerStatus) => list.filter((s) => s.status === status).length;
  const parts: [number, string][] = [
    [n('connected'), 'connected'],
    [n('connecting'), 'connecting'],
    [n('needs_sign_in'), n('needs_sign_in') === 1 ? 'needs sign-in' : 'need sign-in'],
    [n('failed'), 'did not start'],
    [n('disabled'), 'switched off'],
  ];
  return parts.filter(([count]) => count > 0).map(([count, text]) => `${count} ${text}`).join(' · ');
}

/** The section's line when it is closed: "2 running", else how the work ended, in the words of its rows: "1 completed · 1 stopped". */
export function backgroundSummary(list: BackgroundWork[]): string {
  const running = list.filter((w) => w.status === 'running').length;
  if (running) return `${running} running`;
  const ended = new Map<string, number>();
  for (const w of list) ended.set(w.status, (ended.get(w.status) ?? 0) + 1);
  return [...ended].map(([word, n]) => `${n} ${word}`).join(' · ');
}

/** What a row calls its piece of work, when it is no command: "helper", "task". */
export const backgroundKind = (w: BackgroundWork): string | null => (w.kind === 'command' ? null : w.kind === 'helper' ? 'helper' : 'task');

/**
 * "Stop" is Claude Code's `stop_task` request. It is recorded for a command and for a helper
 * of the list (docs/research/cli-claude-stream-json-2.1.311-part3.md, section 1), not for
 * another kind of task.
 */
export const canStop = (w: BackgroundWork) => w.status === 'running' && (w.kind === 'command' || w.kind === 'helper');

/** What "Stop" does to a helper, on its button (`agent`: the name of the helper's agent). A command just ends: no hint. */
export const stopHint = (w: BackgroundWork, agent: string): string | null =>
  w.kind === 'helper' ? `Stops this helper. ${agent} is told and goes on with a turn of its own.` : null;

/**
 * What "Stop" does to a helper its agent still waits for (not in the background list): its
 * call returns to the agent, and the turn goes on (recorded:
 * docs/research/cli-claude-stream-json-2.1.311-part4.md, section 2).
 */
export const waitedHelperStopHint = (agent: string): string => `Stops this helper. ${agent}’s turn goes on.`;

/** The accessible name of a "Stop" button: what it stops ("Stop the helper Run the tests", "Stop the command npm run dev"). */
export const stopLabel = (w: Pick<BackgroundWork, 'kind' | 'description'>): string =>
  `Stop the ${w.kind === 'helper' ? 'helper' : w.kind === 'command' ? 'command' : 'task'} ${clean(w.description)}`;

/**
 * How long the buttons of a list stay off after a row you acted on changed: the row moves or
 * shows another button, and the press after yours must not land on what took its place.
 */
export const HOLD_MS = 1500;

/**
 * Whether the "Stop" buttons of a list are off for a moment: one of the rows you stopped
 * (`stopped`, by id) ended less than `HOLD_MS` ago (`endedAt`, `now`: the clock of the screen),
 * and the list closed up under the pointer.
 */
export const stopsSettling = (rows: readonly { id: string; endedAt: number | null }[], stopped: ReadonlySet<string>, now: number): boolean =>
  rows.some((w) => stopped.has(w.id) && w.endedAt != null && now - w.endedAt < HOLD_MS);

/**
 * How long a row reads "Stopping…" after Claude Code took the stop. Claude Code writes the
 * list that no longer names the work before it answers (recorded: both in the same 10 ms),
 * so the row moves to the ended ones at once. A row that still runs after this wait was not
 * stopped by that request: "Stop" is there again.
 */
export const STOP_WAIT_MS = 10_000;

/** Whether a stop that Claude Code took at `takenAt` is still on its way (`now`: the clock of the screen; no time: none was taken). */
export const stopOnItsWay = (takenAt: number | undefined, now: number): boolean => takenAt != null && now - takenAt < STOP_WAIT_MS;

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

/**
 * Ask Claude Code to connect an MCP server again. Null = it connected (its `mcp_server` event
 * follows). Otherwise Claude Code's own answer, for the row to show.
 */
export async function reconnectMcp(agentId: string, server: string): Promise<string | null> {
  try {
    await api.reconnectAgentMcp(agentId, server);
    return null;
  } catch (e) {
    return msg(e);
  }
}

/**
 * Ask Claude Code to start a connected MCP server again (the request of Reconnect). Returns
 * whether it did; an answer that it did not is said in a toast, with Claude Code's reason.
 */
export async function restartMcp(agentId: string, server: string): Promise<boolean> {
  const failed = await reconnectMcp(agentId, server);
  if (failed != null) toast({ kind: 'error', title: `Couldn’t restart ${clean(server)}`, body: failed });
  return failed == null;
}

/** Stop a background command or helper. Returns whether Claude Code took the request; its next list of background tasks says that the work stopped. */
export async function stopBackgroundTask(agentId: string, work: Pick<BackgroundWork, 'id' | 'kind'>): Promise<boolean> {
  try {
    await api.stopAgentTask(agentId, work.id);
    return true;
  } catch (e) {
    toast({ kind: 'error', title: work.kind === 'helper' ? 'Couldn’t stop the helper' : 'Couldn’t stop the command', body: msg(e) });
    return false;
  }
}
