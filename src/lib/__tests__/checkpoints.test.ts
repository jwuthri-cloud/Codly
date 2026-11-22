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
    expect(demo.list(lead, NOW)).toEqual({ off: 'Lead has no worktree now, so there are no checkpoints.', checkpoints: [] });
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

  it('refuses while the agent works, and for a point that is gone', () => {
    const demo = new DemoCheckpoints();
    expect(() => demo.restore(agent({ status: 'working' }), 2, NOW)).toThrow('Ada is still working; restore files when its turn ends');
    expect(() => demo.restore(agent({}), 99, NOW)).toThrow('that point is gone');
    expect(demo.list(agent({}), NOW).checkpoints).toHaveLength(3);
  });
});
