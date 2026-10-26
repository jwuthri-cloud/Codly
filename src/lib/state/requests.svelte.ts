// Requests: each thing you asked for is its own thread ("lead", "lead:<id>") with its own Lead
// and plan, running in parallel. This is the one list every surface reads (tabs on Home, the
// crew strip's Lead tips, ⌘[ / ⌘], the Plan room's request picker).
import type { Agent } from '../gen/Agent';
import type { Plan } from '../gen/Plan';
import { oneLine } from '../util/sanitize';
import { store } from './store.svelte';
import { isRequestThread, ui } from './ui.svelte';

/** new = nothing asked yet · planning = the Lead is drafting · awaiting = the plan waits for you. */
export type RequestPhase = 'new' | 'talking' | 'planning' | 'awaiting' | 'running' | 'done' | 'merged' | 'failed' | 'cancelled';

export type Request = {
  thread: string;
  /** Your first message, on one line ("New request" until you write it). */
  title: string;
  ts: number;
  lead: Agent | null;
  plan: Plan | null;
  phase: RequestPhase;
  /** Pending approvals and questions from this request's agents. */
  waiting: number;
  /** Amber: the plan waits for your go, or an agent waits for your answer. */
  needsYou: boolean;
  /** One short status: "Planning", "Waiting for you", "Running · 1 of 3", "Ready to merge"… */
  status: string;
};

const LIVE_TASK = new Set(['running', 'verifying', 'reviewing', 'merging']);

class Requests {
  list = $derived.by((): Request[] => {
    void store.rev;
    const seen = new Map<string, { ts: number; title: string | null }>();
    const note = (thread: string, ts: number, title: string | null = null) => {
      const prev = seen.get(thread);
      if (!prev) seen.set(thread, { ts, title });
      else {
        if (ts < prev.ts) prev.ts = ts;
        if (!prev.title && title) prev.title = title;
      }
    };
    for (const c of store.chats) if (isRequestThread(c.thread)) note(c.thread, c.ts, c.author === 'user' ? c.text : null);
    for (const o of store.outbox) if (isRequestThread(o.thread)) note(o.thread, o.ts, o.text);
    for (const p of store.plans) note(p.thread || 'lead', p.created_at, p.request || null);
    for (const a of store.leads) note(a.thread ?? 'lead', a.created_at);
    for (const d of ui.draftRequests) note(d.thread, d.ts);

    const approvalsBy = new Map<string, number>();
    for (const ap of store.pendingApprovals) {
      const t = store.approvalThread(ap);
      approvalsBy.set(t, (approvalsBy.get(t) ?? 0) + 1);
    }

    const out: Request[] = [];
    for (const [thread, s] of seen) {
      const lead = store.leadOf(thread);
      const plan = store.planOf(thread);
      const waiting = approvalsBy.get(thread) ?? 0;
      const tasks = plan ? store.tasks.filter((t) => t.plan_id === plan.id) : [];
      const done = tasks.filter((t) => t.status === 'done').length;
      const turns = lead ? (store.live[lead.id]?.turns ?? []) : [];
      const leadBusy = !!turns.length && turns[turns.length - 1].end == null;
      let phase: RequestPhase;
      if (!plan || (plan.status === 'cancelled' && leadBusy)) phase = leadBusy || (lead && (lead.status === 'starting' || lead.status === 'thinking')) ? 'planning' : s.title ? 'talking' : 'new';
      else if (plan.status === 'planning') phase = 'planning';
      else if (plan.status === 'awaiting_approval') phase = 'awaiting';
      else if (plan.status === 'running' || plan.status === 'integrating') phase = 'running';
      else if (plan.status === 'done') phase = plan.merged_sha || store.planMerges[plan.id] ? 'merged' : 'done';
      else phase = plan.status;
      // A fresh message to the Lead after its plan settled: it is thinking about the next one.
      if ((phase === 'done' || phase === 'merged' || phase === 'cancelled' || phase === 'failed') && leadBusy) phase = 'planning';
      const live = tasks.filter((t) => LIVE_TASK.has(t.status)).length;
      const kept = !!plan && (plan.kept || ui.keptPlans.includes(plan.id));
      const status =
        waiting ? (waiting === 1 ? 'Waiting for you' : `${waiting} waiting for you`)
        : phase === 'new' ? 'New'
        : phase === 'talking' ? 'Answered'
        : phase === 'planning' ? (plan && plan.status !== 'planning' ? 'Thinking' : 'Planning')
        : phase === 'awaiting' ? 'Plan ready · your go'
        : phase === 'running' ? `${done} of ${tasks.length} done${live ? ` · ${live} working` : ''}`
        : phase === 'done' ? (kept ? 'Done · branch kept' : 'Ready to merge')
        : phase === 'merged' ? 'Merged'
        : phase === 'failed' ? 'Failed'
        : 'Cancelled';
      out.push({
        thread,
        title: s.title ? oneLine(s.title, 140) : 'New request',
        ts: s.ts,
        lead,
        plan,
        phase,
        waiting,
        needsYou: waiting > 0 || phase === 'awaiting',
        status,
      });
    }
    return out.sort((a, b) => (a.thread === 'lead' ? -1 : b.thread === 'lead' ? 1 : a.ts - b.ts));
  });

  get(thread: string): Request | null {
    return this.list.find((r) => r.thread === thread) ?? null;
  }

  /** ⌘] / ⌘[: the next / previous request, wrapping around. */
  step(d: 1 | -1) {
    const l = this.list;
    if (!l.length) return;
    const i = l.findIndex((r) => r.thread === ui.request);
    const next = l[(Math.max(0, i) + d + l.length) % l.length];
    ui.openRequest(next.thread);
  }
}

export const requests = new Requests();
