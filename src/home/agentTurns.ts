// What the core keeps of each agent's turns, for "Undo its last turn" in the composer's `/` menu
// and in the palette. Only the core knows it, and it is asked when one of the two opens (the read
// starts git). The answers are kept per agent: a late answer for one agent never shows as
// another agent's, and an agent whose answer came never shows "Reading…" again.
import { SvelteMap } from 'svelte/reactivity';
import { api } from '../lib/api';
import type { Agent } from '../lib/gen/Agent';
import type { AgentCheckpoints } from '../lib/gen/AgentCheckpoints';

/**
 * Why no turn of the agent can be undone, from what the core says it keeps of the agent's turns
 * (`agent_checkpoints`): the core's reason when it keeps none for this agent, or that no turn
 * was kept yet. Null: there is a point to put the files back to.
 */
export function noTurn(kept: Pick<AgentCheckpoints, 'off' | 'checkpoints'>, name: string): string | null {
  return kept.off ?? (kept.checkpoints.length ? null : `Codly kept no turn of ${name} yet`);
}

export class AgentTurns {
  /** By agent id: why no turn of it can be undone (null: one can). An agent not in it was not answered yet. */
  readonly byAgent = new SvelteMap<string, { off: string | null }>();

  constructor(private readonly read: (agentId: string) => Promise<AgentCheckpoints>) {}

  /** The core's last answer for the agent; null: it has not answered yet. */
  of(agentId: string): { off: string | null } | null {
    return this.byAgent.get(agentId) ?? null;
  }

  /** Ask the core again. A refusal is the reason the row gives. */
  ask(agent: Pick<Agent, 'id' | 'name'>): Promise<void> {
    return this.read(agent.id).then(
      (kept) => void this.byAgent.set(agent.id, { off: noTurn(kept, agent.name) }),
      (e) => void this.byAgent.set(agent.id, { off: e instanceof Error ? e.message : String(e) }),
    );
  }
}

/** The one the composer and the palette share. */
export const agentTurns = new AgentTurns((agentId) => api.agentCheckpoints(agentId));
