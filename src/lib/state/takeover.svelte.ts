// "Open in my terminal" (agent sheet). The core owns the hand-over (orchestrator/takeover.rs):
// who may go, the command line, and that Codly starts nothing for the agent while it is in
// your terminal. This keeps what the core said last about each agent, and sends the intents.
// The agents that are in your terminal are read from the core when an office opens
// (`loadTakeovers`, from Shell.svelte), so they are marked everywhere from the start: in the
// agent's one sentence (narration.ts) and with a tab in the Terminal room.
import { api } from '../api';
import type { Takeover } from '../gen/Takeover';
import type { TakeoverState } from '../gen/TakeoverState';
import { toast } from './toasts.svelte';
import { ui } from './ui.svelte';

const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

class Takeovers {
  /** What the core said last, for each agent that was asked about. */
  byAgent = $state.raw<Record<string, TakeoverState>>({});

  /** null: not asked yet. */
  of(agentId: string): TakeoverState | null {
    return this.byAgent[agentId] ?? null;
  }

  /** The agents that are in your terminal. */
  get active(): Takeover[] {
    return Object.values(this.byAgent).flatMap((s) => (s.takeover ? [s.takeover] : []));
  }

  /** Is the agent in your terminal? */
  has(agentId: string): boolean {
    return !!this.byAgent[agentId]?.takeover;
  }

  set(agentId: string, s: TakeoverState) {
    this.byAgent = { ...this.byAgent, [agentId]: s };
  }

  /** The core's list of every agent that is in your terminal. What was said about an agent that is not there stays. */
  setThere(list: Takeover[]) {
    const next: Record<string, TakeoverState> = {};
    for (const [id, s] of Object.entries(this.byAgent)) if (!s.takeover) next[id] = s;
    for (const t of list) next[t.agent_id] = { takeover: t, blocked: null };
    this.byAgent = next;
  }
}

export const takeovers = new Takeovers();

async function run(what: string, agentId: string, fn: () => Promise<TakeoverState>): Promise<TakeoverState | null> {
  try {
    const s = await fn();
    takeovers.set(agentId, s);
    return s;
  } catch (e) {
    toast({ kind: 'error', title: `Couldn’t ${what}`, body: msg(e) });
    // What the sheet shows may be older than the failure (a stop that timed out): ask again.
    await loadTakeover(agentId);
    return null;
  }
}

/** Ask the core where the agent is. A failure is shown in the agent's sheet, as the reason it cannot go. */
export async function loadTakeover(agentId: string) {
  try {
    takeovers.set(agentId, await api.agentTakeover(agentId));
  } catch (e) {
    takeovers.set(agentId, { takeover: null, blocked: msg(e) });
  }
}

/** Ask the core for every agent that is in your terminal: when an office opens, so they are marked from the start. */
export async function loadTakeovers() {
  let list: Takeover[];
  try {
    list = await api.listTakeovers();
  } catch (e) {
    toast({ kind: 'error', title: 'Couldn’t read which agents are in your terminal', body: msg(e) });
    return;
  }
  // Marked here and not on the list: given back elsewhere. The core says where it is now.
  const back = takeovers.active.map((t) => t.agent_id).filter((id) => !list.some((t) => t.agent_id === id));
  takeovers.setThere(list);
  await Promise.all(back.map(loadTakeover));
}

/** The Terminal room, on the agent's own tab: it opens the terminal with the command typed. */
export function showTerminal(agentId: string) {
  ui.terminalTakeover = agentId;
  ui.go('terminal');
}

/** "Open in my terminal": the core stops its own process for the session, then the terminal opens. */
export async function openInTerminal(agentId: string): Promise<boolean> {
  const s = await run('open it in your terminal', agentId, () => api.takeOverAgent(agentId));
  if (!s?.takeover) return false;
  showTerminal(agentId);
  return true;
}

/** "Give it back": the core closes the terminal it opened; the next message resumes the session in Codly. */
export async function giveBack(agentId: string): Promise<boolean> {
  return (await run('give it back', agentId, () => api.giveBackAgent(agentId))) !== null;
}
