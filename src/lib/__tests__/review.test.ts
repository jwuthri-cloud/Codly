import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { Agent } from '../gen/Agent';
import type { Plan } from '../gen/Plan';
import type { Task } from '../gen/Task';
import { draftSlot, loadDraft, saveDraft } from '../../home/drafts';
import { closeRequest, discardReview, reviewDraft, sendBack, sendChat, waitingRequest } from '../state/actions';
import { requests } from '../state/requests.svelte';
import { review, reviewKey } from '../state/review.svelte';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';
import { parseDiff, type DiffFile } from '../util/diff';
import {
  discardWords, editorPlace, fileAnchor, fileMark, firstLines, followUpRequest, heavy, inOrder, LARGE_FILE, lineAnchor, lineCount, linePlace,
  linesToDraw, offDiff, parseDiffKeeping, placeComments, placeId, reviewBarWords, reviewMessage, withComment, type ReviewComment,
} from '../util/review';

const A = `diff --git a/src/a.ts b/src/a.ts
index 111..222 100644
--- a/src/a.ts
+++ b/src/a.ts
@@ -10,4 +10,4 @@ export function a() {
   const keep = 1;
-  const old = 2;
+  const fresh = 2;
   return keep;
\\ No newline at end of file
`;
const B = `diff --git a/b.txt b/b.txt
new file mode 100644
--- /dev/null
+++ b/b.txt
@@ -0,0 +1,2 @@
+one
+two
`;
const [fileA] = parseDiff(A);
const [keep, old, fresh, , meta] = fileA.hunks[0].lines;

describe('where a comment sits', () => {
  it('a line is named by its number in the new file, a removed line by its number in the old one', () => {
    expect(lineAnchor('src/a.ts', keep)).toEqual({ path: 'src/a.ts', line: 10, side: 'new', quote: '   const keep = 1;' });
    expect(lineAnchor('src/a.ts', old)).toEqual({ path: 'src/a.ts', line: 11, side: 'old', quote: '-  const old = 2;' });
    expect(lineAnchor('src/a.ts', fresh)).toEqual({ path: 'src/a.ts', line: 11, side: 'new', quote: '+  const fresh = 2;' });
    // The view asks for the place of every line it draws, without building the anchor.
    for (const l of [keep, old, fresh]) expect(linePlace(l)).toBe(placeId(lineAnchor('src/a.ts', l)));
    expect(linePlace(meta)).toBeNull();
    expect(placeId(fileAnchor('src/a.ts'))).toBe('file');
  });

  it('one comment per place: writing again replaces it, empty text removes it', () => {
    const onOld = lineAnchor('src/a.ts', old);
    const onNew = lineAnchor('src/a.ts', fresh);
    let list = withComment([], onOld, '  Why remove it?  ');
    list = withComment(list, onNew, 'Name it better.');
    list = withComment(list, fileAnchor('src/a.ts'), 'Split this file.');
    expect(list.map((c) => c.text)).toEqual(['Why remove it?', 'Name it better.', 'Split this file.']);
    // The same line number on the other side is another place.
    list = withComment(list, onNew, 'Call it `next`.');
    expect(list.map((c) => c.text)).toEqual(['Why remove it?', 'Split this file.', 'Call it `next`.']);
    expect(withComment(list, onOld, ' \n').map((c) => c.text)).toEqual(['Split this file.', 'Call it `next`.']);
    // The same place in another file is another place too.
    expect(withComment(list, { ...onNew, path: 'b.txt' }, 'Other file.')).toHaveLength(4);
  });

  it('a comment sits on its line only while the diff shows the line as you quoted it', () => {
    const onKeep = { ...lineAnchor('src/a.ts', keep), text: 'Keep it.' };
    const onFresh = { ...lineAnchor('src/a.ts', fresh), text: 'Name it better.' };
    const onFile = { ...fileAnchor('src/a.ts'), text: 'Split this file.' };
    const list = [onKeep, onFresh, onFile];
    const before = placeComments(fileA, list);
    expect([...before.placed]).toEqual([['file', onFile], ['new:10', onKeep], ['new:11', onFresh]]);
    expect(before.lost).toEqual([]);
    // The worker changed line 11 since: your comment is not drawn on the code that is there now.
    const [changed] = parseDiff(A.replace('const fresh = 2;', 'const next = 2;'));
    const after = placeComments(changed, list);
    expect([...after.placed]).toEqual([['file', onFile], ['new:10', onKeep]]);
    expect(after.lost).toEqual([onFresh]);
    // A comment on the line as it reads now is a new one: it does not replace the old one.
    const again = withComment(list, lineAnchor('src/a.ts', changed.hunks[0].lines[2]), 'Better.');
    expect(again.map((c) => c.text)).toEqual(['Keep it.', 'Name it better.', 'Split this file.', 'Better.']);
    expect(placeComments(changed, again).lost).toEqual([onFresh]);
    // Removing the old one takes only it.
    expect(withComment(again, onFresh, '').map((c) => c.text)).toEqual(['Keep it.', 'Split this file.', 'Better.']);
    // The lines left the diff: only the comment on the file still has a place.
    const gone = placeComments({ ...fileA, hunks: [] }, list);
    expect([...gone.placed]).toEqual([['file', onFile]]);
    expect(gone.lost).toEqual([onKeep, onFresh]);
  });
});

describe('the message Send back builds', () => {
  const list: ReviewComment[] = [
    { ...lineAnchor('src/a.ts', fresh), text: 'Call it `next`.\nAnd keep the type.' },
    { path: 'b.txt', line: 2, side: 'new', quote: '+two', text: 'Three is missing.' },
    { ...lineAnchor('src/a.ts', old), text: 'Why remove it?' },
    { ...fileAnchor('src/a.ts'), text: 'Split this file.' },
    { path: 'gone.md', line: 1, side: 'new', quote: '+x', text: 'Left the diff.' },
  ];
  const paths = ['b.txt', 'src/a.ts'];

  it('reads in the order of the diff: file by file, the file comment first, then line by line', () => {
    expect(inOrder(list, paths).map((c) => c.text.split('\n')[0])).toEqual(['Three is missing.', 'Split this file.', 'Why remove it?', 'Call it `next`.', 'Left the diff.']);
  });

  it('finds the comments on files that left the diff', () => {
    expect(offDiff(list, paths).map((c) => c.text)).toEqual(['Left the diff.']);
    expect(offDiff(list, [...paths, 'gone.md'])).toEqual([]);
    expect(offDiff(list, [])).toHaveLength(5);
  });

  it('has, for each comment, the file and line, the quoted line and the comment', () => {
    expect(reviewMessage(list, paths)).toBe(
      [
        'Review comments (5):',
        '',
        '1. `b.txt:2`',
        '   > +two',
        '   Three is missing.',
        '',
        '2. `src/a.ts` (the whole file)',
        '   Split this file.',
        '',
        '3. `src/a.ts:11` (a line the change removes)',
        '   > -  const old = 2;',
        '   Why remove it?',
        '',
        '4. `src/a.ts:11`',
        '   > +  const fresh = 2;',
        '   Call it `next`.',
        '   And keep the type.',
        '',
        '5. `gone.md:1`',
        '   > +x',
        '   Left the diff.',
      ].join('\n'),
    );
  });

  it('cuts a very long quoted line', () => {
    const long: ReviewComment = { path: 'min.js', line: 1, side: 'new', quote: `+${'x'.repeat(5000)}`, text: 'Do not commit this.' };
    const quote = reviewMessage([long], ['min.js']).split('\n')[3];
    expect(quote).toHaveLength('   > '.length + 200);
    expect(quote.endsWith('…')).toBe(true);
  });

  it('a follow-up request names what was reviewed and where its work is', () => {
    const task = { key: 't2', title: 'Add b', branch: 'codly/ab12/t2', plan_id: 'p1' } as Task;
    const plan = { summary: 'Add two files', integration_branch: 'codly/ab12/integration', merged_sha: null } as Plan;
    const one = [list[1]];
    expect(followUpRequest(task, plan, one, paths)).toBe(
      'I reviewed task t2 “Add b” of the plan “Add two files”. Its work is on the branch codly/ab12/integration. Please make these changes.\n\n' + reviewMessage(one, paths),
    );
    expect(followUpRequest(null, { ...plan, merged_sha: 'abc' }, one, paths).split('\n')[0]).toBe('I reviewed the plan “Add two files”. Its work is merged. Please make these changes.');
    expect(followUpRequest({ ...task, plan_id: null }, null, one, paths).split('\n')[0]).toBe('I reviewed task t2 “Add b”. Its work is on the branch codly/ab12/t2. Please make these changes.');
  });
});

describe('what the review bar says', () => {
  it('one comment is "it", several are "them"', () => {
    expect(reviewBarWords(1, 'Ana', null)).toBe('Send back gives it to Ana; Codly checks the work again after the changes.');
    expect(reviewBarWords(2, 'Ana', null)).toBe('Send back gives them to Ana; Codly checks the work again after the changes.');
    expect(reviewBarWords(1, null, null)).toBe('This work is finished: Send back opens a new request with your comment, ready to send.');
    expect(reviewBarWords(3, null, null)).toBe('This work is finished: Send back opens a new request with your comments, ready to send.');
    expect(reviewBarWords(1, null, 1)).toBe('It is in a new request, ready to send, and stays here until you send its message. Send back opens that request.');
    expect(reviewBarWords(2, null, 2)).toBe('They are in a new request, ready to send, and stay here until you send its message. Send back opens that request.');
  });

  it('after Send back, a comment written or changed since is counted apart: it is not in the request yet', () => {
    expect(reviewBarWords(2, null, 1)).toBe('1 is in a new request, ready to send; 1 is not in it yet. Send back puts it in that request too.');
    expect(reviewBarWords(5, null, 2)).toBe('2 are in a new request, ready to send; 3 are not in it yet. Send back puts them in that request too.');
    // Every comment changed since: the request holds none of them as they are now.
    expect(reviewBarWords(1, null, 0)).toBe('A new request holds your comment as it was at Send back, not as it is now. Send back brings that request up to date.');
    expect(reviewBarWords(2, null, 0)).toBe('A new request holds your comments as they were at Send back, not as they are now. Send back brings that request up to date.');
  });

  it('before a discard, says where else the comments are', () => {
    expect(discardWords(1, null)).toBe('It is not kept anywhere else.');
    expect(discardWords(2, null)).toBe('They are not kept anywhere else.');
    expect(discardWords(1, 'seeded')).toBe('Discard also takes it out of the new request.');
    expect(discardWords(2, 'seeded')).toBe('Discard also takes them out of the new request.');
    expect(discardWords(1, 'edited')).toBe('It is also in a draft request, in a message you changed: that draft stays as you wrote it.');
    expect(discardWords(2, 'edited')).toBe('They are also in a draft request, in a message you changed: that draft stays as you wrote it.');
  });
});

describe('large diffs', () => {
  const file = (path: string, added = 1): DiffFile => ({ oldPath: path, newPath: path, path, status: 'modified', hunks: [], added, removed: 0 });

  it('lockfiles, generated files and large files open collapsed', () => {
    for (const p of ['pnpm-lock.yaml', 'web/package-lock.json', 'Cargo.lock', 'crates/x/Cargo.lock', 'go.sum']) expect(heavy(file(p))).toBe('lockfile');
    for (const p of ['dist/app.js', 'web/node_modules/x/index.js', 'public/app.min.js', 'app.js.map', 'src/__snapshots__/a.test.ts.snap', 'api/user.pb.go']) expect(heavy(file(p))).toBe('generated');
    expect(heavy(file('src/a.ts', LARGE_FILE + 1))).toBe('large');
    // Source that only looks like it: a folder named build, a file named lock.ts.
    for (const p of ['src/a.ts', 'src/office/build/build.ts', 'src/lock.ts', 'docs/distance.md']) expect(heavy(file(p, LARGE_FILE))).toBeNull();
  });

  it('a file draws its first lines only, hunk by hunk', () => {
    const [f] = parseDiff(`${B}@@ -5,0 +7,3 @@\n+a\n+b\n+c\n`);
    expect(lineCount(f)).toBe(5);
    expect(firstLines(f, 500).map((s) => s.lines.length)).toEqual([2, 3]);
    expect(firstLines(f, 500)[1].lines).toBe(f.hunks[1].lines);
    expect(firstLines(f, 3).map((s) => s.lines.map((l) => l.text))).toEqual([['one', 'two'], ['a']]);
    expect(firstLines(f, 2).map((s) => s.lines.length)).toEqual([2]);
  });

  it('a commented line is drawn however far down it is', () => {
    const [f] = parseDiff(`${B}@@ -5,0 +7,3 @@\n+a\n+b\n+c\n`);
    const on = (...places: string[]) => new Set(places);
    expect(linesToDraw(f, 2, on())).toBe(2);
    expect(linesToDraw(f, 2, on('file', 'new:1'))).toBe(2);
    // "b" is line 8 of the new file, the fourth line of the diff.
    expect(linesToDraw(f, 2, on('new:8'))).toBe(4);
    expect(linesToDraw(f, 2, on('new:7', 'new:9'))).toBe(5);
    expect(linesToDraw(f, 500, on('new:9'))).toBe(500);
  });

  it('a new diff keeps the parsed files whose text did not change', () => {
    const first = parseDiffKeeping(A + B, new Map());
    expect(first.files).toEqual(parseDiff(A + B));
    const next = parseDiffKeeping(A + B.replace('+two', '+2'), first.parts);
    expect(next.files).toEqual(parseDiff(A + B.replace('+two', '+2')));
    expect(next.files[0]).toBe(first.files[0]);
    expect(next.files[1]).not.toBe(first.files[1]);
    // A file that left the diff leaves the kept parts too.
    expect(parseDiffKeeping(B, next.parts).parts.size).toBe(1);
    expect(parseDiffKeeping('', new Map()).files).toEqual([]);
  });

  it('a file\'s mark changes when its diff does', () => {
    expect(fileMark(parseDiff(B)[0])).toBe(fileMark(parseDiff(B)[0]));
    expect(fileMark(parseDiff(B.replace('+two', '+2'))[0])).not.toBe(fileMark(parseDiff(B)[0]));
    expect(fileMark(parseDiff(B.replace('+two', '-two'))[0])).not.toBe(fileMark(parseDiff(B)[0]));
  });
});

describe('where a file opens in the editor', () => {
  const task = { id: 't1', plan_id: 'p1', status: 'blocked', worktree: '/wt/t1', branch: 'codly/ab12/t1' } as Task;
  const plan = { integration_branch: 'codly/ab12/integration', merged_sha: null } as Plan;
  const worker = { id: 'w1', name: 'Ana', worktree: '/wt/t1', task_id: null } as Agent;
  const lead = { id: 'l1', name: 'Lead', worktree: null } as Agent;

  it('in the worktree of the task, while it is still that task\'s', () => {
    expect(editorPlace(task, plan, worker, lead)).toEqual({ agentId: 'w1', tip: 'Opens the file in Ana’s worktree' });
    expect(editorPlace(task, plan, { ...worker, task_id: 't1' }, lead).agentId).toBe('w1');
    // A team member that moved on to its next task, a worker in another worktree, a merged task.
    for (const w of [{ ...worker, task_id: 't9' }, { ...worker, worktree: '/wt/t9' }, { ...worker, worktree: null }]) {
      expect(editorPlace(task, plan, w, lead)).toEqual({ agentId: null, tip: 'This work is only on the branch codly/ab12/integration for now' });
    }
    expect(editorPlace({ ...task, status: 'done' }, plan, worker, lead).agentId).toBeNull();
    // A done task with no plan keeps its worktree: nothing merged it away.
    expect(editorPlace({ ...task, status: 'done', plan_id: null }, null, worker, lead).agentId).toBe('w1');
  });

  it('in your repository once the plan is merged', () => {
    const merged = { ...plan, merged_sha: 'abc' };
    expect(editorPlace({ ...task, status: 'done' }, merged, worker, lead)).toEqual({ agentId: 'l1', tip: 'Opens the file in your repository: the plan is merged' });
    expect(editorPlace(null, merged, null, lead).agentId).toBe('l1');
    expect(editorPlace(null, merged, null, null).agentId).toBeNull();
    expect(editorPlace(null, null, null, lead)).toEqual({ agentId: null, tip: 'No worktree has this file' });
  });
});

describe('your review of a diff, for the session', () => {
  const key = reviewKey({ kind: 'task', id: 't1' });
  afterEach(() => review.clear(key));

  it('keeps comments per diff until they are sent or discarded', () => {
    review.comment(key, lineAnchor('src/a.ts', fresh), 'Name it better.');
    review.comment(key, lineAnchor('src/a.ts', fresh), 'Call it `next`.');
    expect(review.comments(key).map((c) => c.text)).toEqual(['Call it `next`.']);
    expect(review.comments(reviewKey({ kind: 'plan', id: 't1' }))).toEqual([]);
    review.clear(key);
    expect(review.comments(key)).toEqual([]);
  });

  it('a viewed file is unticked when its diff changes', () => {
    const [b] = parseDiff(B);
    expect(review.viewed(key, b)).toBe(false);
    review.setViewed(key, b, true);
    expect(review.viewed(key, b)).toBe(true);
    // The same diff parsed again (a refresh) is still the one you read; a changed one is not.
    expect(review.viewed(key, parseDiff(B)[0])).toBe(true);
    expect(review.viewed(key, parseDiff(B.replace('+two', '+2'))[0])).toBe(false);
    expect(review.viewed(reviewKey({ kind: 'plan', id: 'p1' }), b)).toBe(false);
    review.setViewed(key, b, false);
    expect(review.viewed(key, b)).toBe(false);
  });

  it('a ticked file folds away, and opens again when its diff changes', () => {
    const mine = reviewKey({ kind: 'task', id: 'folds' });
    const plan = reviewKey({ kind: 'plan', id: 'folds' });
    const [b] = parseDiff(B);
    expect(review.folded(mine, b)).toBe(false);
    // Folded by hand, then ticked: from the tick on, the tick decides.
    review.fold(mine, 'b.txt', true);
    review.setViewed(mine, b, true);
    expect(review.folded(mine, b)).toBe(true);
    // The same file in the plan's diff is not viewed there, so it is open there.
    expect(review.folded(plan, b)).toBe(false);
    // The worker changed the file: it is not viewed any more, and open to be read again.
    const [changed] = parseDiff(B.replace('+two', '+2'));
    expect(review.viewed(mine, changed)).toBe(false);
    expect(review.folded(mine, changed)).toBe(false);
    // A viewed file opened by hand stays open; unticking opens a file too.
    review.setViewed(mine, changed, true);
    review.fold(mine, 'b.txt', false);
    expect(review.folded(mine, changed)).toBe(false);
    review.setViewed(mine, changed, true);
    expect(review.folded(mine, changed)).toBe(true);
    review.setViewed(mine, changed, false);
    expect(review.folded(mine, changed)).toBe(false);
  });

  it('a lockfile opens folded until you open it, in that diff only', () => {
    const mine = reviewKey({ kind: 'task', id: 'folds' });
    const lock: DiffFile = { ...parseDiff(B)[0], path: 'Cargo.lock' };
    expect(review.folded(mine, lock)).toBe(true);
    review.fold(mine, 'Cargo.lock', false);
    expect(review.folded(mine, lock)).toBe(false);
    expect(review.folded(reviewKey({ kind: 'plan', id: 'folds' }), lock)).toBe(true);
  });
});

describe('Send back', () => {
  const task = { id: 't1', key: 't1', title: 'Add a', plan_id: 'p1', status: 'reviewing', assignee: 'w1', branch: 'codly/ab12/t1' } as Task;
  const plan = { id: 'p1', summary: 'Add two files', thread: 'lead', integration_branch: 'codly/ab12/integration', merged_sha: null } as Plan;
  const target = { kind: 'task' as const, id: 't1' };
  const key = reviewKey(target);
  const write = (k = key) => review.comment(k, lineAnchor('src/a.ts', fresh), 'Call it `next`.');
  const message = 'Review comments (1):\n\n1. `src/a.ts:11`\n   > +  const fresh = 2;\n   Call it `next`.';

  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
    review.clear(key);
    review.clear(reviewKey({ kind: 'plan', id: 'p1' }));
    store.tasks = [];
    store.plans = [];
    store.agents = [];
    for (const o of [...store.outbox]) store.dropOutgoing(o.id);
    ui.draftRequests = [];
    ui.composerSeed = null;
    ui.openRequest('lead');
  });
  /** The work is finished: Send back goes to a new request. */
  const finished = () => {
    store.tasks = [{ ...task, status: 'done' }];
    store.plans = [plan];
  };

  it('gives a task that is not done all the comments as one message, and clears them once the core took them', async () => {
    store.tasks = [task];
    store.agents = [{ id: 'w1', name: 'Ana' } as Agent];
    const core = vi.spyOn(api, 'sendBackTask').mockRejectedValueOnce(new Error('Codly is working on t1 right now')).mockResolvedValueOnce({ ...task, status: 'changes_requested' });
    expect(await sendBack(target, ['src/a.ts'])).toBe(false);
    expect(core).not.toHaveBeenCalled(); // nothing to send
    write();
    // A refusal says why, and the comments stay on the diff.
    expect(await sendBack(target, ['src/a.ts'])).toBe(false);
    expect(core).toHaveBeenLastCalledWith('t1', message);
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['error', 'Couldn’t send your comments back', 'Codly is working on t1 right now — your comments stay on the diff.']]);
    expect(review.comments(key)).toHaveLength(1);

    expect(await sendBack(target, ['src/a.ts'])).toBe(true);
    expect(core).toHaveBeenCalledTimes(2);
    expect(review.comments(key)).toEqual([]);
    expect(store.taskMap.get('t1')?.status).toBe('changes_requested');
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toContainEqual(['success', 't1 sent back to Ana', '1 comment']);
    expect(ui.draftRequests).toEqual([]);
  });

  it('opens a new request with the comments for a done task and for a whole plan', async () => {
    const core = vi.spyOn(api, 'sendBackTask');
    finished();
    write();
    expect(await sendBack(target, ['src/a.ts'])).toBe(true);
    expect(ui.draftRequests).toHaveLength(1);
    expect(ui.request).toBe(ui.draftRequests[0].thread);
    expect(ui.composerSeed).toMatchObject({ thread: ui.request, text: `I reviewed task t1 “Add a” of the plan “Add two files”. Its work is on the branch codly/ab12/integration. Please make these changes.\n\n${message}` });
    // Nothing is sent yet: the comments are still on the diff, which knows where they wait.
    expect(review.comments(key)).toHaveLength(1);
    expect(waitingRequest(key)).toBe(ui.request);

    const whole = { kind: 'plan' as const, id: 'p1' };
    write(reviewKey(whole));
    expect(await sendBack(whole, ['src/a.ts'])).toBe(true);
    expect(ui.composerSeed?.text.startsWith('I reviewed the plan “Add two files”.')).toBe(true);
    expect(review.comments(reviewKey(whole))).toHaveLength(1);
    expect(ui.draftRequests).toHaveLength(2); // the first request holds your comments: it stays a tab
    expect(core).not.toHaveBeenCalled();
  });

  it('the request with your comments survives opening another one, and the comments leave the diff only when its message is sent', async () => {
    vi.spyOn(api, 'sendChat').mockResolvedValue();
    finished();
    write();
    await sendBack(target, ['src/a.ts']);
    const draft = ui.request;
    const text = ui.composerSeed!.text;

    ui.openRequest('lead');
    expect(ui.draftRequests.map((d) => d.thread)).toEqual([draft]);
    expect(requests.get(draft)).not.toBeNull(); // still a tab
    expect(review.comments(key)).toHaveLength(1);

    // A message in another request sends nothing of the review.
    sendChat('lead', 'Something else');
    expect(review.comments(key)).toHaveLength(1);
    expect(waitingRequest(key)).toBe(draft);

    sendChat(draft, text);
    expect(review.comments(key)).toEqual([]);
    expect(waitingRequest(key)).toBeNull();
  });

  it('sending the message takes only the comments that are in it: one written or changed since stays on the diff', async () => {
    vi.spyOn(api, 'sendChat').mockResolvedValue();
    finished();
    write();
    review.comment(key, fileAnchor('b.txt'), 'Why two lines?');
    await sendBack(target, ['src/a.ts', 'b.txt']);
    const draft = ui.request;
    const first = ui.composerSeed!.text;

    // Back in Changes: one comment more, one changed, one saved again as it was. Send back is
    // not pressed again, so the message in the request is still the first one.
    review.comment(key, fileAnchor('src/a.ts'), 'And add a test.');
    review.comment(key, lineAnchor('src/a.ts', fresh), 'Call it `after`.');
    review.comment(key, fileAnchor('b.txt'), 'Why two lines?');
    sendChat(draft, first);
    expect(review.comments(key).map((c) => c.text)).toEqual(['And add a test.', 'Call it `after`.']);
    expect(waitingRequest(key)).toBeNull();
  });

  it('another message sent from that request sends no comment: they all stay on the diff, and Send back opens a new request', async () => {
    vi.spyOn(api, 'sendChat').mockResolvedValue();
    finished();
    write();
    await sendBack(target, ['src/a.ts']);
    const draft = ui.request;
    const first = ui.composerSeed!.text;
    ui.composerSeed = null; // the composer took it

    sendChat(draft, 'Something else');
    expect(review.comments(key)).toHaveLength(1);
    expect(waitingRequest(key)).toBeNull();

    expect(await sendBack(target, ['src/a.ts'])).toBe(true);
    expect(ui.request).not.toBe(draft);
    expect(ui.composerSeed).toMatchObject({ thread: ui.request, text: first, replaces: null });
  });

  it('Send back again opens a new request with the comments when the first one was emptied', async () => {
    finished();
    write();
    await sendBack(target, ['src/a.ts']);
    const draft = ui.request;
    const first = ui.composerSeed!.text;
    ui.composerSeed = null; // the composer took it
    ui.holdDraft(draft, false); // and you cleared it there
    expect(waitingRequest(key)).toBeNull(); // the comments are not in a request any more

    // From Changes, with the comments as they were: they are put in a composer again.
    expect(await sendBack(target, ['src/a.ts'])).toBe(true);
    expect(ui.request).not.toBe(draft);
    expect(ui.composerSeed).toMatchObject({ thread: ui.request, text: first, replaces: null });
    expect(ui.draftRequests.map((d) => d.thread)).toEqual([ui.request]); // the emptied one is dropped
    expect(waitingRequest(key)).toBe(ui.request);
  });

  it('Send back again goes to the same request, and brings the comments as they are now', async () => {
    finished();
    write();
    await sendBack(target, ['src/a.ts']);
    const draft = ui.request;
    const first = ui.composerSeed!.text;
    ui.composerSeed = null; // the composer took it
    ui.openRequest('lead');

    // Nothing changed since: you are taken there, and its composer is left as you have it.
    expect(await sendBack(target, ['src/a.ts'])).toBe(true);
    expect(ui.request).toBe(draft);
    expect(ui.composerSeed).toBeNull();
    expect(ui.draftRequests).toHaveLength(1);

    // One comment more: the message there is brought up to date, in place of the earlier one.
    review.comment(key, fileAnchor('src/a.ts'), 'And add a test.');
    expect(await sendBack(target, ['src/a.ts'])).toBe(true);
    expect(ui.draftRequests).toHaveLength(1);
    expect(ui.composerSeed).toMatchObject({ thread: draft, replaces: first });
    expect(ui.composerSeed!.text).toContain('And add a test.');
    expect(review.handedTo(key)?.text).toBe(ui.composerSeed!.text);
  });

  it('a request closed with nothing sent leaves the comments on the diff, and Send back opens a new one', async () => {
    vi.spyOn(api, 'closeThread').mockResolvedValue();
    vi.spyOn(store, 'resync').mockResolvedValue();
    finished();
    write();
    await sendBack(target, ['src/a.ts']);
    const draft = ui.request;

    expect(await closeRequest(draft)).toBe(true);
    expect(review.comments(key)).toHaveLength(1);
    expect(waitingRequest(key)).toBeNull();

    expect(await sendBack(target, ['src/a.ts'])).toBe(true);
    expect(ui.request).not.toBe(draft);
    expect(ui.draftRequests.map((d) => d.thread)).toEqual([ui.request]);
    expect(waitingRequest(key)).toBe(ui.request);
  });

  it('counts the comments that are in the request as they are now', async () => {
    finished();
    write();
    expect(review.inHand(key)).toBe(0); // no Send back yet
    await sendBack(target, ['src/a.ts']);
    expect(review.inHand(key)).toBe(1);
    review.comment(key, fileAnchor('src/a.ts'), 'And add a test.');
    expect([review.comments(key).length, review.inHand(key)]).toEqual([2, 1]);
    review.comment(key, lineAnchor('src/a.ts', fresh), 'Call it `after`.');
    expect([review.comments(key).length, review.inHand(key)]).toEqual([2, 0]);
  });

  /** Send back from the request `lead`, then back to Changes: the composer took the message and keeps it as its draft, off screen. */
  const sentBack = async (around = (message: string) => message) => {
    finished();
    write();
    await sendBack(target, ['src/a.ts']);
    const draft = ui.request;
    const slot = draftSlot(store.workspace?.id, draft);
    saveDraft(slot, around(ui.composerSeed!.text), []);
    ui.composerSeed = null;
    ui.go('changes');
    return { draft, slot };
  };

  it('Discard after Send back takes the comments out of the request too, and drops a request that then holds nothing', async () => {
    expect(reviewDraft(key)).toBeNull(); // in no request
    const { draft, slot } = await sentBack();
    expect(reviewDraft(key)).toBe('seeded');

    discardReview(key);
    expect(review.comments(key)).toEqual([]);
    expect(loadDraft(slot)).toEqual({ text: '', files: [] });
    expect(ui.draftRequests).toEqual([]);
    expect(requests.get(draft)).toBeNull(); // not a tab any more
    // You were on that request: you are on the one beside it now, still in Changes.
    expect([ui.request, ui.thread, ui.room]).toEqual(['lead', 'lead', 'changes']);
  });

  it('Discard leaves what you wrote around the comments, in a request that stays', async () => {
    const { draft, slot } = await sentBack((message) => `Start with the names.\n\n${message}\n\nThen run the tests.`);
    expect(reviewDraft(key)).toBe('seeded');

    discardReview(key);
    expect(review.comments(key)).toEqual([]);
    expect(loadDraft(slot).text).toBe('Start with the names.\n\nThen run the tests.');
    expect(ui.draftRequests.map((d) => d.thread)).toEqual([draft]);
    expect(ui.request).toBe(draft);
    saveDraft(slot, '', []);
  });

  it('Discard leaves a message you changed: the draft is yours', async () => {
    const { draft, slot } = await sentBack((message) => message.replace('Call it `next`.', 'Call it `after`.'));
    expect(reviewDraft(key)).toBe('edited');

    discardReview(key);
    expect(review.comments(key)).toEqual([]);
    expect(loadDraft(slot).text).toContain('Call it `after`.');
    expect(ui.draftRequests.map((d) => d.thread)).toEqual([draft]);
    saveDraft(slot, '', []);
  });

  it('Discard with no Send back clears the diff and touches no request', () => {
    finished();
    write();
    ui.newRequest();
    const other = ui.request;
    ui.holdDraft(other, true);
    discardReview(key);
    expect(review.comments(key)).toEqual([]);
    expect(ui.draftRequests.map((d) => d.thread)).toEqual([other]);
  });
});
