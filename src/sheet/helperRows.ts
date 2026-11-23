// The "Helpers" section of the agent sheet: how a helper that is over ended, in one word, the
// section's line, and what the "Stop" of a helper that runs stops. Pure, so the sheet and the
// tests agree.
import { stopHint, waitedHelperStopHint, type BackgroundWork } from '../lib/state/session';
import type { SubAgent } from '../lib/state/store.svelte';
import { oneLine } from '../lib/util/sanitize';

/** What the "Stop" of a helper's row sends (`id`, for `stop_agent_task`) and says on its button. */
export type HelperStop = { id: string; hint: string };

/**
 * The "Stop" of a helper's row, for a helper that runs (none for one that is over). A helper in
 * the background list (`background`: the agent's live run's) is stopped by its id there, and its
 * button says what its "Stop" there says. A helper its agent still waits for is stopped by the
 * id of the call that started it: its call returns to the agent and the turn goes on (recorded:
 * docs/research/cli-claude-stream-json-2.1.311-part4.md, section 2). The list names a helper by
 * its description: when two helpers that run have the same one, which is which is not known,
 * and the row has no "Stop" (each has its own in "In the background").
 */
export function helperStop(h: Pick<SubAgent, 'callId' | 'description' | 'status'>, background: readonly BackgroundWork[], agent: string): HelperStop | null {
  if (h.status !== 'running') return null;
  const listed = background.filter((w) => w.kind === 'helper' && w.status === 'running' && w.description === h.description);
  if (listed.length > 1) return null;
  if (listed.length === 1) return { id: listed[0].id, hint: stopHint(listed[0], agent)! };
  return { id: h.callId, hint: waitedHelperStopHint(agent) };
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
