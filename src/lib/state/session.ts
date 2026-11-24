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
import { CODLY_SERVER } from '../util/toolWords';
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
  disabled: 'This server is switched off for Claude Code in this folder: the agent works without its tools. Turn on starts it again.',
};

/** How to sign in to a server that needs it, under its row. Codly cannot do it for you: the
 *  sign-in is Claude Code's own (no recording in docs/research names the command for it, so
 *  the line names the place, not a command). */
export const signInSteps = (name: string): string =>
  `Open Claude Code in a terminal and sign in to ${name} in Claude Code’s own MCP settings. Then press Reconnect here. If this server uses an API key, check the key in its settings instead.`;

/** A server Codly can ask Claude Code to connect again. */
export const canReconnect = (s: McpServer) => s.status === 'failed' || s.status === 'needs_sign_in';

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

/**
 * Whether the agent sheet shows its MCP section: when the live run reported a server, and also
 * when it reported none but started without the folder's own configuration. That line is the
 * only place in the sheet that says so, and such a run can have no server to list (a CLI other
 * than Claude Code reports none).
 */
export const showsMcpSection = (servers: readonly McpServer[], withoutFolderConfig: boolean): boolean => servers.length > 0 || withoutFolderConfig;

/** The switch of a server's row: its button, the button while the request is on its way, and what the request turns the server to. */
export type McpSwitch = { label: string; busy: string; enabled: boolean };

/**
 * The switch a server's row offers: "Turn off" on one that is connected, "Turn on" on one
 * that is switched off, none in any other state (a server that did not start has Reconnect)
 * and none on Codly's own server.
 */
export function mcpSwitch(s: McpServer): McpSwitch | null {
  if (s.name === CODLY_SERVER) return null;
  if (s.status === 'connected') return { label: 'Turn off', busy: MCP_BUSY.off, enabled: false };
  if (s.status === 'disabled') return { label: 'Turn on', busy: MCP_BUSY.on, enabled: true };
  return null;
}

/**
 * What "Turn off" does, on its button. Recorded (docs/research/cli-claude-stream-json-2.1.311-part5.md,
 * section 1, `r7-mcp-toggle-saved-*`, n=2): Claude Code saves the switch for the folder, and
 * every later Claude Code process there reports the server switched off and does not start it,
 * until "Turn on". That your own Claude Code sessions in the folder follow the same switch is
 * what part 5 expects, not what it recorded: the line does not claim it.
 */
export const mcpSwitchHint = (agent: string): string =>
  `Turns this server off for Claude Code in this folder: for ${agent} now and for every later run here, until you turn it on.`;

/** The accessible name of a server's button: the words it shows now ("Turn off", "Turning off…" while its request is on its way) and the server's name. */
export const mcpButtonLabel = (shown: string, server: string): string => `${shown} ${clean(server)}`;

/**
 * Turn an MCP server of the agent's live run off or on. Returns whether Claude Code took
 * the request; the server's next `mcp_server` event says where it stands. A refusal (Codly's
 * own server, a server that does not start again) is said in a toast, with the reason. Only
 * an answer of Claude Code to a "Turn on" (`cliAnswer`) saved the switch on.
 */
export async function switchMcp(agentId: string, server: string, enabled: boolean): Promise<boolean> {
  try {
    await api.setAgentMcpEnabled(agentId, server, enabled);
    return true;
  } catch (e) {
    const said = enabled ? cliAnswer(msg(e)) : null;
    if (said != null) toast({ kind: 'error', title: mcpStartFailure(server, said, true) });
    else toast({ kind: 'error', title: `Couldn’t turn ${clean(server)} ${enabled ? 'on' : 'off'}`, body: msg(e) });
    return false;
  }
}

/** What the core puts before an error that Claude Code itself answered (orchestrator/controls.rs `session_answer_within`). */
const CLI_ANSWERED = 'Claude Code answered: ';

/**
 * Claude Code's own words in an error of the core, or null for an error of Codly's own (the
 * agent is not running, is still starting, or Claude Code did not answer): then nothing
 * reached Claude Code, and nothing was switched or started.
 */
export const cliAnswer = (error: string): string | null => (error.startsWith(CLI_ANSWERED) ? error.slice(CLI_ANSWERED.length) : null);

/**
 * Why a server you turned on or restarted did not start, from Claude Code's answer. Recorded
 * (docs/research/cli-claude-stream-json-2.1.311-part5.md, section 9, `r7-mcp-cannot-start-*`,
 * n=2): the answer is an error in Claude Code's words, "Connection closed" for a program that
 * exits at once or whose file is gone, and `Executable not found in $PATH: "stdio"` for a
 * program that does not exist. Those words name the transport ("stdio"), not the program, so
 * they are never shown as the name of what is missing. A "Turn on" that failed still counts as
 * on for the saved switch (`turningOn`): the line says so.
 */
export function mcpStartFailure(server: string, words: string, turningOn: boolean): string {
  const why = /Executable not found in \$PATH/.test(words)
    ? 'its program was not found on your PATH'
    : /\bConnection closed\b/i.test(words)
      ? 'the connection closed'
      : clean(words).trim().replace(/[.\s]+$/, '');
  return `${clean(server)} did not start: ${why}.${turningOn ? ' It stays switched on: Reconnect tries again.' : ''}`;
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

/**
 * What "Stop" does to a helper, on its button (`agent`: the name of the helper's agent;
 * `turnRuns`: a turn of the agent runs now). A command just ends: no hint. With no turn running,
 * Claude Code tells the agent in a turn of its own (part3.md, section 1). While a turn runs, the
 * agent is told within that turn and no turn follows (part5.md, section 11,
 * `r7-stop-helper-mid-turn-*`, n=2: the stop came during a tool call of the turn). Not
 * recorded: a stop during the turn's last model request.
 */
export const stopHint = (w: BackgroundWork, agent: string, turnRuns: boolean): string | null =>
  w.kind !== 'helper' ? null : turnRuns ? `Stops this helper. ${agent} is told in the turn that runs.` : `Stops this helper. ${agent} is told and goes on with a turn of its own.`;

/**
 * What "Stop" does to a helper its agent still waits for (not in the background list): its
 * call returns to the agent, and the turn goes on (recorded:
 * docs/research/cli-claude-stream-json-2.1.311-part4.md, section 2).
 */
export const waitedHelperStopHint = (agent: string): string => `Stops this helper. ${agent}’s turn goes on.`;

/** What a "Stop" button reads while its stop is on its way. */
export const STOP_BUSY = 'Stopping…';

/**
 * The accessible name of a "Stop" button: the words it shows and what it stops ("Stop the helper
 * Run the tests"; `busy`, while the stop is on its way: "Stopping… the command npm run dev").
 */
export const stopLabel = (w: Pick<BackgroundWork, 'kind' | 'description'>, busy = false): string =>
  `${busy ? STOP_BUSY : 'Stop'} the ${w.kind === 'helper' ? 'helper' : w.kind === 'command' ? 'command' : 'task'} ${clean(w.description)}`;

/**
 * How long the buttons of a list stay off after a row you acted on changed: the row moves or
 * shows another button, and the press after yours must not land on what took its place.
 */
export const HOLD_MS = 1500;

/** How long a row you stopped stays in its place among the running ones, with how it ended, before the list closes up. */
export const KEEP_STOPPED_MS = 2000;

/**
 * The order of the rows of the running work: the ids it had (`order`) that are still in the
 * list, then the work that started since, in the CLI's order. A row keeps its place while it shows.
 */
export function rowOrder(order: readonly string[], work: readonly BackgroundWork[]): string[] {
  const known = new Set(work.map((w) => w.id));
  const kept = order.filter((id) => known.has(id));
  const seen = new Set(kept);
  return [...kept, ...work.filter((w) => w.status === 'running' && !seen.has(w.id)).map((w) => w.id)];
}

/**
 * The rows of "In the background": `running`, in their order (`rowOrder`), with a row you
 * stopped (`stopped`, by id) that ended less than `KEEP_STOPPED_MS` ago (`now`: the clock of
 * the screen) still in its place, with how it ended; `ended`: the other work that is over.
 * Before, a stopped row left the running ones at once, and the next row's "Stop" moved under
 * the pointer.
 */
export function backgroundRows(work: readonly BackgroundWork[], order: readonly string[], stopped: ReadonlySet<string>, now: number): { running: BackgroundWork[]; ended: BackgroundWork[] } {
  const stays = (w: BackgroundWork) => w.status === 'running' || (stopped.has(w.id) && w.endedAt != null && now - w.endedAt < KEEP_STOPPED_MS);
  const byId = new Map(work.map((w) => [w.id, w]));
  const running = rowOrder(order, work).flatMap((id) => {
    const w = byId.get(id);
    return w && stays(w) ? [w] : [];
  });
  const shown = new Set(running.map((w) => w.id));
  return { running, ended: work.filter((w) => !shown.has(w.id)) };
}

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
 * whether it did; an answer that it did not is said in a toast, with Claude Code's reason, and
 * a refusal of Codly's own (nothing reached Claude Code) with the refusal.
 */
export async function restartMcp(agentId: string, server: string): Promise<boolean> {
  const failed = await reconnectMcp(agentId, server);
  if (failed == null) return true;
  const said = cliAnswer(failed);
  if (said != null) toast({ kind: 'error', title: mcpStartFailure(server, said, false) });
  else toast({ kind: 'error', title: `Couldn’t restart ${clean(server)}`, body: failed });
  return false;
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
