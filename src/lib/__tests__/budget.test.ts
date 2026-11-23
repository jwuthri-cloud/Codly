// A used-up budget holds new work. Home says so: a mark in the top bar while it holds, and one
// toast when it happens. The mark opens Settings at the budgets.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mkAgent } from '../../office/demo/script';
import { api } from '../api';
import { MockBreaker } from '../demo/breaker';
import type { Sim } from '../demo/sim';
import type { BreakerStatus } from '../gen/BreakerStatus';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import type { JsonValue } from '../gen/serde_json/JsonValue';
import { budget, budgetChange } from '../state/budget.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';

let id = 0;
const row = (kind: unknown): EventRow => ({ id: ++id, ts: id, agent_id: null, task_id: null, run_id: null, kind: kind as EventKind });
/** A `breaker` decision, with the detail the core writes (orchestrator/breaker_run.rs). */
const breaker = (detail: JsonValue, summary = '') => row({ type: 'decision', id: `d${id}`, decision_type: 'breaker', method: 'rule', summary, detail });
const WHY = 'the team has used 156,000 work tokens, over the budget of 100,000';
const reached = () => breaker({ action: 'budget', level: 'constrained', trip: 'floor_budget', reason: WHY }, `Budget reached: ${WHY}. No new task starts.`);
const cleared = () => breaker({ action: 'budget_cleared', level: 'healthy' }, 'The budget has room again: waiting tasks start.');
const status = (hold: string | null): BreakerStatus => ({ used: { tokens: 0, cost_usd: 0 }, today: { tokens: 0, cost_usd: 0 }, hold, agents: [] });

afterEach(() => {
  vi.restoreAllMocks();
  for (const t of toasts.list) toasts.dismiss(t.id);
  budget.setHold(null);
  budget.reveal = false;
  ui.settingsOpen = false;
  ui.settingsTab = null;
});

describe('the budget mark', () => {
  it('shows from the moment a budget is reached until work goes on, with one toast when it happens', () => {
    expect(budget.hold).toBeNull();
    budget.observe(reached());
    expect(budget.hold).toBe(WHY);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body, t.action?.label])).toEqual([['attention', 'Budget reached', `No new task starts: ${WHY}.`, 'Open Settings']]);

    budget.observe(cleared());
    expect(budget.hold).toBeNull();
    expect(toasts.list).toEqual([]); // the toast does not say a budget holds work that has room again
  });

  it('takes the toast back when Settings reads that nothing holds (Reset counters), and says the next time anew', async () => {
    budget.observe(reached());
    budget.setHold(null); // the budget section of Settings read the core
    expect(toasts.list).toEqual([]);

    budget.observe(reached());
    expect(toasts.list).toHaveLength(1);
    vi.spyOn(api, 'breakerStatus').mockResolvedValue(status(null));
    await budget.refresh();
    expect(toasts.list).toEqual([]);
  });

  it('shows one toast when the core says a reached budget again', () => {
    budget.observe(reached());
    budget.observe(reached());
    expect(toasts.list).toHaveLength(1);
  });

  it('is not moved by an agent over a limit of its own, or by any other decision', () => {
    const steered = breaker({ level: 'steering', from: 'healthy', action: 'steered', trip: 'repeated_tool', reason: 'it ran the same tool call 8 times in a row (limit 8)' });
    const routed = row({ type: 'decision', id: 'd0', decision_type: 'route', method: 'rule', summary: 't1 → Ada', detail: { action: 'budget' } });
    expect([steered, routed, row({ type: 'notice', text: 'Budget reached' })].map(budgetChange)).toEqual([undefined, undefined, undefined]);
    budget.observe(reached());
    for (const ev of [steered, routed]) budget.observe(ev);
    expect(budget.hold).toBe(WHY);
    expect(toasts.list).toHaveLength(1);
  });

  it('reads what holds from the core: a budget used up before you quit still shows, without a toast', async () => {
    const core = vi.spyOn(api, 'breakerStatus').mockResolvedValue(status(WHY));
    await budget.refresh();
    expect(budget.hold).toBe(WHY);
    expect(toasts.list).toEqual([]);
    core.mockResolvedValue(status(null));
    await budget.refresh();
    expect(budget.hold).toBeNull();
    // The core does not answer: the mark keeps what it last knew.
    budget.setHold(WHY);
    core.mockRejectedValue(new Error('not connected'));
    vi.spyOn(console, 'error').mockImplementation(() => {});
    await budget.refresh();
    expect(budget.hold).toBe(WHY);
  });

  it('opens Settings on the Safety tab, at the budgets (the toast’s button does the same)', () => {
    budget.openSettings();
    expect([ui.settingsOpen, ui.settingsTab, budget.reveal]).toEqual([true, 'safety', true]);
    ui.settingsOpen = false;
    ui.settingsTab = null;
    budget.reveal = false;
    budget.observe(reached());
    toasts.list[0].action!.run();
    expect([ui.settingsOpen, ui.settingsTab, budget.reveal]).toEqual([true, 'safety', true]);
  });
});

describe('the demo’s breaker', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('says a reached budget once, and once that work goes on, in the words the app listens for', () => {
    const worker = mkAgent('ada', 'Ada', 'claude', 'worker', 1, 'working');
    const ada = { ...worker, usage: { ...worker.usage, input_tokens: 150_000, output_tokens: 6_000 } };
    const said: EventRow[] = [];
    const sim = { agents: new Map([[ada.id, ada]]), tasks: new Map(), decision: (type: string, method: string, summary: string, detail: JsonValue) => said.push(row({ type: 'decision', id: `d${id}`, decision_type: type, method, summary, detail })) } as unknown as Sim;
    const mock = new MockBreaker(sim);
    const budgets = () => said.map(budgetChange).filter((c) => c !== undefined);

    mock.set({ ...mock.cfg, token_budget: 100_000 });
    vi.advanceTimersByTime(12_000); // three beats: said at the first one only
    expect(budgets()).toEqual(['the team has used 156,000 work tokens, over the budget of 100,000']);
    expect(mock.status().hold).toBe(budgets()[0]); // the same words as Settings shows

    mock.set({ ...mock.cfg, token_budget: 0 });
    vi.advanceTimersByTime(8_000);
    expect(budgets()).toEqual(['the team has used 156,000 work tokens, over the budget of 100,000', null]);
  });
});
