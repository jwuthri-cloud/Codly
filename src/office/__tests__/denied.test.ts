// A request in the office where nothing asks "May I": the ticker while it waits for you, and
// the agent's line after you said no. The verb-first texts of "May I …?" do not read there.
import { describe, expect, it } from 'vitest';
import type { Approval } from '../../lib/gen/Approval';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import { AGENTS } from '../demo/script';
import type { OfficeModel } from '../model';
import { sentence } from '../narrate';
import { approvalWaits, deniedLine, emptyModel, reduce, syncAgents, waitingLine } from '../reducer';

const T0 = 1_800_000_000_000;
let id = 0;
const row = (t: number, agent: string, kind: EventKind): EventRow => ({ id: ++id, ts: T0 + t, agent_id: agent, task_id: null, run_id: null, kind });
const live = { replay: false };

// Ada runs Claude Code, Bo runs Codex.
function office(): OfficeModel {
  const m = emptyModel();
  syncAgents(m, AGENTS.map((a) => ({ ...a, status: a.role === 'lead' ? 'idle' : 'working' })), T0);
  return m;
}
const request = (agent: string, subject: Approval['subject']): Approval =>
  ({ id: 'ap', agent_id: agent, run_id: 'r', task_id: null, call_id: null, subject, reason: null, choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: 0 });

const question: Approval['subject'] = { kind: 'question', questions: [{ question: 'Which DB?' }] };
const permissions: Approval['subject'] = { kind: 'permissions', detail: {} };
const command: Approval['subject'] = { kind: 'command', command: 'rm -rf dist', cwd: null };
const edit: Approval['subject'] = { kind: 'file_change', paths: ['src/theme.ts'], diff: null };
const plan: Approval['subject'] = { kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add the line' } };
const host: Approval['subject'] = { kind: 'tool', name: 'SandboxNetworkAccess', input: { host: 'example.com' } };

/** The ticker line of the request, then the agent's line after you said no to it. */
function askedThenDenied(agent: string, subject: Approval['subject']): [string, string] {
  const m = office();
  const asked = reduce(m, row(1000, agent, { type: 'approval_requested', approval: request(agent, subject) }), live);
  const ticker = asked.flatMap((f) => (f.t === 'ticker' ? [[f.text, f.code].filter(Boolean).join(' ')] : []));
  reduce(m, row(2000, agent, { type: 'approval_resolved', approval_id: 'ap', status: 'denied', by: 'user' }), live);
  return [ticker.join(' | '), sentence(m.agents.get(agent)!, T0 + 2100).text];
}

describe('a request you said no to, in the office', () => {
  it('a skipped question and denied permissions read as that, not as "Denied ask a question"', () => {
    expect(askedThenDenied('a-ada', question)).toEqual(['Ada · Waiting for you: its question', 'Skipped its question']);
    expect(askedThenDenied('a-ada', permissions)).toEqual(['Ada · Waiting for you: more permissions', 'Denied: more permissions']);
  });

  it('a command is denied by its first words; every other request says what the agent wanted to do', () => {
    expect(askedThenDenied('a-ada', command)).toEqual(['Ada · Waiting for you: rm -rf dist', 'Denied rm -rf dist']);
    expect(askedThenDenied('a-ada', edit)).toEqual(['Ada · Waiting for you: edit theme.ts', 'Denied: edit theme.ts']);
    expect(askedThenDenied('a-ada', host)).toEqual(['Ada · Waiting for you: reach example.com', 'Denied: reach example.com']);
  });

  it('a Claude agent’s plan is sent back, not denied; from another CLI that request is a tool by its name', () => {
    expect(askedThenDenied('a-ada', plan)).toEqual(['Ada · Waiting for you: read its plan', 'Plan sent back']);
    expect(askedThenDenied('a-bo', plan)).toEqual(['Bo · Waiting for you: use ExitPlanMode', 'Denied: use ExitPlanMode']);
  });

  it('the line of each kind, and what the ticker says while it waits', () => {
    const line = (s: Approval['subject']) => deniedLine(request('a-ada', s), 'claude');
    expect(line(question)).toEqual({ text: 'Skipped its question', code: null });
    expect(line(permissions)).toEqual({ text: 'Denied: more permissions', code: null });
    expect(line(command)).toEqual({ text: 'Denied', code: 'rm -rf dist' });
    expect(line(edit)).toEqual({ text: 'Denied:', code: 'edit theme.ts' });
    expect(line(plan)).toEqual({ text: 'Plan sent back', code: null });
    expect(approvalWaits(request('a-ada', question), 'claude')).toBe('its question');
    expect(approvalWaits(request('a-ada', permissions), null)).toBe('more permissions');
    expect(approvalWaits(request('a-ada', command), 'claude')).toBe('rm -rf dist');
  });

  it('a request the office never saw asked is only known as denied; an approved one leaves no line', () => {
    const m = office();
    reduce(m, row(1000, 'a-ada', { type: 'approval_resolved', approval_id: 'unseen', status: 'denied', by: 'user' }), live);
    expect(sentence(m.agents.get('a-ada')!, T0 + 1100).text).toBe('Denied');
    const n = office();
    reduce(n, row(1000, 'a-ada', { type: 'approval_requested', approval: request('a-ada', question) }), live);
    reduce(n, row(2000, 'a-ada', { type: 'approval_resolved', approval_id: 'ap', status: 'approved', by: 'user' }), live);
    expect(n.agents.get('a-ada')!.outcome).toBeNull();
  });
});

describe('an agent that waits for you, in the Sims panel', () => {
  /** Its "Now" row while the request waits; the reason of its moodlet is the same words after the colon. */
  const waiting = (agent: string, subject: Approval['subject']) => {
    const m = office();
    reduce(m, row(1000, agent, { type: 'approval_requested', approval: request(agent, subject) }), live);
    return waitingLine(m.agents.get(agent)!.approval!);
  };

  it('names a question, a plan and a permissions request as the ticker does', () => {
    expect(waiting('a-ada', question)).toBe('Waiting for you: its question');
    expect(waiting('a-ada', plan)).toBe('Waiting for you: read its plan');
    expect(waiting('a-bo', permissions)).toBe('Waiting for you: more permissions');
    expect(waiting('a-ada', command)).toBe('Waiting for you: rm -rf dist');
  });

  it('says only that it waits when the office knows nothing more', () => {
    expect(waitingLine({ waits: '' })).toBe('Waiting for you');
  });
});
