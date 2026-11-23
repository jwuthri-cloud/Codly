// What you answered to an agent's question, by approval id. The core hands an answer to the
// agent's CLI and records only that the question was answered, so the words are known for the
// questions answered in this window: an older one reads "Answered by you" without them.
import { SvelteMap } from 'svelte/reactivity';

export const yourAnswers = new SvelteMap<string, string>();
