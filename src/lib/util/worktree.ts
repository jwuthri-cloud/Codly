// Settings → Safety → New worktrees: what you can do with the repo's setup script now.

import type { WorktreeSetup } from '../gen/WorktreeSetup';

export interface SetupChoices {
  /** The fingerprint to send to turn on the text shown; null = it is on, or there is no text Codly can run. */
  on: string | null;
  /** A text is on, or was turned on and is not the file's text any more: it can be turned off. */
  off: boolean;
  /** The switch alone cannot say it: the text you turned on is not the file's text any more.
   *  Each choice is then its own button, so turning it off does not first turn the new text on. */
  ask: boolean;
}

export function setupChoices(s: WorktreeSetup): SetupChoices {
  return { on: s.on ? null : s.sha, off: s.on || s.changed, ask: s.changed };
}
