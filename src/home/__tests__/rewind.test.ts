// "Undo a turn": what the list over the composer shows, what its keys do, what it asks before a
// restore, and which checkpoint the sheet's "Put this file back" uses.
import { describe, expect, it } from 'vitest';
import type { AgentCheckpoints } from '../../lib/gen/AgentCheckpoints';
import type { Checkpoint } from '../../lib/gen/Checkpoint';
import { ESC_PAIR_WINDOW } from '../../lib/state/esc';
import { confirmLine, focusRow, heldAfterClose, leftOutLine, namesLine, pick, putBackTitle, REWIND_ROWS, rewindItems, rewindKey, rewindOff, rewindRows, rewindStart, rewindWhere, toggleFiles, turnPoint, type RewindNav } from '../rewind';

const NOW = 1_760_000_000_000;
const MIN = 60_000;
const point = (n: number, o: Partial<Checkpoint> = {}): Checkpoint => ({ n, kind: 'turn', ts: NOW - n * MIN, label: `Turn ${n}`, changed: 1, files: ['a.ts'], ...o });
const kept = (checkpoints: Checkpoint[], o: Partial<AgentCheckpoints> = {}): AgentCheckpoints => ({ off: null, checkpoints, task: null, left_out: [], ...o });

describe('the rows of the list', () => {
  it('are the turns, newest first as the core gives them, with their age, count and first files', () => {
    const rows = rewindRows(kept([point(3, { ts: NOW - 4 * MIN, changed: 2, files: ['a.ts', 'b.ts'] }), point(2, { kind: 'restore' }), point(1, { ts: NOW - 50 * MIN })]), NOW);
    expect(rows).toEqual([
      { n: 3, label: 'Turn 3', age: '4 min ago', changed: 2, files: ['a.ts', 'b.ts'] },
      { n: 1, label: 'Turn 1', age: '50 min ago', changed: 1, files: ['a.ts'] },
    ]);
  });

  it(`shows at most ${REWIND_ROWS}`, () => {
    const many = Array.from({ length: 14 }, (_, i) => point(14 - i));
    expect(rewindRows(kept(many), NOW).map((r) => r.n)).toEqual([14, 13, 12, 11, 10, 9, 8, 7, 6, 5]);
  });

  it('says why there is none: the core’s reason, or that no turn was kept yet', () => {
    expect(rewindOff(kept([], { off: 'Lea has no worktree of its own, so there are no checkpoints.' }), 'Lea')).toBe('Lea has no worktree of its own, so there are no checkpoints.');
    expect(rewindOff(kept([]), 'Ada')).toBe('Codly kept no turn of Ada yet');
    // A restore's own point is not a turn to undo.
    expect(rewindOff(kept([point(4, { kind: 'restore' })]), 'Ada')).toBe('Codly kept no turn of Ada yet');
    expect(rewindOff(kept([point(1)]), 'Ada')).toBeNull();
  });

  it('cuts the names after three', () => {
    expect(namesLine(['a.ts'], 1)).toBe('a.ts');
    expect(namesLine(['a.ts', 'b.ts', 'c.ts', 'd.ts'], 4)).toBe('a.ts, b.ts, c.ts and 1 more');
    // The core lists the first files only: the count says how many there are.
    expect(namesLine(['a.ts', 'b.ts'], 7)).toBe('a.ts, b.ts and 5 more');
  });
});

describe('the confirm line', () => {
  it('says what goes back and that the conversation stays', () => {
    expect(confirmLine('Ada', { changed: 3, files: ['a.ts', 'b.ts', 'c.ts'] }, null)).toBe(
      'Put 3 files back to how they were before this turn? a.ts, b.ts, c.ts. The conversation stays: Ada is told which files went back.',
    );
    expect(confirmLine('Ada', { changed: 1, files: ['a.ts'] }, null)).toBe('Put 1 file back to how it was before this turn? a.ts. The conversation stays: Ada is told which file went back.');
    expect(confirmLine('Ada', { changed: 3, files: ['a.ts', 'b.ts', 'c.ts'] }, 'b.ts')).toBe('Put b.ts back to how it was before this turn? The conversation stays: Ada is told which file went back.');
  });

  it('the toast after it names the turn', () => {
    expect(putBackTitle(3, 'Wire the toggle')).toBe('3 files put back to before “Wire the toggle”');
    expect(putBackTitle(1, 'Wire the toggle')).toBe('1 file put back to before “Wire the toggle”');
  });
});

describe('the keys of the list', () => {
  // The newest turn changed nothing that differs now; the two before it did.
  const rows = rewindRows(kept([point(3, { changed: 0, files: [] }), point(2, { changed: 2, files: ['a.ts', 'b.ts'] }), point(1)]), NOW);
  const press = (nav: RewindNav, ...keys: string[]) => {
    let r = { nav, act: null as ReturnType<typeof rewindKey>['act'] };
    for (const k of keys) r = rewindKey(r.nav, rows, k);
    return r;
  };
  const start = rewindStart(rows);
  const at = (nav: RewindNav) => rewindItems(rows, nav.open)[nav.at];

  it('start on the newest turn with something to put back: one with nothing cannot be picked', () => {
    expect(at(start)).toEqual({ kind: 'turn', n: 2 });
    expect(at(press(start, 'ArrowDown').nav)).toEqual({ kind: 'turn', n: 1 });
    expect(at(press(start, 'ArrowDown', 'ArrowDown').nav)).toEqual({ kind: 'all' });
    // Round, past the turn that changed nothing.
    expect(at(press(start, 'ArrowDown', 'ArrowDown', 'ArrowDown').nav)).toEqual({ kind: 'turn', n: 2 });
    expect(at(press(start, 'ArrowUp').nav)).toEqual({ kind: 'all' });
    expect(pick(start, rows, 0)).toEqual({ nav: start, act: null });
  });

  it('↵ shows the confirm line, a second ↵ restores the turn; Esc closes one level', () => {
    const asked = press(start, 'Enter');
    expect(asked).toEqual({ nav: { ...start, asking: { n: 2, path: null } }, act: null });
    expect(press(asked.nav, 'Enter').act).toEqual({ kind: 'restore', n: 2, path: null });
    // Other keys while it asks do nothing.
    expect(press(asked.nav, 'ArrowDown').nav).toEqual(asked.nav);
    const back = press(asked.nav, 'Escape');
    expect(back).toEqual({ nav: start, act: null });
    expect(press(back.nav, 'Escape').act).toEqual({ kind: 'close' });
  });

  it('→ lists the files of the turn, each one goes back alone; ← and Esc close them', () => {
    const open = press(start, 'ArrowRight').nav;
    expect(open.open).toBe(2);
    expect(at(open)).toEqual({ kind: 'turn', n: 2 });
    const second = press(open, 'ArrowDown', 'ArrowDown').nav;
    expect(at(second)).toEqual({ kind: 'file', n: 2, path: 'b.ts' });
    expect(press(second, 'Enter', 'Enter').act).toEqual({ kind: 'restore', n: 2, path: 'b.ts' });
    // Esc: the confirm line, then the files (the keys go back to the turn), then the list.
    const asked = press(second, 'Enter').nav;
    expect(press(asked, 'Escape').nav).toEqual(second);
    expect(press(second, 'Escape').nav).toEqual(start);
    expect(press(second, 'ArrowLeft').nav).toEqual(start);
    expect(press(open, 'Escape', 'Escape').act).toEqual({ kind: 'close' });
  });

  it('a press on the file count opens and closes the files; a turn with nothing has none', () => {
    const open = toggleFiles(start, rows, 1);
    expect(open).toEqual({ at: 2, open: 1, asking: null });
    expect(toggleFiles(open, rows, 1)).toEqual({ at: 2, open: null, asking: null });
    expect(toggleFiles(start, rows, 3)).toBe(start);
  });

  it('"All checkpoints…" opens the sheet', () => {
    expect(press(start, 'ArrowUp', 'Enter').act).toEqual({ kind: 'sheet' });
  });

  it('with no turn the keys have only "All checkpoints…"', () => {
    expect(rewindStart([])).toEqual({ at: 0, open: null, asking: null });
    expect(rewindKey(rewindStart([]), [], 'Enter').act).toEqual({ kind: 'sheet' });
  });
});

describe('the keys and the focus are on one row', () => {
  const rows = rewindRows(kept([point(3, { changed: 0, files: [] }), point(2, { changed: 2, files: ['a.ts', 'b.ts'] }), point(1)]), NOW);
  const start = rewindStart(rows);

  it('Tab or a press onto another row moves the keys there: ↵ acts on the row that has the focus', () => {
    const tabbed = focusRow(start, rows, 2);
    expect(tabbed).toEqual({ ...start, at: 2 });
    const asked = rewindKey(tabbed, rows, 'Enter');
    expect(asked.nav.asking).toEqual({ n: 1, path: null });
    expect(rewindKey(asked.nav, rows, 'Enter').act).toEqual({ kind: 'restore', n: 1, path: null });
  });

  it('the focus leaving a row that asks closes its confirm line: a later ↵ never restores that row', () => {
    const asked = rewindKey(start, rows, 'Enter').nav;
    expect(asked.asking).toEqual({ n: 2, path: null });
    const moved = focusRow(asked, rows, 2);
    expect(moved).toEqual({ at: 2, open: null, asking: null });
    expect(rewindKey(moved, rows, 'Enter').act).toBeNull();
    // On the same row (its file count, its "Put back"), the confirm line stays.
    expect(focusRow(asked, rows, asked.at)).toBe(asked);
  });

  it('a file row can take the keys; a turn with nothing to put back, or no row, cannot', () => {
    const open = toggleFiles(start, rows, 2);
    const file = focusRow(open, rows, 3);
    expect(rewindItems(rows, file.open)[file.at]).toEqual({ kind: 'file', n: 2, path: 'b.ts' });
    expect(rewindKey(rewindKey(file, rows, 'Enter').nav, rows, 'Enter').act).toEqual({ kind: 'restore', n: 2, path: 'b.ts' });
    expect(focusRow(start, rows, 0)).toBe(start);
    expect(focusRow(start, rows, 99)).toBe(start);
  });
});

describe('the head of the list', () => {
  it('says when the turns are of a task’s worktree, and names the files too big to keep', () => {
    expect(rewindWhere(kept([], { task: 'T-4' }))).toBe('Turns of task T-4, in its worktree');
    expect(rewindWhere(kept([]))).toBeNull();
    expect(leftOutLine(kept([], { left_out: ['node_modules/ (20000 new files)', 'dist/app.bin'] }))).toBe('Not kept, too big: node_modules/ (20000 new files), dist/app.bin');
    expect(leftOutLine(kept([]))).toBeNull();
  });
});

describe('the Esc right after the one that closed the list', () => {
  it('is held for the pair window only: it begins no pair that opens the list again', () => {
    expect(heldAfterClose(NOW, NOW + 4)).toBe(true);
    expect(heldAfterClose(NOW, NOW + ESC_PAIR_WINDOW)).toBe(true);
    expect(heldAfterClose(NOW, NOW + ESC_PAIR_WINDOW + 1)).toBe(false);
    expect(heldAfterClose(NOW, NOW - 1)).toBe(false);
  });
});

describe('the checkpoint of "Put this file back" in Last turn', () => {
  const turn = { ts: NOW - 3 * MIN, task: null };

  it('is the newest turn checkpoint, when it is the point of the turn shown', () => {
    expect(turnPoint(turn, kept([point(4, { kind: 'restore', ts: NOW }), point(3), point(2)]))).toBe(3);
  });

  it('is none when the list is of another worktree, the newest turn is another, or there are none', () => {
    expect(turnPoint(turn, kept([point(3)], { task: 'T-2' }))).toBeNull();
    expect(turnPoint({ ...turn, task: 'T-2' }, kept([point(3)]))).toBeNull();
    expect(turnPoint({ ...turn, task: 'T-2' }, kept([point(3)], { task: 'T-2' }))).toBe(3);
    expect(turnPoint(turn, kept([point(4), point(3)]))).toBeNull();
    expect(turnPoint(turn, kept([], { off: 'Ada has no worktree now, so there are no checkpoints.' }))).toBeNull();
  });
});
