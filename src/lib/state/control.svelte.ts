// Per-agent live controls (agent panel): block tools, stop after this step, steer note, compact.
// The core owns the state machine (orchestrator/controls.rs); this mirrors its latest
// `control_changed` event per agent (store.svelte.ts feeds `set`) and sends the intents.
// Kill is the existing `stopAgent` action. A Claude agent also has its mode and a context
// reading: requests to its live session (the mode is on the agent itself). "Compact now" is one
// control for a Codex agent and a 1:1 Claude agent.
import { api } from '../api';
import type { Agent } from '../gen/Agent';
import type { ContextUsage } from '../gen/ContextUsage';
import type { ControlState } from '../gen/ControlState';
import type { EventRow } from '../gen/EventRow';
import type { PermissionMode } from '../gen/PermissionMode';
import type { Provider } from '../gen/Provider';
import { clock, tokens } from '../util/format';
import { toast } from './toasts.svelte';

export const NO_CONTROL: ControlState = { block_tools: false, stop_requested: false, paused: false, held: 0 };

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

class Controls {
  byAgent = $state.raw<Record<string, ControlState>>({});

  of(agentId: string): ControlState {
    return this.byAgent[agentId] ?? NO_CONTROL;
  }

  set(agentId: string, c: ControlState) {
    const { [agentId]: _old, ...rest } = this.byAgent;
    this.byAgent = c.block_tools || c.stop_requested || c.paused || c.held ? { ...rest, [agentId]: c } : rest;
  }
}

export const controls = new Controls();

async function run(what: string, agentId: string, fn: () => Promise<ControlState>): Promise<boolean> {
  try {
    controls.set(agentId, await fn()); // the event says the same; this is for the instant feedback
    return true;
  } catch (e) {
    toast({ kind: 'error', title: `Couldn’t ${what}`, body: msg(e) });
    return false;
  }
}

export const setBlockTools = (agentId: string, on: boolean) => run(on ? 'block its tools' : 'allow its tools', agentId, () => api.setAgentBlockTools(agentId, on));
export const stopAfterStep = (agentId: string) => run('stop it after this step', agentId, () => api.stopAgentAfterStep(agentId));
export const resumeAgent = (agentId: string) => run('resume it', agentId, () => api.resumeAgent(agentId));
export const steerAgent = (agentId: string, text: string) => run('send the steer note', agentId, () => api.steerAgent(agentId, text));

/**
 * How a 1:1 Claude agent works with edits. The hints say what Codly no longer sees in a mode.
 * In "accept edits" Claude Code runs edits, and shell commands that change, move or delete
 * files inside the worktree, without a request (recorded with mv, rm and mkdir), so no
 * permission of yours applies to them (the core refuses the mode under "Ask first"); an edit
 * of the folder's own .claude settings still comes as a request (recorded). In "plan first"
 * the core approves only reads until you accept the plan.
 */
export const PERMISSION_MODES: { id: PermissionMode; label: string; hint: string }[] = [
  { id: 'ask', label: 'Ask for edits', hint: 'Every edit follows your permissions first, like a command.' },
  {
    id: 'accept_edits',
    label: 'Accept edits',
    hint: 'Edits, and shell commands that change, move or delete files in its worktree, run without asking you: your permissions do not see them. A change to its own Claude Code settings there (.claude) still asks you, and other commands still follow your permissions. Not available with “Ask first” permissions.',
  },
  {
    id: 'plan',
    label: 'Plan first',
    hint: 'It reads and plans first. Until you accept its plan, Codly approves only reads for it, whatever your permissions. The plan comes to you as a card: accept it, change it, or send it back.',
  },
];

/** Returns whether the agent switched. The agent's own `agent_changed` event carries the new mode. */
export async function setPermissionMode(agentId: string, mode: PermissionMode): Promise<boolean> {
  try {
    await api.setAgentPermissionMode(agentId, mode);
    return true;
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t change its mode', body: msg(e) });
    return false;
  }
}

/**
 * What goes back with "accept" when you changed the plan on its card: your text, which Claude
 * Code takes as the plan (recorded). Null when there is nothing to send: the plan as it was.
 */
export function changedPlan(plan: string, edited: string): { plan: string } | null {
  return edited.trim() && edited !== plan ? { plan: edited } : null;
}

/** How full a Claude agent's context is right now; null when it could not be read. */
export async function readContext(agentId: string): Promise<ContextUsage | null> {
  try {
    return await api.agentContextUsage(agentId);
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t read its context', body: msg(e) });
    return null;
  }
}

/** "6k of 1M tokens (1%) · compacts by itself at 967k" */
export function contextLine(u: ContextUsage): string {
  const line = `${tokens(u.used_tokens)} of ${tokens(u.max_tokens)} tokens (${Math.round(u.percent)}%)`;
  return u.auto_compact_at == null ? line : `${line} · compacts by itself at ${tokens(u.auto_compact_at)}`;
}

/** Who has a "Compact now": a Codex agent, and a 1:1 Claude agent. Claude Code compacts as a
 *  turn of its own, and for a Claude worker, a reviewer or the Lead the end of a turn means
 *  its work is ready to be checked: the core refuses those. */
export const canCompact = (agent: Pick<Agent, 'provider' | 'role'>): boolean => agent.provider === 'codex' || (agent.provider === 'claude' && agent.role === 'solo');

/** What "Compact now" does, for the row's hint. */
export const compactHint = (provider: Provider): string =>
  `${provider === 'codex' ? 'Codex sums up the conversation so far' : 'Claude replaces the conversation so far with a summary'} to free context. Use it between turns of a long-lived agent.`;

const IN_TURN: Agent['status'][] = ['starting', 'thinking', 'working', 'waiting_approval'];

/** Why "Compact now" has to wait, or null. The core has the last word (a turn may start
 *  meanwhile); this only keeps the button honest. */
export function compactWait(agent: Pick<Agent, 'task_id' | 'status'>): string | null {
  if (agent.task_id) return 'It is on a task. Compact it between tasks.';
  return IN_TURN.includes(agent.status) ? 'Wait until its turn ends.' : null;
}

/** Ask an agent to compact its conversation now (the core picks the way of its CLI). The toast
 *  says that the CLI took the request; the compaction itself shows in the agent's history. */
export async function compactAgent(agentId: string, name: string): Promise<boolean> {
  try {
    await api.compactAgent(agentId);
    toast({ kind: 'success', title: `Compacting ${name}’s conversation`, body: 'Its history shows it when it is done.' });
    return true;
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t compact its conversation', body: msg(e) });
    return false;
  }
}

/**
 * The sign of a compaction in the row of "Compact now": what the agent's live run last said
 * about one (`events`: the log, oldest first). Claude Code says from how many tokens to how
 * many the conversation went when a compaction is done. An event without those numbers can
 * be a start or an end: Claude Code sends one when it begins, Codex one when it begins and one
 * when it is done, and nothing tells them apart. So that line gives the time and calls it
 * neither started nor done. Null: the live run said nothing of the kind.
 */
export function compactionLine(events: EventRow[], agent: Pick<Agent, 'id' | 'run_id'>): string | null {
  if (!agent.run_id) return null;
  for (let i = events.length - 1; i >= 0; i--) {
    const e = events[i];
    if (e.kind.type !== 'compacting' || e.agent_id !== agent.id || e.run_id !== agent.run_id) continue;
    const { pre_tokens: pre, post_tokens: post } = e.kind;
    if (post == null) return `Last compaction: ${clock(e.ts)}`;
    return `Compacted at ${clock(e.ts)}${pre == null ? '' : ` · ${tokens(pre)} → ${tokens(post)} tokens`}`;
  }
  return null;
}

/** `/context` in the composer: read how full the agent's context is and say it in a toast. A
 *  read that fails says why (`readContext`). */
export async function showContext(agentId: string, name: string): Promise<void> {
  const u = await readContext(agentId);
  if (u) toast({ kind: 'info', title: `${name}’s context`, body: contextLine(u) });
}
