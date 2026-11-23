// Settings → Safety: what you can do now with a text from the repo that Codly runs only once you
// turned it on (New worktrees: the setup script; Required checks: the checks file).

import type { RequiredChecks } from '../gen/RequiredChecks';
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

export function setupChoices(s: WorktreeSetup | RequiredChecks): SetupChoices {
  return { on: s.on ? null : s.sha, off: s.on || s.changed, ask: s.changed };
}

/**
 * What to say when turning the required checks on (`allow`: the fingerprint of the text) or off
 * (null) ended with an error. `now`: the checks as the core has them after that (null: they
 * could not be read either). The core saves the choice first, and only then looks for the tasks
 * that waited for it: an error can come after the choice was saved, and then the checks did change.
 */
export function checksTurnTrouble(allow: string | null, now: RequiredChecks | null): string {
  if (!now) return 'Couldn’t tell whether the required checks changed';
  const saved = allow === null ? !now.on && !now.changed : now.on && now.sha === allow;
  if (!saved) return 'Couldn’t change the required checks';
  return `The required checks are ${allow === null ? 'off' : 'on'} now, but Codly may not have checked the waiting tasks again`;
}
