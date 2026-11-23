// The launch question, from the screen's side. Work was under way when Codly last closed, and
// the core starts nothing of it before you say so (orchestrator/resume.rs), unless "Resume work
// when Codly opens" is on Always. With Ask the question is on screen until you answer. After
// "Not now", and with Never, one line with the Resume work button stays: the work waits for it.
import { api } from '../api';
import type { ResumeOffer } from '../gen/ResumeOffer';
import { toast } from './toasts.svelte';

const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

class ResumeWork {
  /** What waits for your Resume (null: nothing waits, or not read yet). */
  offer = $state.raw<ResumeOffer | null>(null);
  /** You said "Not now": the question is put away until Codly opens again. */
  later = $state(false);
  /** Your Resume is on its way to the core. */
  busy = $state(false);

  /** Ask the core what waits. Once the core is up: only a start of Codly holds work. */
  async check() {
    try {
      this.offer = await api.resumeWork();
    } catch (e) {
      // Not read: if work waits, nothing starts and no button says so. Offer the read again.
      toast({ kind: 'error', title: 'Couldn’t check for work to resume', body: msg(e), action: { label: 'Try again', run: () => void this.check() } });
    }
  }

  /** Resume: the work goes on. */
  async resume() {
    this.busy = true;
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
