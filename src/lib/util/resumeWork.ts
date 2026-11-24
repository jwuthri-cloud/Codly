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

/** The room, in px, that the bar needs for its text on one line and the line under it: the
 *  longest question ("3 tasks were running when Codly closed. Resume them?") with Not now and
 *  Resume work beside it. */
export const BOTH_LINES = 600;

/** Does the bar show the line under its text? Only in a room of `BOTH_LINES` px or more (the
 *  window less a sheet or the Ask me panel beside it). In less room the line goes and the text
 *  takes two lines: the question is never cut. */
export const showsBothLines = (room: number): boolean => room >= BOTH_LINES;
