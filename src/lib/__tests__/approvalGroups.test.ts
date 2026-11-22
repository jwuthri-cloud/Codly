import { afterEach, describe, expect, it } from 'vitest';
import { buildSpine } from '../../home/spine';
import type { Approval } from '../gen/Approval';
import { groupSame, sameQuestion, twinsIn } from '../state/approvalGroups';
import { store } from '../state/store.svelte';

let seq = 0;
const ask = (agent: string, command: string, o: Partial<Approval> = {}): Approval => ({
  id: `ap${++seq}`, agent_id: agent, run_id: `run-${agent}`, task_id: null, call_id: null,
  subject: { kind: 'command', command, cwd: `/wt/${agent}` }, reason: null,
  choices: ['allow_once', 'deny'], risks: [], status: 'pending', decided_by: null, created_at: seq, ...o,
});

describe('requests that wait for the same answer', () => {
  it('the same command from several agents is one group, in the order asked', () => {
    const [a, b, c, d] = [ask('ana', 'pnpm install'), ask('bo', 'cargo test'), ask('cy', 'pnpm install'), ask('dee', 'pnpm install')];
    const groups = groupSame([d, c, b, a]);
    expect(groups.map((g) => [g.approval.id, g.twins.map((t) => t.id)])).toEqual([[a.id, [c.id, d.id]], [b.id, []]]);
    expect(twinsIn([a, b, c, d], c).map((t) => t.id)).toEqual([a.id, d.id]);
    expect(twinsIn([a, b, c, d], b)).toEqual([]);
  });

  it('a different risk, a different set of choices or another kind of request stands alone', () => {
    const plain = ask('ana', 'pnpm install');
    const risky = ask('bo', 'pnpm install', { risks: ['outside worktree'] });
    const session = ask('cy', 'pnpm install', { choices: ['allow_once', 'allow_session', 'deny'] });
    const answered = ask('dee', 'pnpm install', { status: 'approved' });
    const edit = ask('eve', '', { subject: { kind: 'file_change', paths: ['a.ts'], diff: null } });
    const question = ask('fay', '', { subject: { kind: 'question', questions: [] } });
    expect(groupSame([plain, risky, session, answered, edit, question]).every((g) => g.twins.length === 0)).toBe(true);
    expect([answered, edit, question].map(sameQuestion)).toEqual([null, null, null]);
    // The order of the labels does not matter: the card shows the same ones.
    const ab = ask('ana', 'make deploy', { risks: ['network', 'push'] });
    const ba = ask('bo', 'make deploy', { risks: ['push', 'network'] });
    expect(groupSame([ab, ba])).toEqual([{ approval: ab, twins: [ba] }]);
  });
});

describe('the conversation', () => {
  afterEach(() => {
    store.approvals = [];
  });

  it('shows one card for the same waiting request, and each answered one on its own', () => {
    const [a, b, other] = [ask('ana', 'pnpm install'), ask('bo', 'pnpm install'), ask('cy', 'git add -A')];
    store.approvals = [a, b, other];
    const cards = () => buildSpine('lead').flatMap((i) => (i.kind === 'approval' ? [[i.approval.id, i.twins.map((t) => t.id)]] : []));
    expect(cards()).toEqual([[a.id, [b.id]], [other.id, []]]);
    // Answered together: each agent's request is on record as its own line.
    store.approvals = [{ ...a, status: 'approved' }, { ...b, status: 'approved' }, other];
    expect(cards()).toEqual([[a.id, []], [b.id, []], [other.id, []]]);
  });
});
