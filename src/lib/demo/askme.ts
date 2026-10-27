// The mock "Ask me" backend (pnpm dev and the public demo): a few cards a team would raise,
// answered through the same three commands as the real core. Answering the blocking card on the
// board's blocked checkout task moves that task back to Ready, like the real "respond & unblock".
import type { Blocker } from '../gen/Blocker';
import type { Sim } from './sim';

const DAY = 86_400_000;

export class MockAskMe {
  private cards = new Map<string, Blocker>();
  private seeded = false;

  constructor(private sim: Sim) {}

  private seed() {
    const sim = this.sim;
    if (this.seeded || !sim.workspace) return;
    this.seeded = true;
    const now = sim.now();
    const agents = [...sim.agents.values()];
    const lead = agents.find((a) => a.role === 'lead') ?? agents[0];
    const worker = agents.find((a) => a.role !== 'lead') ?? lead;
    const blockedTask = [...sim.tasks.values()].find((t) => t.status === 'blocked');
    const card = (c: Partial<Blocker> & Pick<Blocker, 'id' | 'title' | 'body' | 'severity' | 'created_at'>): Blocker => ({
      workspace_id: sim.workspace!.id, agent_id: lead?.id ?? 'agent', task_id: null, thread: 'lead', status: 'open', answer: null,
      updated_at: c.created_at, answered_at: null, reminded_at: null, ...c,
    });
    const seeds: Blocker[] = [
      card({
        id: 'ask-stripe', agent_id: worker?.id ?? 'agent', task_id: blockedTask?.id ?? null, severity: 'blocking', created_at: now - 3 * DAY - 2 * 3_600_000,
        reminded_at: now - DAY, updated_at: now - DAY,
        title: 'Which Stripe key should the checkout tests use?',
        body: 'The e2e test needs a Stripe test key in `.env.test`, and agents cannot read your keychain. Paste a **test** key (it starts with `sk_test_`), or tell me to mock Stripe in the test instead. Either way the test stops being flaky; mocking is faster but checks less.',
      }),
      card({
        id: 'ask-flag', severity: 'blocking', created_at: now - 40 * 60_000, title: 'Ship the dark-mode toggle to everyone, or behind a flag?',
        body: 'The toggle is ready. If it ships to everyone, users see it on the next deploy. Behind a flag, you turn it on yourself when you are ready. Reply with “everyone” or “flag”, plus anything you want changed first.',
      }),
      card({
        id: 'ask-fyi', agent_id: worker?.id ?? 'agent', severity: 'heads_up', created_at: now - 6 * 3_600_000, title: 'The e2e suite now takes about 4 minutes',
        body: 'Heads-up, no action needed today: the checkout e2e suite grew from 90 seconds to about 4 minutes. It still passes. Say so if you want it split up.',
      }),
    ];
    for (const s of seeds) this.cards.set(s.id, s);
  }

  list(): Blocker[] {
    this.seed();
    return [...this.cards.values()].sort((a, b) => a.created_at - b.created_at);
  }

  /** Respond & unblock (answer), or skip/read (dismiss). */
  close(id: string, status: 'answered' | 'dismissed', answer: string | null): Blocker {
    const c = this.cards.get(id);
    if (!c) throw new Error(`Unknown question ${id}`);
    if (c.status !== 'open') throw new Error(`That question is already ${c.status}.`);
    if (status === 'answered' && !answer?.trim()) throw new Error('Write an answer first.');
    const now = this.sim.now();
    const next: Blocker = { ...c, status, answer: answer?.trim() ?? null, updated_at: now, answered_at: now };
    this.cards.set(id, next);
    if (c.severity === 'blocking' && c.task_id && this.sim.tasks.get(c.task_id)?.status === 'blocked')
      this.sim.patchTask(c.task_id, { status: 'ready', status_reason: status === 'answered' ? 'you answered; the worker will pick it up' : 'you skipped the question; the worker will use its best judgment' });
    this.sim.emit({ type: 'blocker_changed', blocker: next }, { agent: c.agent_id, task: c.task_id });
    return next;
  }
}
