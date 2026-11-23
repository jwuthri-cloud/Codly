// What runs beside a live agent's own calls, as its CLI reports it: its MCP servers
// (`mcp_server` events) and the commands it left running in the background
// (`background_command` events). Both belong to one run of the agent: the process that holds
// the servers and runs the commands. The store keeps them per agent (store.svelte.ts feeds
// these functions); the agent sheet shows the ones of the agent's live run, with "Reconnect"
// and "Stop" (requests to the live Claude session).
import { api } from '../api';
import type { EventKind } from '../gen/EventKind';
import type { McpServerStatus } from '../gen/McpServerStatus';
import { plural } from '../util/format';
import { toast } from './toasts.svelte';

/** One MCP server of an agent's run, as the CLI last gave it. `error` is the CLI's own reason. */
export type McpServer = { name: string; status: McpServerStatus; error: string | null; runId: string | null; ts: number };

/**
 * A command the agent left running in the background. `taskId` is the CLI's id of it (what
 * "Stop" names), `callId` the tool call that started it. `status` is "running", then the
 * CLI's word when it is over ("completed", "stopped").
 */
export type BackgroundCommand = { taskId: string; callId: string; description: string; status: string; runId: string | null; startedAt: number; ts: number };

type McpEvent = Extract<EventKind, { type: 'mcp_server' }>;
type CommandEvent = Extract<EventKind, { type: 'background_command' }>;

/** Ended commands kept per agent, beside the ones that still run. */
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

/** The commands after one more `background_command` event: the ones of this run, the oldest ended ones dropped. */
export function withBackgroundCommand(list: BackgroundCommand[], k: CommandEvent, runId: string | null, ts: number): BackgroundCommand[] {
  const same = list.filter((c) => c.runId === runId);
  const prev = same.find((c) => c.taskId === k.task_id);
  const next: BackgroundCommand = { taskId: k.task_id, callId: k.call_id, description: k.description, status: k.status, runId, startedAt: prev?.startedAt ?? ts, ts };
  const all = prev ? same.map((c) => (c === prev ? next : c)) : [...same, next];
  const ended = all.filter((c) => c.status !== 'running');
  const drop = new Set(ended.slice(0, Math.max(0, ended.length - ENDED_KEPT)));
  return all.filter((c) => !drop.has(c));
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
  needs_sign_in: 'The agent works without this server’s tools until you sign in to it in Claude Code. Then reconnect.',
  disabled: 'This server is switched off in Claude Code.',
};

/** A server Codly can ask Claude Code to connect again. */
export const canReconnect = (s: McpServer) => s.status === 'failed' || s.status === 'needs_sign_in';

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

/** The section's line when it is closed: "2 running", else "3 ended". */
export function backgroundSummary(list: BackgroundCommand[]): string {
  const running = list.filter((c) => c.status === 'running').length;
  return running ? `${running} running` : `${plural(list.length, 'command')} ended`;
}

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

/** Stop a background command. Returns whether Claude Code took the request; the command's event says that it stopped. */
export async function stopBackgroundCommand(agentId: string, taskId: string): Promise<boolean> {
  try {
    await api.stopAgentTask(agentId, taskId);
    return true;
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t stop the command', body: msg(e) });
    return false;
  }
}
