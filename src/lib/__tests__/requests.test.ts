// "N waiting for you" on a request's tab and on its plan header counts what its conversation
// shows you as waiting: approval cards and questions. Ask-me cards are not in the conversation:
// the Ask me button counts those.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { buildSpine } from '../../home/spine';
import { mkAgent } from '../../office/demo/script';
import type { Approval } from '../gen/Approval';
import type { Blocker } from '../gen/Blocker';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Plan } from '../gen/Plan';
import type { Task } from '../gen/Task';
import { blockers } from '../state/blockers.svelte';
import { requests } from '../state/requests.svelte';
import { store } from '../state/store.svelte';

const asked = (thread: string, text: string, ts: number): ChatMessage => ({ id: `${thread}-${ts}`, thread, author: 'user', text, attachment: null, ts });
const plan = (id: string, thread: string, status: Plan['status']) => ({ id, thread, status, request: 'Add dark mode', summary: 'Dark mode', merged_sha: null, kept: false, created_at: 1 }) as Plan;
const task = (id: string, planId: string, status: Task['status']) => ({ id, plan_id: planId, key: id, title: id, status, assignee: null }) as Task;
let seq = 0;
/** A worker's request on a task, asked in its own worktree (`/wt/<agent>`). */
const ask = (agent: string, taskId: string, subject: Approval['subject']): Approval => ({
  id: `ap${++seq}`, agent_id: agent, run_id: `run-${agent}`, task_id: taskId, call_id: null, subject, reason: null,
  choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: seq,
});
const command = (agent: string, taskId: string, cmd: string) => ask(agent, taskId, { kind: 'command', command: cmd, cwd: `/wt/${agent}` });
const question = (agent: string, taskId: string) => ask(agent, taskId, { kind: 'question', questions: { questions: [{ question: 'Keep the fallback?', options: ['Keep it', 'Drop it'] }] } });
const askMe = (id: string, thread: string): Blocker => ({
  id, workspace_id: null, agent_id: 'ada', task_id: 't1', thread, title: 'Which token?', body: 'I need a deploy token.', severity: 'blocking', status: 'open',
  answer: null, created_at: 5, updated_at: 5, answered_at: null, reminded_at: null,
});
/** The waiting cards the conversation of `thread` shows. */
const cards = (thread: string) => buildSpine(thread).filter((i) => i.kind === 'approval' && i.approval.status === 'pending').length;

beforeEach(() => {
  store.agents = [mkAgent('lead1', 'Lead', 'claude', 'lead', 0, 'idle', 'lead'), ...['ada', 'bo'].map((id) => ({ ...mkAgent(id, id, 'codex', 'worker', 1, 'working'), worktree: `/wt/${id}` }))];
  store.chats = [asked('lead', 'Add dark mode', 1), asked('lead:b', 'Fix the footer', 2)];
  store.plans = [plan('p1', 'lead', 'running'), plan('p2', 'lead:b', 'running')];
  store.tasks = [task('t1', 'p1', 'running'), task('t2', 'p1', 'running'), task('t3', 'p1', 'pending'), task('u1', 'p2', 'running')];
});

afterEach(() => {
  store.agents = [];
  store.chats = [];
  store.plans = [];
  store.tasks = [];
  store.approvals = [];
  blockers.list = [];
});

describe('what waits for you in a request', () => {
  it('is what its conversation shows: one per card, and the same command from two agents is one card', () => {
    store.approvals = [command('ada', 't1', 'pnpm install'), command('bo', 't2', 'pnpm install'), question('ada', 't1'), command('bo', 'u1', 'cargo test')];
    const [first, second] = [requests.get('lead')!, requests.get('lead:b')!];
    expect([first.waiting, second.waiting]).toEqual([2, 1]);
    expect([first.waiting, second.waiting]).toEqual([cards('lead'), cards('lead:b')]);
    expect([first.status, second.status]).toEqual(['2 waiting for you', 'Waiting for you']);
    expect([first.needsYou, second.needsYou]).toEqual([true, true]);
  });

  it('does not count an Ask-me card: the conversation does not show it, the Ask me button does', () => {
    blockers.list = [askMe('b1', 'lead'), askMe('b2', 'lead')];
    const r = requests.get('lead')!;
    expect(cards('lead')).toBe(0);
    expect([r.waiting, r.needsYou, r.status]).toEqual([0, false, '0 of 3 done · 2 working']);
    expect(blockers.blocking).toHaveLength(2); // counted where they are shown
    // With one approval card in the conversation it says one, not three.
    store.approvals = [command('ada', 't1', 'pnpm install')];
    expect([requests.get('lead')!.waiting, requests.get('lead')!.status]).toEqual([1, 'Waiting for you']);
  });

  it('a finished plan with nothing to answer in its conversation says it is finished', () => {
    store.plans = [plan('p1', 'lead', 'done')];
    store.tasks = ['t1', 't2', 't3'].map((id) => task(id, 'p1', 'done'));
    blockers.list = [askMe('b1', 'lead'), askMe('b2', 'lead')];
    const r = requests.get('lead')!;
    expect([r.waiting, r.needsYou, r.phase, r.status]).toEqual([0, false, 'done', 'Ready to merge']);
  });
});
