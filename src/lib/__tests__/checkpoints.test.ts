import { describe, expect, it } from 'vitest';
import { DemoCheckpoints } from '../demo/checkpoints';
import type { Agent } from '../gen/Agent';

const NOW = 1_760_000_000_000;
const agent = (o: Partial<Agent>) => ({ id: 'a1', name: 'Ada', status: 'idle', worktree: '/demo/wt/ada', ...o }) as Agent;

// The demo follows the rules of orchestrator/checkpoints.rs; these are the same cases.
describe('DemoCheckpoints', () => {
  it('lists the points of an agent with a worktree, newest first', () => {
    const view = new DemoCheckpoints().list(agent({}), NOW);
    expect(view.off).toBeNull();
    expect(view.checkpoints.map((c) => c.n)).toEqual([3, 2, 1]);
    expect(view.checkpoints.every((c) => c.kind === 'turn' && c.changed === c.files.length && c.ts < NOW)).toBe(true);
  });

  it('says why an agent without a worktree has none, and restores nothing there', () => {
    const demo = new DemoCheckpoints();
    const lead = agent({ name: 'Lead', worktree: null });
    expect(demo.list(lead, NOW)).toEqual({ off: 'Lead has no worktree now, so there are no checkpoints.', checkpoints: [], task: null, left_out: [] });
    expect(() => demo.restore(lead, 1, NOW)).toThrow('no worktree');
    expect(demo.list(undefined, NOW).checkpoints).toEqual([]);
  });

  it('a restore keeps the way back as the newest point', () => {
    const demo = new DemoCheckpoints();
    const ada = agent({});
    const before = demo.list(ada, NOW).checkpoints.find((c) => c.n === 2)!;
    const done = demo.restore(ada, 2, NOW + 1);
    expect(done).toEqual({ files: before.files, undo: 4 });
    const after = demo.list(ada, NOW + 2).checkpoints;
    expect(after.map((c) => [c.n, c.kind])).toEqual([[4, 'restore'], [3, 'turn'], [2, 'turn'], [1, 'turn']]);
    expect(after[0]).toMatchObject({ label: before.label, changed: before.changed, ts: NOW + 1 });
    expect(after[2]).toMatchObject({ changed: 0, files: [] });
    // The files are as at that point now: nothing to restore there, and the undo is possible.
    expect(() => demo.restore(ada, 2, NOW + 3)).toThrow('already as they were');
    expect(demo.restore(ada, 4, NOW + 3).undo).toBe(5);
  });

  it('one file goes back alone, and the other files of the point still differ', () => {
    const demo = new DemoCheckpoints();
    const ada = agent({});
    const done = demo.restore(ada, 2, NOW + 1, 'src/theme/useTheme.ts');
    expect(done).toEqual({ files: ['src/theme/useTheme.ts'], undo: 4 });
    const after = demo.list(ada, NOW + 2).checkpoints;
    expect(after[0]).toMatchObject({ n: 4, kind: 'restore', changed: 1, files: ['src/theme/useTheme.ts'] });
    expect(after.find((c) => c.n === 2)).toMatchObject({ changed: 1, files: ['src/theme/ThemeProvider.tsx'] });
    // That file is as it was at the point now; a file the point never listed was never different.
    expect(() => demo.restore(ada, 2, NOW + 3, 'src/theme/useTheme.ts')).toThrow('src/theme/useTheme.ts is already as it was at that point');
    expect(() => demo.restore(ada, 2, NOW + 3, 'README.md')).toThrow('README.md is already as it was at that point');
    expect(() => demo.restore(agent({ status: 'working' }), 2, NOW + 3, 'src/theme/ThemeProvider.tsx')).toThrow('Ada is still working');
  });

  it('the last turn is the newest point taken before a turn, with what differs from it now', () => {
    const demo = new DemoCheckpoints();
    const ada = agent({});
    const turn = demo.lastTurn(ada, NOW)!;
    expect(turn).toMatchObject({ label: 'Wire the theme toggle into the header', ts: NOW - 4 * 60_000, task: null });
    expect(turn.diff).toContain('diff --git a/src/theme/ThemeProvider.tsx b/src/theme/ThemeProvider.tsx');
    // Its files went back: the turn is still the last one, and nothing of it is left.
    demo.restore(ada, 3, NOW + 1);
    expect(demo.lastTurn(ada, NOW + 2)).toMatchObject({ label: 'Wire the theme toggle into the header', diff: '' });
    expect(demo.lastTurn(agent({ worktree: null }), NOW)).toBeNull();
    expect(demo.lastTurn(undefined, NOW)).toBeNull();
  });

  it('refuses while the agent works, and for a point that is gone', () => {
    const demo = new DemoCheckpoints();
    expect(() => demo.restore(agent({ status: 'working' }), 2, NOW)).toThrow('Ada is still working; restore files when its turn ends');
    expect(() => demo.restore(agent({}), 99, NOW)).toThrow('that point is gone');
    expect(demo.list(agent({}), NOW).checkpoints).toHaveLength(3);
  });
});
