// The browser demo's background command (demo/background.ts, docs/DEMO.md): in her first task
// Ada leaves a type check running, and the demo says Claude Code's list of background tasks at
// each change, as the app gets it. "Stop" on her sheet ends the command sooner.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MockControls } from '../demo/controls';
import { REPO, Sim } from '../demo/sim';
import { Story } from '../demo/story';
import type { EventKind } from '../gen/EventKind';
import { withBackgroundTasks, type BackgroundWork } from '../state/session';
import { DEFAULT_SETTINGS } from '../state/store.svelte';

const COMMAND = 'npx tsc --noEmit --watch';
type Listed = Extract<EventKind, { type: 'background_tasks' }>;

/** The demo with its repo open, playing by itself (plans run, requests are allowed). */
function demo() {
  const sim = new Sim({ speed: 1, skipMs: 0, autoplay: true }, { ...DEFAULT_SETTINGS });
  new Story(sim).openWorkspace(REPO);
  return sim;
}

/** Let the script play until `done`, for 400 seconds of its time at most. */
async function until(done: () => boolean) {
  for (let i = 0; i < 800 && !done(); i++) await vi.advanceTimersByTimeAsync(500);
  expect(done(), 'the script got there in time').toBe(true);
}

const ada = (sim: Sim) => [...sim.agents.values()].find((a) => a.name === 'Ada');
/** Ada's lists of background tasks so far, oldest first. */
const lists = (sim: Sim) => sim.log.filter((e) => e.kind.type === 'background_tasks' && e.agent_id === ada(sim)?.id).map((e) => e.kind as Listed);
const taskDone = (sim: Sim) => sim.taskByKey('t1')?.status === 'done';

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date', 'performance'] });
  // Every pause and every length of a step is the same in each run.
  vi.spyOn(Math, 'random').mockReturnValue(0.5);
  // The story draws one picture on a canvas; a test has none.
  vi.stubGlobal('OffscreenCanvas', class { getContext() { return null; } });
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('the demo: a command an agent leaves running in the background', () => {
  it('is listed while it runs, and Ada stops it before her report', async () => {
    const sim = demo();
    await until(() => lists(sim).length === 1);
    expect(lists(sim)[0]).toMatchObject({ tasks: [{ kind: 'command', description: COMMAND }], ended: [] });
    const id = lists(sim)[0].tasks[0].id;

    await until(() => taskDone(sim));
    expect(lists(sim)).toHaveLength(2);
    expect(lists(sim)[1]).toEqual({ type: 'background_tasks', tasks: [], ended: [{ id, kind: 'command', description: COMMAND, status: 'stopped' }] });
    // The stop comes before the message that ends her turn on the task.
    const events = sim.log.filter((e) => e.agent_id === ada(sim)!.id);
    const stopped = events.findIndex((e) => e.kind.type === 'background_tasks' && e.kind.ended.length > 0);
    const reported = events.findIndex((e) => e.kind.type === 'message' && e.kind.text.startsWith('Added `ThemeProvider`'));
    expect(stopped).toBeGreaterThan(0);
    expect(stopped).toBeLessThan(reported);
    // What her sheet makes of the two lists: one row, over, in the CLI's word.
    const rows = sim.log.reduce<BackgroundWork[]>((held, e) => (e.kind.type === 'background_tasks' && e.agent_id === ada(sim)!.id ? withBackgroundTasks(held, e.kind, e.run_id, e.ts) : held), []);
    expect(rows.map((w) => [w.description, w.status])).toEqual([[COMMAND, 'stopped']]);
    expect(rows[0].endedAt! - rows[0].startedAt!).toBeGreaterThan(0);
  });

  it('"Stop" on her sheet ends it sooner, and it is not stopped a second time', async () => {
    const sim = demo();
    const controls = new MockControls(sim);
    await until(() => lists(sim).length === 1);
    const before = sim.log.length;

    controls.stopTask(ada(sim)!.id);
    expect(lists(sim)).toHaveLength(2);
    expect(lists(sim)[1]).toMatchObject({ tasks: [], ended: [{ description: COMMAND, status: 'stopped' }] });
    // Your act is on record, as in the app.
    const yours = sim.log.slice(before).filter((e) => e.kind.type === 'decision').map((e) => (e.kind.type === 'decision' ? e.kind.summary : ''));
    expect(yours).toEqual(['asked Ada to stop a command it left running in the background']);
    expect(() => controls.stopTask(ada(sim)!.id)).toThrow('Ada has no command running in the background');

    // Her own stop before the report finds it over: no third list.
    await until(() => taskDone(sim));
    expect(lists(sim)).toHaveLength(2);
  });
});
