// Two lines of the model pickers that say what Codly does: what a switch of model does to the
// agent's conversation (the model dialog), and what a list that is not the CLI's own is (the
// Lead's model menu).

import type { Agent } from '../gen/Agent';

/**
 * What switching the model does to the agent's conversation, after the CLI's name. A Claude
 * agent with a live process takes the new model with no restart (orchestrator/runs.rs,
 * `live_model_switch`); any other agent starts again with your next message and resumes its
 * conversation there.
 */
export function switchNote(agent: Pick<Agent, 'provider' | 'run_id' | 'status'>): string {
  const live = agent.provider === 'claude' && !!agent.run_id && agent.status !== 'offline';
  return live ? 'The agent keeps its conversation: its next turn runs on the new model.' : 'The next message resumes the same conversation on the new model.';
}

/**
 * The first rows of a model menu whose list is not the CLI's own, under the menu's header
 * "<CLI> model" (it says what was asked for). The first row is the one that tries again.
 * `listed`: the CLI could not be asked and Codly's own names are shown in its place. A menu
 * row is one short line that is cut when it is longer: two sentences, two rows.
 */
export function listTroubleRows(cli: string, listed: boolean): string[] {
  return listed ? [`Couldn’t ask ${cli}`, 'These are the names Codly knows'] : ['Couldn’t list the models'];
}
