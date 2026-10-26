// Toasts (design §4): bottom-right, max 3, progress updates in place, success 4 s, info 6 s,
// errors persist. Hover pauses dismissal. Approvals are never toast-only: an approval toast
// is just a pointer into the Inbox.

export type ToastKind = 'progress' | 'success' | 'info' | 'error' | 'attention';
export type ToastAction = { label: string; run: () => void; kbd?: string };
export type Toast = {
  id: number;
  kind: ToastKind;
  title: string;
  body?: string;
  action?: ToastAction;
  /** ms; 0 = sticky */
  duration: number;
  createdAt: number;
};

const DEFAULT_DURATION: Record<ToastKind, number> = { progress: 0, success: 4000, info: 6000, error: 0, attention: 8000 };
const MAX = 3;

class Toasts {
  list = $state<Toast[]>([]);
  private seq = 0;
  private timers = new Map<number, ReturnType<typeof setTimeout>>();
  private remaining = new Map<number, number>();
  private startedAt = new Map<number, number>();

  show(t: { kind?: ToastKind; title: string; body?: string; action?: ToastAction; duration?: number }): number {
    const kind = t.kind ?? 'info';
    const id = ++this.seq;
    const toast: Toast = { id, kind, title: t.title, body: t.body, action: t.action, duration: t.duration ?? DEFAULT_DURATION[kind], createdAt: Date.now() };
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

  pause(id: number) {
    const timer = this.timers.get(id);
    if (!timer) return;
    clearTimeout(timer);
    this.timers.delete(id);
    const started = this.startedAt.get(id) ?? Date.now();
    this.remaining.set(id, Math.max(1200, (this.remaining.get(id) ?? 0) - (Date.now() - started)));
  }

  resume(id: number) {
    const rem = this.remaining.get(id);
    if (rem != null && !this.timers.has(id)) this.arm(id, rem);
  }

  private arm(id: number, ms: number) {
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    if (!ms) return;
    this.remaining.set(id, ms);
    this.startedAt.set(id, Date.now());
    this.timers.set(id, setTimeout(() => this.dismiss(id), ms));
  }
}

export const toasts = new Toasts();
export const toast = (t: Parameters<Toasts['show']>[0]) => toasts.show(t);
