// Send back from the Changes room to agents at work: a 1:1 agent's comments go to its 1:1 chat,
// and a running plan's comments go to the workers of the tasks that wrote their files, with only
// the rest in a new request. A refused call keeps its comments on the diff.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { Agent } from '../gen/Agent';
import type { Plan } from '../gen/Plan';
import type { Task } from '../gen/Task';
import { sendBack } from '../state/actions';
import { review, reviewKey } from '../state/review.svelte';
import { store, type TaskMark } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';
import { fileAnchor } from '../util/review';

const shown = () => toasts.list.map((t) => [t.kind, t.title, t.body]);
const paths = (key: string) => review.comments(key).map((c) => c.path);

afterEach(() => {
  vi.restoreAllMocks();
  for (const t of [...toasts.list]) toasts.dismiss(t.id);
  review.clear(reviewKey({ kind: 'agent', id: 's1' }));
  review.clear(reviewKey({ kind: 'plan', id: 'p1' }));
  store.tasks = [];
  store.plans = [];
  store.agents = [];
  store.taskMarks = {};
  store.live = {};
  for (const o of [...store.outbox]) store.dropOutgoing(o.id);
  ui.draftRequests = [];
  ui.composerSeed = null;
  ui.openRequest('lead');
});

describe('Send on a 1:1 agent’s diff', () => {
  const target = { kind: 'agent' as const, id: 's1' };
  const key = reviewKey(target);
  const message = 'Review comments (1):\n\n1. `src/a.ts` (the whole file)\n   Split it.';

  it('sends the comments as one message of yours in its 1:1 chat; one written meanwhile stays', async () => {
    store.agents = [{ id: 's1', name: 'Eli', role: 'solo', worktree: '/wt/eli' } as Agent];
    let took!: () => void;
    const chat = vi.spyOn(api, 'sendChat').mockReturnValue(new Promise<void>((resolve) => (took = resolve)));
    review.comment(key, fileAnchor('src/a.ts'), 'Split it.');
    const sending = sendBack(target, ['src/a.ts']);
    expect(chat).toHaveBeenCalledWith('agent:s1', message);
    // It shows in the chat at once, as a message you typed does.
    expect(store.outbox.map((o) => [o.thread, o.text])).toEqual([['agent:s1', message]]);

    review.comment(key, fileAnchor('b.txt'), 'And this one.');
    took();
    expect(await sending).toBe(true);
    expect(paths(key)).toEqual(['b.txt']);
    expect(shown()).toEqual([['success', '1 comment sent to Eli', undefined]]);
    expect(ui.draftRequests).toEqual([]);
  });

  it('a refused message is not left in the chat, and the comments stay on the diff', async () => {
    store.agents = [{ id: 's1', name: 'Eli', role: 'solo', worktree: '/wt/eli' } as Agent];
    vi.spyOn(api, 'sendChat').mockRejectedValue(new Error('Eli was let go'));
    review.comment(key, fileAnchor('src/a.ts'), 'Split it.');
    expect(await sendBack(target, ['src/a.ts'])).toBe(false);
    expect(paths(key)).toEqual(['src/a.ts']);
    expect(store.outbox).toEqual([]);
    expect(shown()).toEqual([['error', 'Couldn’t send your comments to Eli', 'Eli was let go — your comments stay on the diff.']]);
  });
});

describe('Send back on the diff of a plan that still runs', () => {
  const target = { kind: 'plan' as const, id: 'p1' };
  const key = reviewKey(target);
  const plan = { id: 'p1', summary: 'Add search', status: 'running', thread: 'lead', integration_branch: 'codly/ab12/integration', merged_sha: null } as Plan;
  const tasks = [
    { id: 't1', key: 't1', title: 'Index', plan_id: 'p1', status: 'running', assignee: 'w1' },
    { id: 't2', key: 't2', title: 'Box', plan_id: 'p1', status: 'reviewing', assignee: 'w2' },
    { id: 't3', key: 't3', title: 'Docs', plan_id: 'p1', status: 'done', assignee: 'w3' },
  ] as Task[];
  const committed = (files: string[]): TaskMark[] => [{ kind: 'committed', ts: 1, eventId: 1, agentId: null, text: '', ok: true, files }];
  const all = ['a.ts', 'b.ts', 'c.md'];

  const setUp = () => {
    store.plans = [plan];
    store.tasks = tasks;
    store.agents = [{ id: 'w1', name: 'Ada' }, { id: 'w2', name: 'Bo' }, { id: 'w3', name: 'Cy' }] as Agent[];
    store.taskMarks = { t1: committed(['a.ts']), t2: committed(['b.ts']), t3: committed(['c.md']) };
    review.comment(key, fileAnchor('a.ts'), 'Rename the index.');
    review.comment(key, { path: 'a.ts', line: 3, side: 'new', quote: '+const i = 0;' }, 'Why zero?');
    review.comment(key, fileAnchor('b.ts'), 'Wider box.');
    review.comment(key, fileAnchor('c.md'), 'One more example.');
  };

  it('gives each running task its comments, one call per task, and puts the done task’s in a new request', async () => {
    setUp();
    const core = vi.spyOn(api, 'sendBackTask').mockImplementation(async (id) => ({ ...tasks.find((t) => t.id === id)!, status: 'changes_requested' }));
    expect(await sendBack(target, all)).toBe(true);
    expect(core.mock.calls.map(([id, text]) => [id, text.match(/`[^`]+`/g)])).toEqual([
      ['t1', ['`a.ts`', '`a.ts:3`']],
      ['t2', ['`b.ts`']],
    ]);
    // The new request names only the comment no worker took.
    expect(ui.draftRequests).toHaveLength(1);
    expect(ui.composerSeed?.text).toContain('Review comments (1):');
    expect(ui.composerSeed?.text).toContain('`c.md`');
    expect(ui.composerSeed?.text).not.toContain('a.ts');
    expect(paths(key)).toEqual(['c.md']);
    expect(shown()).toEqual([['success', 'Comments sent back', '2 comments to Ada (t1), 1 to Bo (t2); 1 is in a new request']]);
  });

  it('a refused call keeps its comments on the diff, and the toast says why', async () => {
    setUp();
    vi.spyOn(api, 'sendBackTask').mockImplementation(async (id) => {
      if (id === 't1') throw new Error('t1 is merging');
      return { ...tasks.find((t) => t.id === id)!, status: 'changes_requested' };
    });
    expect(await sendBack(target, all)).toBe(true);
    expect(paths(key).sort()).toEqual(['a.ts', 'a.ts', 'c.md']);
    expect(shown()).toEqual([
      ['error', 'Couldn’t send your comments back to t1', 't1 is merging — your comments stay on the diff.'],
      ['success', 'Comments sent back', '1 comment to Bo (t2); 1 is in a new request'],
    ]);
  });

  it('a file a done task committed is that task’s, whichever comes first: a task at work that touched it too does not get its comments', async () => {
    for (const order of [['t1', 't3'], ['t3', 't1']]) {
      setUp();
      store.tasks = order.map((id) => tasks.find((t) => t.id === id)!);
      // Ada (t1, at work) touched a.ts too, in a commit and in her worktree now; Cy's merged t3 wrote the lines on screen.
      store.taskMarks = { t1: committed(['a.ts']), t3: committed(['a.ts', 'c.md']) };
      store.live = { w1: { files: { 'a.ts': { modified: true } } } } as unknown as typeof store.live;
      const core = vi.spyOn(api, 'sendBackTask');
      expect(await sendBack(target, all)).toBe(true);
      expect(core, order.join(',')).not.toHaveBeenCalled();
      expect(ui.composerSeed?.text).toContain('`a.ts:3`');
      review.clear(key);
      vi.restoreAllMocks();
    }
  });

  it('a task at work takes a file no done task names, even one its agent only has in its worktree now', async () => {
    setUp();
    store.taskMarks = { t3: committed(['c.md']) };
    store.live = { w1: { files: { 'a.ts': { modified: true } } }, w3: { files: { 'b.ts': { modified: true } } } } as unknown as typeof store.live;
    const core = vi.spyOn(api, 'sendBackTask').mockImplementation(async (id) => ({ ...tasks.find((t) => t.id === id)!, status: 'changes_requested' }));
    expect(await sendBack(target, all)).toBe(true);
    // Cy's live b.ts is not of the done t3 (an agent's files now are of its current task): it goes to the request.
    expect(core.mock.calls.map(([id]) => id)).toEqual(['t1']);
    expect(paths(key).sort()).toEqual(['b.ts', 'c.md']);
  });

  it('with no task at work, everything goes to the new request as before', async () => {
    setUp();
    store.tasks = tasks.map((t) => ({ ...t, status: 'done' }));
    const core = vi.spyOn(api, 'sendBackTask');
    expect(await sendBack(target, all)).toBe(true);
    expect(core).not.toHaveBeenCalled();
    expect(ui.composerSeed?.text).toContain('Review comments (4):');
    expect(paths(key)).toHaveLength(4);
  });
});
