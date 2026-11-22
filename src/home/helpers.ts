// A helper (a subagent the agent started with one tool call) in a work row: the call that
// started it is one step, and the helper's own calls show under that step, not among the
// agent's. Pure: the row gives its calls and the helpers its calls started.
import type { Seg, SubAgent } from '../lib/state/store.svelte';
import { plural, tokens } from '../lib/util/format';

/**
 * The agent's own steps of a row (`own`), and each helper's calls by the id of the call that
 * started the helper (`under`). A helper's helper shows under the first one. A call whose
 * helper is not in this row stays among the agent's steps: no call is ever left out.
 */
export function splitHelpers(segs: Seg[], helpers: SubAgent[]): { own: Seg[]; under: Map<string, Seg[]> } {
  const known = new Set(helpers.map((h) => h.callId));
  const byCall = new Map(segs.map((s) => [s.callId, s]));
  const own: Seg[] = [];
  const under = new Map<string, Seg[]>();
  for (const s of segs) {
    let top: string | undefined;
    for (let p = s.parent; p != null && known.has(p) && byCall.has(p); p = byCall.get(p)?.parent) top = p;
    if (top == null) own.push(s);
    else under.set(top, [...(under.get(top) ?? []), s]);
  }
  return { own, under };
}

export type HelperState = 'running' | 'done' | 'failed';

/**
 * Where a helper stands. Its status is the CLI's own word: "running", "completed", "failed",
 * "stopped"… A word that says neither (another CLI's "started") follows the call that started it.
 */
export function helperState(h: SubAgent, call: Seg): HelperState {
  if (h.status === 'running') return 'running';
  if (/complet|done|finish/i.test(h.status)) return 'done';
  if (/fail|stop|kill|cancel|error/i.test(h.status)) return 'failed';
  return call.end == null ? 'running' : call.ok === false ? 'failed' : 'done';
}

/** The headline of a helper's step, in plain words. */
export function helperTitle(h: SubAgent, state: HelperState): string {
  if (state === 'running') return 'Helper at work';
  if (state === 'done') return 'Helper finished';
  return h.status === 'failed' || !/stop|kill|cancel/i.test(h.status) ? 'Helper failed' : 'Helper stopped';
}

/** How much a helper did so far: "3 steps · 5.9k tokens" (empty when the CLI did not say). */
export function helperProgress(h: SubAgent): string {
  const parts: string[] = [];
  if (h.toolUses) parts.push(plural(h.toolUses, 'step'));
  if (h.tokens) parts.push(`${tokens(h.tokens)} tokens`);
  return parts.join(' · ');
}
