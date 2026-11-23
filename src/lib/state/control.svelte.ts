// Per-agent live controls (agent panel): block tools, stop after this step, steer note, compact.
// The core owns the state machine (orchestrator/controls.rs); this mirrors its latest
// `control_changed` event per agent (store.svelte.ts feeds `set`) and sends the intents.
// Kill is the existing `stopAgent` action. A Claude agent also has its mode, "compact now"
// and a context reading: requests to its live session (the mode is on the agent itself).
import { api } from '../api';
import type { ContextUsage } from '../gen/ContextUsage';
import type { ControlState } from '../gen/ControlState';
import type { PermissionMode } from '../gen/PermissionMode';
import { tokens } from '../util/format';
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
 * How a 1:1 Claude agent works with edits. The hints say what Codly no longer sees in a mode:
 * in "accept edits" Claude Code runs edits and file-writing shell commands inside the worktree
 * without a request, so no permission of yours applies to them (the core refuses the mode under
 * "Ask first"); in "plan first" the core approves only reads until you accept the plan.
 */
export const PERMISSION_MODES: { id: PermissionMode; label: string; hint: string }[] = [
  { id: 'ask', label: 'Ask for edits', hint: 'Every edit follows your permissions first, like a command.' },
  {
    id: 'accept_edits',
    label: 'Accept edits',
    hint: 'Edits, and shell commands that change files in its worktree, run without asking you: your permissions do not see them. Other commands still follow your permissions. Not available with “Ask first” permissions.',
  },
  {
    id: 'plan',
    label: 'Plan first',
    hint: 'It reads and plans first. Until you accept its plan, Codly approves only reads for it, whatever your permissions. The plan comes to you as a card: accept it or send it back.',
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

/** Ask a 1:1 Claude agent to summarize its conversation now, to free context. */
export async function compactNow(agentId: string): Promise<boolean> {
  try {
    await api.compactAgent(agentId);
    return true;
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t compact its conversation', body: msg(e) });
    return false;
  }
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

/** Ask Codex to compact the agent's conversation now. The toast says whether Codex took the
 *  request; the compaction itself shows in the agent's history. */
export async function compactAgent(agentId: string, name: string): Promise<boolean> {
  try {
    await api.compactAgent(agentId);
    toast({ kind: 'success', title: `Codex is compacting ${name}’s conversation`, body: 'Its context meter updates when Codex is done.' });
    return true;
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t compact the conversation', body: msg(e) });
    return false;
  }
}
