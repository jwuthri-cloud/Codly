import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { Agent } from '../gen/Agent';
import type { Plan } from '../gen/Plan';
import type { Task } from '../gen/Task';
import { sendBack } from '../state/actions';
import { review, reviewKey } from '../state/review.svelte';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';
import { parseDiff, type DiffFile } from '../util/diff';
import {
  editorPlace, fileAnchor, fileMark, firstLines, followUpRequest, heavy, inOrder, LARGE_FILE, lineAnchor, lineCount, linePlace,
  linesToDraw, parseDiffKeeping, placeId, reviewMessage, withComment, type ReviewComment,
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
    ui.draftRequests = [];
    ui.composerSeed = null;
    ui.openRequest('lead');
  });

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
    store.tasks = [{ ...task, status: 'done' }];
    store.plans = [plan];
    write();
    expect(await sendBack(target, ['src/a.ts'])).toBe(true);
    expect(ui.draftRequests).toHaveLength(1);
    expect(ui.request).toBe(ui.draftRequests[0].thread);
    expect(ui.composerSeed).toMatchObject({ thread: ui.request, text: `I reviewed task t1 “Add a” of the plan “Add two files”. Its work is on the branch codly/ab12/integration. Please make these changes.\n\n${message}` });
    expect(review.comments(key)).toEqual([]);

    const whole = { kind: 'plan' as const, id: 'p1' };
    write(reviewKey(whole));
    expect(await sendBack(whole, ['src/a.ts'])).toBe(true);
    expect(ui.composerSeed?.text.startsWith('I reviewed the plan “Add two files”.')).toBe(true);
    expect(review.comments(reviewKey(whole))).toEqual([]);
    expect(core).not.toHaveBeenCalled();
  });
});
