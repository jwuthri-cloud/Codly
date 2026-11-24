// The "Helpers" section of the agent sheet: how a helper that is over ended, in one word, the
// section's line, and what the "Stop" of a helper that runs stops. Pure, so the sheet and the
// tests agree.
import { stopHint, waitedHelperStopHint, type BackgroundWork } from '../lib/state/session';
import type { SubAgent } from '../lib/state/store.svelte';
import { oneLine } from '../lib/util/sanitize';

/**
 * What the "Stop" of a helper's row sends (`id`, for `stop_agent_task`) and says on its button.
 * `id` null: the row has no "Stop", and `hint` says why.
 */
export type HelperStop = { id: string | null; hint: string };

/** Why a helper's row has no "Stop": the background list cannot tell it from another helper. */
export const SAME_DESCRIPTION = 'Two helpers have this description: stop them in “In the background”.';

/**
 * The same, when one of them is a helper its agent waits for: that one is not in "In the
 * background" at all, and ends when you stop the agent's turn in the conversation.
 */
export const sameDescriptionWaited = (agent: string): string =>
  `Two helpers have this description: stop the one in the background in “In the background”, and the one ${agent} waits for by stopping its turn in the conversation.`;

/**
 * The "Stop" of a helper's row, for a helper that runs (none for one that is over). The
 * background list (`background`: the agent's live run's) names a helper by its description
 * only, so a row is stopped by the id there only when that is sure: exactly one helper that
 * runs (`helpers`: the agent's helpers) has the description, and exactly one entry of the list
 * has it; its button then says what its "Stop" there says (`turnRuns`: a turn of the agent runs
 * now, `stopHint`). A helper the list does not name is one its agent still waits for: it is
 * stopped by the id of the call that started it, its call returns to the agent and the turn
 * goes on (recorded: docs/research/cli-claude-stream-json-2.1.311-part4.md, section 2). In any
 * other case which entry is which helper is not known (two helpers of one description, one of
 * them listed and one waited for, or both listed): the row has no "Stop", and says where to stop
 * them (`sameDescriptionWaited` when one of them is waited for).
 */
export function helperStop(
  h: Pick<SubAgent, 'callId' | 'description' | 'status'>,
  helpers: readonly Pick<SubAgent, 'description' | 'status'>[],
  background: readonly BackgroundWork[],
  agent: string,
  turnRuns: boolean,
): HelperStop | null {
  if (h.status !== 'running') return null;
  const listed = background.filter((w) => w.kind === 'helper' && w.status === 'running' && w.description === h.description);
  if (!listed.length) return { id: h.callId, hint: waitedHelperStopHint(agent) };
  const alike = helpers.filter((x) => x.status === 'running' && x.description === h.description).length;
  if (listed.length === 1 && alike === 1) return { id: listed[0].id, hint: stopHint(listed[0], agent, turnRuns)! };
  return { id: null, hint: alike > listed.length ? sameDescriptionWaited(agent) : SAME_DESCRIPTION };
}

/**
 * How a helper that is no longer at work ended, as its row says it. The CLI's "completed" is
 * "finished". "stopped" and "killed" are "stopped": you, Codly or the end of its turn stopped
 * it, so it did not finish. "failed" is "failed". Another word of a CLI stays as it is: Codly
 * does not guess what it means.
 */
export function helperEndWord(status: string): string {
  if (status === 'completed') return 'finished';
  if (status === 'stopped' || status === 'killed') return 'stopped';
  return oneLine(status, 24) || 'ended';
}

/** The words of the section's line that come first, in this order; a CLI's other words follow as they come. */
const ORDER = ['running', 'finished', 'stopped', 'failed'];

/** The section's line: "1 running", "2 finished · 1 stopped", "1 running · 1 failed". */
export function helpersSummary(helpers: readonly Pick<SubAgent, 'status'>[]): string {
  const counts = new Map<string, number>();
  for (const h of helpers) {
    const word = h.status === 'running' ? 'running' : helperEndWord(h.status);
    counts.set(word, (counts.get(word) ?? 0) + 1);
  }
  const words = [...ORDER.filter((w) => counts.has(w)), ...[...counts.keys()].filter((w) => !ORDER.includes(w))];
  return words.map((w) => `${counts.get(w)} ${w}`).join(' · ');
}
