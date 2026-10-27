// Reactive onboarding progress over src/lib/onboarding.ts. App.svelte calls `sync()` in an effect
// so quests complete (and celebrate) from live state whichever screen is up.

import { EMPTY, QUESTS, cliReady, completed, levelOf, loadProgress, saveProgress, totalXp, type Progress, type QuestId } from '../lib/onboarding';
import { store } from '../lib/state/store.svelte';

export type Celebration = { title: string; xp: number; level: string | null; nonce: number };

class QuestLog {
  progress = $state<Progress>(loadProgress());
  /** The latest completion, for the sparkles and the aria-live line; cleared after a few seconds. */
  celebration = $state<Celebration | null>(null);

  readyClis = $derived(store.clis.filter(cliReady).length);
  done = $derived(completed(store, this.progress));
  xp = $derived(totalXp(this.done, this.readyClis));
  level = $derived(levelOf(this.xp));
  next = $derived(QUESTS.find((q) => !this.done.includes(q.id)) ?? null);
  allDone = $derived(this.done.length === QUESTS.length);
  /** The full-screen first-run flow: until dismissed, or until you have opened a repository. */
  showFlow = $derived(!this.progress.dismissed && !this.progress.done.includes('base'));
  showTracker = $derived(!this.progress.dismissed && !this.allDone);

  private timer: ReturnType<typeof setTimeout> | undefined;

  /** Record newly completed quests (sticky) and celebrate the last one. */
  sync() {
    const fresh = this.done.filter((id) => !this.progress.done.includes(id));
    if (!fresh.length) return;
    const before = levelOf(totalXp(this.progress.done, this.readyClis));
    this.set({ done: [...this.progress.done, ...fresh] });
    const q = QUESTS.find((x) => x.id === fresh[fresh.length - 1])!;
    const after = levelOf(totalXp(this.progress.done, this.readyClis));
    this.celebration = { title: q.title, xp: q.xp, level: after.index > before.index ? after.name : null, nonce: Math.random() };
    clearTimeout(this.timer);
    this.timer = setTimeout(() => (this.celebration = null), 3200);
  }

  flag(f: keyof Progress['flags']) {
    this.set({ flags: { ...this.progress.flags, [f]: true } });
  }
  dismiss() {
    this.set({ dismissed: true });
  }
  replay() {
    this.set({ ...EMPTY, flags: { ...EMPTY.flags } });
  }
  isDone(id: QuestId) {
    return this.done.includes(id);
  }

  private set(patch: Partial<Progress>) {
    this.progress = { ...this.progress, ...patch };
    saveProgress(this.progress);
  }
}

export const quest = new QuestLog();
