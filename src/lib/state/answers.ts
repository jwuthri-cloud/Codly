// What you answered to an agent's question, by approval id. The core hands an answer to the
// agent's CLI and records only that the question was answered, so the words are known for the
// questions answered in this window: an older one reads "Answered by you" without them.
import { SvelteMap } from 'svelte/reactivity';
import type { ApprovalChoice } from '../gen/ApprovalChoice';

export const yourAnswers = new SvelteMap<string, string>();

/** How you allowed a request, by approval id, for the requests allowed in this window: the closed
 *  row says "for this session" (and "for this turn" for a permissions request). The core keeps
 *  only that it was approved. */
export const yourChoice = new SvelteMap<string, ApprovalChoice>();
