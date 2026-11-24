// Toasts (design §4): top-right under the top bar, laid over the page at a place where they cover
// nothing you can press (see components/Toasts.svelte), max 3, progress updates in place, success 4 s, info 6 s,
// errors persist. Hover pauses dismissal, and so does a folded stack for the toasts it has put
// away (`putAway`): a toast does not go before it was shown. The same message shows once, with a
// count. Approvals are never toast-only: an approval toast is just a pointer into the Inbox.

export type ToastKind = 'progress' | 'success' | 'info' | 'error' | 'attention';
/** `gone`: the action can no longer be done (a merge it offers was made, what refused it is no
 *  more): the toast goes (`dropGone`). It may read reactive state: the stack asks it again when that changes. */
export type ToastAction = { label: string; run: () => void; kbd?: string; gone?: () => boolean };
export type Toast = {
  id: number;
  kind: ToastKind;
  title: string;
  body?: string;
  action?: ToastAction;
  /** ms; 0 = sticky */
  duration: number;
  createdAt: number;
  /** How often this message was shown: an equal one joins the toast that shows (`show`). */
  count: number;
};

const DEFAULT_DURATION: Record<ToastKind, number> = { progress: 0, success: 4000, info: 6000, error: 0, attention: 8000 };
const MAX = 3;

class Toasts {
  list = $state<Toast[]>([]);
  private seq = 0;
  private timers = new Map<number, ReturnType<typeof setTimeout>>();
  private remaining = new Map<number, number>();
  private startedAt = new Map<number, number>();
  /** The toasts that are put away now (`putAway`): none of them counts down. */
  private away = new Set<number>();
  /** The page is out of view (`seen`): no toast counts down. */
  private hidden = false;

  /**
   * Shows a toast, and gives its id. A toast with the kind, the title and the text of one that
   * shows is not added: that one counts it, takes its time (which starts again) and keeps its
   * place. Never joined: a progress toast (each one tells of its own work), and a toast with an
   * action, new or showing (its action and its id are its own: an Undo, a pointer to one approval).
   */
  show(t: { kind?: ToastKind; title: string; body?: string; action?: ToastAction; duration?: number }): number {
    const kind = t.kind ?? 'info';
    const duration = t.duration ?? DEFAULT_DURATION[kind];
    const same = kind === 'progress' || t.action ? undefined : this.list.find((x) => !x.action && x.kind === kind && x.title === t.title && x.body === t.body);
    if (same) {
      this.list = this.list.map((x) => (x.id === same.id ? { ...x, duration, count: x.count + 1 } : x));
      this.restart(same.id, duration);
      return same.id;
    }
    const id = ++this.seq;
    const toast: Toast = { id, kind, title: t.title, body: t.body, action: t.action, duration, createdAt: Date.now(), count: 1 };
    const next = [...this.list, toast];
    // Drop the oldest dismissible toasts beyond the cap.
    while (next.length > MAX) {
      const i = next.findIndex((x) => x.kind !== 'error' && x.kind !== 'progress');
      next.splice(i >= 0 ? i : 0, 1);
    }
    this.list = next;
    this.arm(id, toast.duration);
    return id;
  }

  update(id: number, patch: Partial<Omit<Toast, 'id'>>) {
    this.list = this.list.map((t) => (t.id === id ? { ...t, ...patch } : t));
    if (patch.kind && patch.duration == null) {
      const d = DEFAULT_DURATION[patch.kind];
      this.list = this.list.map((t) => (t.id === id ? { ...t, duration: d } : t));
      this.arm(id, d);
    } else if (patch.duration != null) this.arm(id, patch.duration);
  }

  dismiss(id: number) {
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    this.list = this.list.filter((t) => t.id !== id);
  }

  /** The toasts you can dismiss: all but the progress toasts, which go when their work ends. */
  get dismissible(): Toast[] {
    return this.list.filter((t) => t.kind !== 'progress');
  }

  /** "Dismiss all": every toast you can dismiss goes; a progress toast stays. */
  dismissAll() {
    for (const t of this.dismissible) this.dismiss(t.id);
  }

  /** Every toast whose action can no longer be done goes: it would offer what is not there any more. */
  dropGone() {
    for (const t of this.list) if (t.action?.gone?.()) this.dismiss(t.id);
  }

  /** The newest toast that shows and waits for you (Esc dismisses it): an error, or one with no
   *  time limit; never a progress toast, and not one that a folded stack has put away. */
  get waiting(): Toast | undefined {
    return this.dismissible.findLast((t) => (t.kind === 'error' || t.duration === 0) && !this.away.has(t.id));
  }

  pause(id: number) {
    const timer = this.timers.get(id);
    if (!timer) return;
    clearTimeout(timer);
    this.timers.delete(id);
    const started = this.startedAt.get(id) ?? Date.now();
    this.remaining.set(id, Math.max(1200, (this.remaining.get(id) ?? 0) - (Date.now() - started)));
  }

  resume(id: number) {
    // Not one that is put away (the pointer can leave it at the moment the stack folds), not one
    // that went, and none while the page is out of view.
    if (this.hidden || this.away.has(id) || !this.list.some((t) => t.id === id)) return;
    const rem = this.remaining.get(id);
    if (rem != null && !this.timers.has(id)) this.arm(id, rem);
  }

  /**
   * Says which toasts are put away: in the list, but not shown (components/Toasts.svelte folds a
   * stack that finds no free place, and shows one toast of it). A toast that is put away does not
   * count down: it would go without ever being seen. When it shows again it goes on with the time
   * it had left, 1.2 s at least. `ids`: all toasts that are put away now.
   */
  putAway(ids: number[]) {
    const before = this.away;
    this.away = new Set(ids);
    for (const id of this.away) if (!before.has(id)) this.pause(id);
    for (const id of before) if (!this.away.has(id)) this.resume(id);
  }

  /**
   * The page is in view (`visible`) or not (another tab or window is in front). While it is not,
   * no toast counts down, so none goes before you could see it: one that came meanwhile starts
   * its time when the page is seen, one that showed goes on with the time it had left, 1.2 s at least.
   */
  seen(visible: boolean) {
    if (this.hidden === !visible) return;
    this.hidden = !visible;
    for (const t of this.list) {
      if (this.hidden) this.pause(t.id);
      else this.resume(t.id);
    }
  }

  /** Its time starts again. One that stands still now (the pointer is on it, or it is put away) gets the whole time when it goes on. */
  private restart(id: number, ms: number) {
    if (ms && !this.timers.has(id) && this.remaining.has(id)) this.remaining.set(id, ms);
    else this.arm(id, ms);
  }

  private arm(id: number, ms: number) {
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    if (!ms) {
      // It stays from now on: the time it had left before must not start again (`resume`).
      this.remaining.delete(id);
      return;
    }
    this.remaining.set(id, ms);
    // Put away, or the page is out of view: its time starts when it shows again.
    if (this.away.has(id) || this.hidden) return;
    this.startedAt.set(id, Date.now());
    this.timers.set(id, setTimeout(() => this.dismiss(id), ms));
  }
}

export const toasts = new Toasts();
export const toast = (t: Parameters<Toasts['show']>[0]) => toasts.show(t);
