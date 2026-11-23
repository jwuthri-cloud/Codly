// The "Model" row of the agent sheet's Controls: the model and its thinking effort in words,
// and whether "Change…" (the same dialog as the sheet's menu entry "Change model…") can run.
// Pure, so the sheet and the tests agree.
import type { Agent } from '../lib/gen/Agent';
import { effortLabel } from '../lib/util/models';
import { splitModel } from '../lib/util/team';

/**
 * "Opus 4.7 · High effort": `model`, the model in words (the sheet's own: the one picked for
 * the agent, else the one its CLI reported or runs by default), then the effort picked with it
 * (`saved`: the agent's model as Codly keeps it, "opus:high"). The model alone when none was
 * picked: the CLI's own default applies.
 */
export function modelLine(model: string, saved: string | null): string {
  const { effort } = splitModel(saved);
  return effort ? `${model} · ${effortLabel(effort)} effort` : model;
}

/** Why the model of this agent cannot change from its sheet; null: it can. */
export const modelChangeOff = (a: Pick<Agent, 'role'>): string | null => (a.role === 'solo' ? null : 'Only a 1:1 agent’s model can change here.');
