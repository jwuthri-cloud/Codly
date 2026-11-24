// ⌘↵ runs a plan only while its "Run plan" button is in view, uncovered and still, as a card's
// parts must be for the approval keys (`keysView`, `holdStep`, `settled`): a press before only
// brings the button into view. Its keycap shows only then, and never while a request card holds
// the keys or a moment after an answer: one ⌘↵ keycap on screen at a time.
import { holdStep, keyAct, NO_HOLD, restsAfterAnswer, settled } from '../../lib/util/approvalCard';
import { keyTargetApproval, keyTargetPlan, lastAnswerAt, registerPlanKey } from '../../lib/state/keys';
import { ui } from '../../lib/state/ui.svelte';
import { bringIntoView, readPart } from './cardParts';

/** Watches the "Run plan" button of `planId` while it shows; `onCap` hears whether its ⌘↵ keycap shows. Returns the stop. */
export function watchRunPlan(button: HTMLElement, planId: string, onCap: (shows: boolean) => void): () => void {
  /** It showed from now: the wait counts from here. */
  const from = performance.now();
  let hold = NO_HOLD;
  const act = (now: number) => {
    const read = button.isConnected ? readPart(button) : { view: 'out' as const, box: null };
    hold = holdStep(hold, now, read.view === 'ok', read.box ? [read.box] : []);
    return keyAct(read.view, settled(hold, now, from));
  };
  let shows = false;
  let frame = 0;
  const each = () => {
    const now = performance.now();
    const cap = act(now) === 'answer' && !keyTargetApproval() && keyTargetPlan() === planId && !restsAfterAnswer(now, lastAnswerAt());
    if (cap !== shows) onCap((shows = cap));
    frame = requestAnimationFrame(each);
  };
  frame = requestAnimationFrame(each);
  const off = registerPlanKey(planId, { act: () => act(performance.now()), reveal: () => bringIntoView([button], !ui.reducedMotion) });
  return () => {
    cancelAnimationFrame(frame);
    off();
  };
}
