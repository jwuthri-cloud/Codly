// The launch question, from the screen's side. Work was under way when Codly last closed, and
// the core starts nothing of it before you say so (orchestrator/resume.rs), unless "Resume work
// when Codly opens" is on Always. With Ask the question is on screen until you answer. After
// "Not now", and with Never, one line with the Resume work button stays: the work waits for it.
import { api } from '../api';
import type { ResumeOffer } from '../gen/ResumeOffer';
import { toast } from './toasts.svelte';

const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** While the question is on screen, the core is asked again at most this often. */
export const REREAD_MS = 1000;

class ResumeWork {
  /** What waits for your Resume (null: nothing waits, or not read yet). */
  offer = $state.raw<ResumeOffer | null>(null);
  /** You said "Not now": the question is put away until Codly opens again. */
  later = $state(false);
  /** Your Resume is on its way to the core. */
  busy = $state(false);
  /** Counts the questions to the core: only the answer to the newest one is shown. */
  private asked = 0;
  /** A read that waits for its turn (`changed`). */
  private timer: ReturnType<typeof setTimeout> | null = null;

  /** Ask the core what waits. Once the core is up: only a start of Codly holds work. */
  async check() {
    const mine = ++this.asked;
    try {
      const offer = await api.resumeWork();
      if (mine === this.asked) this.offer = offer;
    } catch (e) {
      // Not read: if work waits, nothing starts and no button says so. Offer the read again.
      toast({ kind: 'error', title: 'Couldn’t check for work to resume', body: msg(e), action: { label: 'Try again', run: () => void this.check() } });
    }
  }

  /**
   * A task or a plan changed while the question is on screen: what it says may not be true any
   * more (you cancelled the tasks that waited, or another window resumed them). The core is asked
   * again, once for all the changes of a second. Nothing to do when no question is on screen:
   * work is put on hold only when Codly starts.
   */
  changed() {
    if (!this.offer || this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.reread();
    }, REREAD_MS);
  }

  /** The count again, or that nothing waits any more (the bar goes). Your Resume says it too: no read beside it. */
  private async reread() {
    if (!this.offer || this.busy) return;
    const mine = ++this.asked;
    try {
      const offer = await api.resumeWork();
      if (mine === this.asked) this.offer = offer;
    } catch (e) {
      // The bar stays as it was read last, with its button; the next change asks again.
      console.error('[codly] could not read again what waits for Resume', e);
    }
  }

  /** Resume: the work goes on. The answer says what still waits (nothing, when all of it went on). */
  async resume() {
    this.busy = true;
    // A read still on its way was answered before this Resume: its answer does not count.
    this.asked++;
    try {
      this.offer = await api.resumeWork(true);
    } catch (e) {
      toast({ kind: 'error', title: 'Couldn’t resume the work', body: msg(e) });
    } finally {
      this.busy = false;
    }
  }

  /** Not now: the work stays on hold, with the Resume work button. */
  notNow() {
    this.later = true;
  }
}

export const resumeWork = new ResumeWork();
