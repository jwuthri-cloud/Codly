// Toasts that point at a request: none while its card is in front of you, and their words are
// sentences (a plan is "its plan", a tool is named after "use").
import { afterEach, describe, expect, it } from 'vitest';
import { mkAgent } from '../../office/demo/script';
import type { Approval } from '../gen/Approval';
import type { EventKind } from '../gen/EventKind';
import type { EventRow } from '../gen/EventRow';
import { asks, asksTitle, cardInView, installNotifications, type Looking } from '../state/notify';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';

const home: Looking = { room: 'home', mode: 'blend', thread: 'agent:eli', focused: true };
const plan: Approval['subject'] = { kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add the line' } };

describe('whether a request’s card is in front of you', () => {
  const threads = ['lead:b', 'agent:eli'];

  it('is when Home shows a conversation that holds the card, and the window has the focus', () => {
    expect(cardInView(home, threads)).toBe(true);
    expect(cardInView({ ...home, thread: 'lead:b' }, threads)).toBe(true);
    expect(cardInView({ ...home, mode: 'work' }, threads)).toBe(true);
  });

  it('is not in another conversation, in another room, behind the full office or the map, or in a window without the focus', () => {
    expect(cardInView({ ...home, thread: 'lead' }, threads)).toBe(false);
    expect(cardInView({ ...home, room: 'plan' }, threads)).toBe(false);
    expect(cardInView({ ...home, mode: 'office' }, threads)).toBe(false);
    expect(cardInView({ ...home, mode: 'map' }, threads)).toBe(false);
    expect(cardInView({ ...home, focused: false }, threads)).toBe(false);
  });
});

describe('the words of a toast that points at a request', () => {
  it('a Claude agent’s plan is shown to you: the name of its tool is not', () => {
    expect([asksTitle('Eli', plan, 'claude'), asks(plan, 'claude')]).toEqual(['Eli shows you its plan', 'Read it in the conversation']);
  });

  it('the same request from another CLI, or from an agent Codly does not know, is a tool by its name', () => {
    for (const provider of ['codex', 'gemini', null, undefined] as const)
      expect([asksTitle('Eli', plan, provider), asks(plan, provider)], String(provider)).toEqual(['Eli needs your approval', 'Wants to use ExitPlanMode']);
  });

  it('every other request is a sentence: no bare kind, and a tool by its name', () => {
    expect(asksTitle('Eli', { kind: 'tool', name: 'WebFetch', input: {} }, 'claude')).toBe('Eli needs your approval');
    expect(asks({ kind: 'tool', name: 'WebFetch', input: {} }, 'claude')).toBe('Wants to use WebFetch');
    expect(asks({ kind: 'file_change', paths: ['README.md'], diff: null }, 'codex')).toBe('Wants to change README.md');
    expect(asks({ kind: 'permissions', detail: {} }, 'codex')).toBe('Wants to get more permissions');
    expect(asks({ kind: 'command', command: 'pnpm test', cwd: null }, 'claude')).toBe('pnpm test');
    expect([asksTitle('Eli', { kind: 'question', questions: [] }, 'claude'), asks({ kind: 'question', questions: [] }, 'claude')]).toEqual(['Eli has a question', 'Answer it in the conversation']);
  });
});

describe('a request arrives', () => {
  let eid = 0;
  const apply = (...evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
  const asked = (id: string): EventRow => ({
    id: ++eid, ts: eid, agent_id: 'eli', task_id: null, run_id: 'r1',
    kind: { type: 'approval_requested', approval: { id, agent_id: 'eli', run_id: 'r1', task_id: null, call_id: null, subject: plan, reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: eid } } as EventKind,
  });

  afterEach(() => {
    for (const t of toasts.list) toasts.dismiss(t.id);
    store.agents = [];
    store.approvals = [];
    store.events = [];
    ui.thread = 'lead';
    ui.room = 'home';
    ui.mode = 'blend';
  });

  it('no toast while you read the conversation that holds its card; a toast from anywhere else', () => {
    const off = installNotifications();
    store.agents = [mkAgent('eli', 'Eli', 'claude', 'solo', 0, 'working')];
    ui.room = 'home';
    ui.mode = 'blend';
    ui.thread = 'agent:eli';
    apply(asked('ap1'));
    expect(toasts.list).toHaveLength(0); // the card is the notice
    ui.thread = 'lead';
    apply(asked('ap2'));
    expect(toasts.list.map((t) => [t.title, t.body])).toEqual([['Eli shows you its plan', 'Read it in the conversation']]);
    off();
  });
});
