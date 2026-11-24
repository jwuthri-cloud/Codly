// The words of the sheet's "Needs you" about the keys: ⌘↵, ⌘⌫ and the digits answer the oldest
// waiting request on screen (keys.ts `keyTargetApproval`), which can be another agent's than the
// one this sheet shows. Pure, so the sheet and the tests agree.
import type { Approval } from '../lib/gen/Approval';

/** The agent whose request the keys answer, when it is none of the requests the sheet shows (null: the keys answer one of them, or none waits). */
export function keysAnswerOther(target: Pick<Approval, 'id' | 'agent_id'> | null, shown: readonly Pick<Approval, 'id'>[]): string | null {
  if (!target || shown.some((a) => a.id === target.id)) return null;
  return target.agent_id;
}

/** What "Needs you" says then (`name`: the agent whose request the keys answer). */
export const keysAnswerOtherLine = (name: string): string => `The keys answer the oldest request first (${name}’s). Use the buttons here.`;
