import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { PreviewDemo } from '../demo/preview';
import type { DemoWorld } from '../demo/pullrequest';
import type { Agent } from '../gen/Agent';
import type { Plan } from '../gen/Plan';
import type { Preview } from '../gen/Preview';
import type { PreviewRun } from '../gen/PreviewRun';
import type { PreviewSource } from '../gen/PreviewSource';
import { loadPreview, previewKey, previewUi, startPreview, stopPreview } from '../state/preview.svelte';
import { toasts } from '../state/toasts.svelte';

const plan: PreviewSource = { kind: 'plan', id: 'p1' };
const WT = '/wt/dark-mode/integration';
const run = (r: Partial<PreviewRun>): PreviewRun => ({ id: 'r1', command: 'pnpm dev', started_at: 1, ended: null, url: null, lines: [], printed: 0, ...r });
const view = (v: Partial<Preview>): Preview => ({ worktree: WT, command: null, port_base: null, run: null, ...v });
const said = () => toasts.list.map((t) => [t.kind, t.title, t.body]);
const shown = () => previewUi.shown[previewKey(plan)];

describe('trying the work before a merge', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of toasts.list) toasts.dismiss(t.id);
    previewUi.shown = {};
  });

  it('reading the preview runs nothing, and Run sends the text you confirmed', async () => {
    const core = vi.spyOn(api, 'preview').mockResolvedValue(view({ command: 'pnpm dev' }));
    await loadPreview(plan);
    expect(core).toHaveBeenCalledWith(plan, null, null);
    expect(shown()).toEqual({ view: view({ command: 'pnpm dev' }), lines: [] });

    core.mockResolvedValue(view({ command: 'pnpm dev --port $CODLY_PORT_BASE', port_base: 20000, run: run({ lines: ['> vite'], printed: 1 }) }));
    expect(await startPreview(plan, 'pnpm dev --port $CODLY_PORT_BASE')).toBe(true);
    expect(core).toHaveBeenLastCalledWith(plan, { do: 'start', command: 'pnpm dev --port $CODLY_PORT_BASE' }, null);
    expect(shown().lines).toEqual(['> vite']);
    expect(said()).toEqual([]);
  });

  it('a refresh asks for the lines after the ones shown and adds them', async () => {
    const core = vi.spyOn(api, 'preview').mockResolvedValue(view({ run: run({ lines: ['one', 'two'], printed: 2 }) }));
    await loadPreview(plan);
    core.mockResolvedValue(view({ run: run({ lines: ['three'], printed: 3, url: 'http://localhost:20000/' }) }));
    await loadPreview(plan);
    expect(core).toHaveBeenLastCalledWith(plan, null, { run: 'r1', lines: 2 });
    expect(shown().lines).toEqual(['one', 'two', 'three']);
    expect(shown().view.run?.url).toBe('http://localhost:20000/');

    // Two answers that crossed: the second one repeats a line the page has. It is not shown twice.
    core.mockResolvedValue(view({ run: run({ lines: ['three', 'four'], printed: 4 }) }));
    await loadPreview(plan);
    expect(shown().lines).toEqual(['one', 'two', 'three', 'four']);

    // An answer from before that one arrives late: nothing goes back, nothing is asked twice.
    core.mockResolvedValue(view({ run: run({ lines: ['three'], printed: 3 }) }));
    await loadPreview(plan);
    expect(shown().lines).toEqual(['one', 'two', 'three', 'four']);
    expect(shown().view.run?.printed).toBe(4);

    // The core dropped old lines the page never got: the output starts again from what it has.
    core.mockResolvedValue(view({ run: run({ lines: ['ninety'], printed: 90 }) }));
    await loadPreview(plan);
    expect(shown().lines).toEqual(['ninety']);

    // Another run in the same worktree: its output replaces the old one.
    core.mockResolvedValue(view({ run: run({ id: 'r2', lines: ['fresh'], printed: 1 }) }));
    await loadPreview(plan);
    expect(shown().lines).toEqual(['fresh']);

    // A refresh that fails keeps what is shown, without a toast for each try.
    core.mockRejectedValue(new Error('no core'));
    await loadPreview(plan);
    expect(shown().lines).toEqual(['fresh']);
    expect(said()).toEqual([]);
  });

  it('a command that cannot start, or stop, says why', async () => {
    const core = vi.spyOn(api, 'preview').mockRejectedValue(new Error('the plan is still running; try it when it is done'));
    expect(await startPreview(plan, 'pnpm dev')).toBe(false);
    expect(shown()).toBeUndefined();
    expect(said()).toEqual([['error', 'Couldn’t run the command', 'the plan is still running; try it when it is done']]);

    for (const t of toasts.list) toasts.dismiss(t.id);
    core.mockResolvedValue(view({ run: run({ lines: ['up'], printed: 1 }) }));
    await loadPreview(plan);
    core.mockResolvedValue(view({ run: run({ ended: 'stopped', printed: 1 }) }));
    expect(await stopPreview(plan)).toBe(true);
    expect(core).toHaveBeenLastCalledWith(plan, { do: 'stop' }, { run: 'r1', lines: 1 });
    expect(shown().view.run?.ended).toBe('stopped');
    expect(shown().lines).toEqual(['up']);
    // A refresh sent before the Stop answers after it: the run stays ended.
    core.mockResolvedValue(view({ run: run({ printed: 1 }) }));
    await loadPreview(plan);
    expect(shown().view.run?.ended).toBe('stopped');
  });
});

describe('the demo’s preview', () => {
  let now = 1000;
  const world: DemoWorld = {
    plans: new Map([
      ['p1', { id: 'p1', integration_worktree: WT } as Plan],
      ['p0', { id: 'p0', integration_worktree: null } as Plan],
    ]),
    tasks: new Map(),
    agents: new Map([['a1', { id: 'a1', name: 'Ada', worktree: '/wt/ada' } as Agent]]),
    chats: [],
    now: () => now,
  };

  it('runs nothing until Run, prints over time on the worktree’s port, and stops', () => {
    const demo = new PreviewDemo();
    expect(demo.handle({ source: plan, action: null, seen: null }, world)).toEqual({ worktree: WT, command: null, port_base: null, run: null });
    expect(() => demo.handle({ source: { kind: 'plan', id: 'p0' }, action: null, seen: null }, world)).toThrow('the plan never ran');
    expect(() => demo.handle({ source: plan, action: { do: 'start', command: '  ' }, seen: null }, world)).toThrow('write the command');

    const started = demo.handle({ source: plan, action: { do: 'start', command: ' pnpm dev ' }, seen: null }, world);
    expect(started).toMatchObject({ command: 'pnpm dev', port_base: 20000, run: { command: 'pnpm dev', ended: null, url: null, lines: [], printed: 0 } });
    expect(() => demo.handle({ source: plan, action: { do: 'start', command: 'pnpm dev' }, seen: null }, world)).toThrow('already runs');

    now += 1000;
    const up = demo.handle({ source: plan, action: null, seen: null }, world).run!;
    expect(up.printed).toBe(7);
    expect(up.url).toBe('http://localhost:20000/');
    const more = demo.handle({ source: plan, action: null, seen: { run: up.id, lines: 5 } }, world).run!;
    expect(more.lines).toEqual(up.lines.slice(5));

    // The command is the repo's, and each worktree has ports of its own.
    const ada = demo.handle({ source: { kind: 'agent', id: 'a1' }, action: null, seen: null }, world);
    expect(ada).toEqual({ worktree: '/wt/ada', command: 'pnpm dev', port_base: null, run: null });
    expect(demo.handle({ source: { kind: 'agent', id: 'a1' }, action: { do: 'start', command: 'pnpm dev' }, seen: null }, world).port_base).toBe(20010);

    const stopped = demo.handle({ source: plan, action: { do: 'stop' }, seen: null }, world).run!;
    expect(stopped.ended).toBe('stopped');
    now += 10_000;
    expect(demo.handle({ source: plan, action: null, seen: null }, world).run!.printed).toBe(7);
  });

  it('`?preview=running`: a command you started earlier runs in each worktree, until you stop it', () => {
    const demo = new PreviewDemo(true);
    const first = demo.handle({ source: plan, action: null, seen: null }, world);
    expect(first).toMatchObject({ command: 'pnpm dev --port $CODLY_PORT_BASE', port_base: 20000, run: { ended: null, url: 'http://localhost:20000/', printed: 8 } });
    expect(() => demo.handle({ source: plan, action: { do: 'start', command: 'pnpm dev' }, seen: null }, world)).toThrow('already runs');
    expect(demo.handle({ source: { kind: 'agent', id: 'a1' }, action: null, seen: null }, world)).toMatchObject({ port_base: 20010, run: { ended: null } });
    // Stopped, it stays stopped: nothing starts by itself a second time.
    expect(demo.handle({ source: plan, action: { do: 'stop' }, seen: null }, world).run!.ended).toBe('stopped');
    expect(demo.handle({ source: plan, action: null, seen: null }, world).run!.ended).toBe('stopped');
  });
});
