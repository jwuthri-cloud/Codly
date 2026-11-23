// The launch question in words (src/components/ResumeWork.svelte): what was under way when
// Codly last closed, and whether the question or only its Resume work button is on screen.
import type { ResumeOffer } from '../gen/ResumeOffer';
import type { ResumeOnLaunch } from '../gen/ResumeOnLaunch';

/** What the question asks: how many tasks were running, or that work only waited to go on. */
export function resumeQuestion(offer: ResumeOffer): string {
  if (offer.tasks === 0) return 'Work was waiting to go on when Codly closed. Resume it?';
  if (offer.tasks === 1) return '1 task was running when Codly closed. Resume it?';
  return `${offer.tasks} tasks were running when Codly closed. Resume them?`;
}

/** Is the question itself on screen? With Ask, until you say "Not now". After that, and with
 *  Never, only the line with the Resume work button stays. */
export function asksNow(mode: ResumeOnLaunch, later: boolean): boolean {
  return mode === 'ask' && !later;
}
