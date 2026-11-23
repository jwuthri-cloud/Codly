// What you said with your answer to an agent's request, by approval id: the answer to a
// question, the note of a "no", and that you changed a plan before you accepted it. The core
// puts it on record with the answer (the `approval_said` row), so it is known after a restart
// too. A request answered before the core recorded this reads "Answered by you" without the words.
import { SvelteMap } from 'svelte/reactivity';
import type { EventKind } from '../gen/EventKind';

/** Your answer to a question, on one line. */
export const yourAnswers = new SvelteMap<string, string>();

/** What else you said with an answer: the note of a "no", and that a plan went ahead with your text. */
export type Words = { note: string | null; planChanged: boolean };
export const yourWords = new SvelteMap<string, Words>();

/** A row of the record, live or read again: what you said with the answer to that request. */
export function recordSaid(k: Extract<EventKind, { type: 'approval_said' }>): void {
  if (k.answer) yourAnswers.set(k.approval_id, k.answer);
  if (k.note || k.plan_changed) yourWords.set(k.approval_id, { note: k.note, planChanged: k.plan_changed });
}
