// The "Helpers" section of the agent sheet: how a helper that is over ended, in one word, and
// the section's line. Pure, so the sheet and the tests agree.
import type { SubAgent } from '../lib/state/store.svelte';
import { oneLine } from '../lib/util/sanitize';

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
