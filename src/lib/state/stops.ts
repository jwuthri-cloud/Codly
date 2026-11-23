// The turns you asked to stop yourself (Stop, Esc, Send now, Interrupt): a button says
// "Stopping…" until that turn ends, and the conversation does not report it as a problem.
// A turn stopped with Esc is the exception: a key leaves nothing on screen, so the conversation
// says that the turn was interrupted.
// A turn is named by its start time (the core's clock), so no two clocks are ever compared.
import { SvelteMap } from 'svelte/reactivity';
import { store } from './store.svelte';

const KEPT = 20;
/** agent → the turns you asked to stop, by their start; `byKey`: with Esc. */
const asked = new SvelteMap<string, { start: number; byKey: boolean }[]>();

/** The start of the agent's running turn (null: it is between turns). */
export function openTurn(agentId: string): number | null {
  void store.rev;
  const turns = store.live[agentId]?.turns ?? [];
  const last = turns[turns.length - 1];
  return last && last.end == null ? last.start : null;
}

/** Remember that you asked to stop the agent's running turn (`byKey`: with Esc). Call the result if the core refused. */
export function noteStop(agentId: string, byKey = false): () => void {
  const start = openTurn(agentId);
  if (start == null) return () => {};
  asked.set(agentId, [...(asked.get(agentId) ?? []), { start, byKey }].slice(-KEPT));
  return () => asked.set(agentId, (asked.get(agentId) ?? []).filter((s) => s.start !== start));
}

/**
 * Whether the end of the agent's turn that started at `turnStart` needs no line in the
 * conversation: you stopped it with a button, and the button said so. A turn you stopped with
 * Esc gets its line ("…’s turn was interrupted").
 */
export const stoppedByYou = (agentId: string, turnStart: number): boolean => !!asked.get(agentId)?.some((s) => s.start === turnStart && !s.byKey);

/** Whether the agent's running turn is one you asked to stop (it ends when the CLI lets go). */
export function stopping(agentId: string): boolean {
  const start = openTurn(agentId);
  return start != null && !!asked.get(agentId)?.some((s) => s.start === start);
}
