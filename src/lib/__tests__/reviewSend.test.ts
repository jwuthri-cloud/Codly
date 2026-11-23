// Which review comments leave their diff when they are sent: the ones the worker got, or the
// ones the message you sent still names. A comment written or changed meanwhile, and one you
// took out of the message, stay.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { Agent } from '../gen/Agent';
import type { Plan } from '../gen/Plan';
import type { Task } from '../gen/Task';
import { sendBack, sendChat, waitingRequest } from '../state/actions';
import { review, reviewKey } from '../state/review.svelte';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';
import { parseDiff } from '../util/diff';
import { fileAnchor, lineAnchor, reviewMessage, sentIn, type ReviewComment } from '../util/review';

const A = `diff --git a/src/a.ts b/src/a.ts
index 111..222 100644
--- a/src/a.ts
+++ b/src/a.ts
@@ -10,2 +10,2 @@ export function a() {
   const keep = 1;
-  const old = 2;
+  const fresh = 2;
`;
const [, old, fresh] = parseDiff(A)[0].hunks[0].lines;

describe('the comments a sent message names', () => {
  const onOld: ReviewComment = { ...lineAnchor('src/a.ts', old), text: 'Why remove it?' };
  const onNew: ReviewComment = { ...lineAnchor('src/a.ts', fresh), text: 'Name it better.' };
  const onFile: ReviewComment = { ...fileAnchor('src/a.ts'), text: 'Split this file.' };
  const all = [onOld, onNew, onFile];
  // 1. the file, 2. the removed line 11, 3. the new line 11.
  const message = reviewMessage(all, ['src/a.ts']);

  it('all of them in the message as Send back built it, also with your words around them and in them', () => {
    expect(sentIn(message, all)).toEqual(all);
    expect(sentIn(`First of all:\n\n${message.replace('Name it better.', 'Call it `next`.')}\n\nThanks.`, all)).toEqual(all);
  });

  it('not one whose lines you took out: each is named by its file and line with its quoted line', () => {
    // The removed line and the new line have the same number.
    expect(sentIn(message.slice(0, message.indexOf('\n\n3. ')), all)).toEqual([onOld, onFile]);
    expect(sentIn(message.replace(/\n\n2\. [\s\S]*(?=\n\n3\. )/, ''), all)).toEqual([onNew, onFile]);
    expect(sentIn(message.replace(/1\. [\s\S]*(?=2\. )/, ''), all)).toEqual([onOld, onNew]);
  });

  it('none in a message of your own, also one that points at the same line', () => {
    expect(sentIn('Something else', all)).toEqual([]);
    expect(sentIn('`src/a.ts:11` looks wrong to me, and so does `src/a.ts`', all)).toEqual([]);
  });
});

describe('review comments that are sent leave their diff', () => {
  const task = { id: 't1', key: 't1', title: 'Add a', plan_id: 'p1', status: 'reviewing', assignee: 'w1', branch: 'codly/ab12/t1' } as Task;
  const plan = { id: 'p1', summary: 'Add two files', thread: 'lead', integration_branch: 'codly/ab12/integration', merged_sha: null } as Plan;
  const target = { kind: 'task' as const, id: 't1' };
  const key = reviewKey(target);
  const texts = () => review.comments(key).map((c) => c.text);

  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
    review.clear(key);
    store.tasks = [];
    store.plans = [];
    store.agents = [];
    for (const o of [...store.outbox]) store.dropOutgoing(o.id);
    ui.draftRequests = [];
    ui.composerSeed = null;
    ui.openRequest('lead');
  });

  /** Two comments on the diff of a task whose work is finished, and Send back: a new request holds them. */
  const handed = async () => {
    vi.spyOn(api, 'sendChat').mockResolvedValue();
    store.tasks = [{ ...task, status: 'done' }];
    store.plans = [plan];
    review.comment(key, lineAnchor('src/a.ts', fresh), 'Call it `next`.');
    review.comment(key, fileAnchor('b.txt'), 'Why two lines?');
    await sendBack(target, ['src/a.ts', 'b.txt']);
    return { draft: ui.request, message: ui.composerSeed!.text };
  };

  it('a comment you write or change while Send back is on its way to the worker stays', async () => {
    store.tasks = [task];
    store.agents = [{ id: 'w1', name: 'Ana' } as Agent];
    let took!: (t: Task) => void;
    const core = vi.spyOn(api, 'sendBackTask').mockReturnValue(new Promise<Task>((resolve) => (took = resolve)));
    review.comment(key, lineAnchor('src/a.ts', fresh), 'Call it `next`.');
    review.comment(key, fileAnchor('b.txt'), 'Why two lines?');
    const sending = sendBack(target, ['src/a.ts', 'b.txt']);
    expect(core).toHaveBeenCalledTimes(1);

    // The core has not answered yet: one comment more, and one of the two changed.
    review.comment(key, fileAnchor('src/a.ts'), 'And add a test.');
    review.comment(key, fileAnchor('b.txt'), 'One line is enough.');
    took({ ...task, status: 'changes_requested' });
    expect(await sending).toBe(true);

    // The worker got the two as they were: what it did not get is still on the diff.
    expect(texts()).toEqual(['And add a test.', 'One line is enough.']);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toContainEqual(['success', 't1 sent back to Ana', '2 comments']);
  });

  it('a message you changed before you sent it takes its comments with it', async () => {
    const { draft, message } = await handed();
    sendChat(draft, `Start with the names.\n\n${message.replace('Call it `next`.', 'Call it `after`, like the others.')}`);
    expect(texts()).toEqual([]);
    expect(waitingRequest(key)).toBeNull();
  });

  it('a comment you took out of the message stays on the diff, and so does one you changed there since', async () => {
    const { draft, message } = await handed();
    // In the request: the second comment is cut from the message. On the diff: one more comment.
    const cut = message.slice(0, message.indexOf('\n\n2. '));
    expect(cut).toContain('Call it `next`.');
    review.comment(key, fileAnchor('src/a.ts'), 'And add a test.');
    sendChat(draft, cut);
    expect(texts()).toEqual(['Why two lines?', 'And add a test.']);
    // The request holds none of them any more: Send back opens a new one with what is left.
    expect(waitingRequest(key)).toBeNull();
    expect(await sendBack(target, ['src/a.ts', 'b.txt'])).toBe(true);
    expect(ui.request).not.toBe(draft);
    expect(ui.composerSeed!.text).toContain('Review comments (2):');
  });
});
