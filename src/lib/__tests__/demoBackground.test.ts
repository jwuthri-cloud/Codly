// The browser demo's background work (demo/background.ts, docs/DEMO.md): in her first task Ada
// leaves a type check running, and the demo says Claude Code's list of background tasks at each
// change, as the app gets it. "Stop" on her sheet ends the command sooner. With `?background=on`
// a 1:1 agent leaves a command and a helper at work: "Stop" ends the one it names.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MockControls } from '../demo/controls';
import { REPO, Sim } from '../demo/sim';
import { Story } from '../demo/story';
import type { Agent } from '../gen/Agent';
import type { EventKind } from '../gen/EventKind';
import { ofLiveRun, withBackgroundTasks, type BackgroundWork } from '../state/session';
import { DEFAULT_SETTINGS } from '../state/store.svelte';

const COMMAND = 'npx tsc --noEmit --watch';
type Listed = Extract<EventKind, { type: 'background_tasks' }>;

/** The demo with its repo open, playing by itself (plans run, requests are allowed). */
function demo() {
  const sim = new Sim({ speed: 1, skipMs: 0, autoplay: true }, { ...DEFAULT_SETTINGS });
  new Story(sim).openWorkspace(REPO);
  return sim;
}

/** What the agent's sheet lists under "In the background": the work of its live run (sheet/AgentBackground.svelte). */
function sheet(sim: Sim, agent: Agent): [string, string][] {
  const rows = sim.log.reduce<BackgroundWork[]>((held, e) => (e.kind.type === 'background_tasks' && e.agent_id === agent.id ? withBackgroundTasks(held, e.kind, e.run_id, e.ts) : held), []);
  return ofLiveRun(rows, agent.run_id).map((w) => [w.description, w.status]);
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

    const id = lists(sim)[0].tasks[0].id;
    controls.stopTask(ada(sim)!.id, id);
    expect(lists(sim)).toHaveLength(2);
    expect(lists(sim)[1]).toMatchObject({ tasks: [], ended: [{ description: COMMAND, status: 'stopped' }] });
    // Your act is on record, as in the app: it names what you stopped.
    const yours = () => sim.log.slice(before).filter((e) => e.kind.type === 'decision').map((e) => (e.kind.type === 'decision' ? e.kind.summary : ''));
    expect(yours()).toEqual([`asked Ada to stop what it left running in the background: ${COMMAND}`]);
    // A second Stop finds it over: it answers, and nothing is said or recorded again.
    expect(controls.stopTask(ada(sim)!.id, id)).toBeNull();
    expect(lists(sim)).toHaveLength(2);
    expect(yours()).toHaveLength(1);
    expect(() => controls.stopTask(ada(sim)!.id, ' ')).toThrow('which background command?');

    // Her own stop before the report finds it over: no third list.
    await until(() => taskDone(sim));
    expect(lists(sim)).toHaveLength(2);
  });

  it('is gone with her run when you stop Ada: her sheet lists nothing that still runs', async () => {
    const sim = new Sim({ speed: 1, skipMs: 0, autoplay: true }, { ...DEFAULT_SETTINGS });
    const story = new Story(sim);
    story.openWorkspace(REPO);
    await until(() => lists(sim).length === 1);
    expect(sheet(sim, ada(sim)!)).toEqual([[COMMAND, 'running']]);

    story.interrupt(ada(sim)!.id, true);
    // As in the app: the run ended with her process, and the sheet shows the work of the live run only.
    expect(ada(sim)).toMatchObject({ status: 'offline', run_id: null });
    expect(sheet(sim, ada(sim)!)).toEqual([]);
    // Her task lets go of her, and no list comes from a process that is gone.
    await until(() => sim.taskByKey('t1')?.status === 'blocked');
    await vi.advanceTimersByTimeAsync(10_000);
    expect(lists(sim)).toHaveLength(1);
    expect(sheet(sim, ada(sim)!)).toEqual([]);
  });
});

describe('`?background=on`: a 1:1 Claude agent leaves a command and a helper at work', () => {
  const SERVER = 'npm run dev';
  const HELPER = 'Check every link in the docs';

  /** The demo in that state, with nothing else playing. */
  function left() {
    const sim = new Sim({ speed: 1, skipMs: 0, autoplay: false }, { ...DEFAULT_SETTINGS });
    const story = new Story(sim, { background: true, yourRequestFirst: true });
    story.openWorkspace(REPO);
    return { sim, story, controls: new MockControls(sim) };
  }
  const eli = (sim: Sim) => [...sim.agents.values()].find((a) => a.name === 'Eli')!;
  const of = <T extends EventKind['type']>(sim: Sim, type: T) =>
    sim.log.filter((e) => e.kind.type === type && e.agent_id === eli(sim).id).map((e) => e.kind as Extract<EventKind, { type: T }>);
  /** Both are listed, and Eli's turn is over. */
  const atRest = (sim: Sim) => of(sim, 'background_tasks').length === 2 && of(sim, 'turn_completed').length === 1;
  const said = (sim: Sim) => sim.chats.filter((c) => c.thread === `agent:${eli(sim).id}` && c.author === eli(sim).id).map((c) => c.text);

  it('lists both when its turn is over, the helper with a row of its own', async () => {
    const { sim } = left();
    await until(() => atRest(sim));
    expect(eli(sim)).toMatchObject({ role: 'solo', status: 'idle' });
    expect(of(sim, 'background_tasks')[1]).toMatchObject({ tasks: [{ kind: 'command', description: SERVER }, { kind: 'helper', description: HELPER }], ended: [] });
    expect(of(sim, 'sub_agent')).toEqual([expect.objectContaining({ description: HELPER, status: 'running' })]);
    // The helper's `Agent` call returned at once: it does not wait for the helper.
    const call = of(sim, 'sub_agent')[0].call_id;
    expect(of(sim, 'tool_completed').find((c) => c.call_id === call)).toMatchObject({ ok: true });
    // A snapshot that has lost the list's own row still has it (as the core's `session_events`).
    const at = sim.log.find((e) => e.kind.type === 'background_tasks' && e.kind.tasks.length === 2)!.id;
    for (let i = 0; i < 2100; i++) sim.decision('note', 'rule', `filler ${i}`, {});
    const snapshot = sim.snapshot();
    expect(snapshot.events.some((e) => e.id === at)).toBe(false);
    expect(snapshot.session_events.map((e) => e.id)).toContain(at);
  });

  it('“Stop” ends the one whose id it sends: the command, and no turn follows', async () => {
    const { sim, controls } = left();
    await until(() => atRest(sim));
    const [server, helper] = of(sim, 'background_tasks')[1].tasks;
    const before = sim.log.length;
    controls.stopTask(eli(sim).id, server.id);
    expect(of(sim, 'background_tasks')[2]).toEqual({ type: 'background_tasks', tasks: [helper], ended: [{ ...server, status: 'stopped' }] });
    const yours = sim.log.slice(before).filter((e) => e.kind.type === 'decision').map((e) => (e.kind.type === 'decision' ? e.kind.summary : ''));
    expect(yours).toEqual([`asked Eli to stop what it left running in the background: ${SERVER}`]);
    // As recorded for a command: Claude Code starts no turn after its stop. The helper works on.
    await vi.advanceTimersByTimeAsync(20_000);
    expect(of(sim, 'turn_started')).toHaveLength(1);
    expect(of(sim, 'sub_agent').map((s) => s.status)).toEqual(['running']);
  });

  it('“Stop” on the helper ends it, and Eli says so in a short turn of its own', async () => {
    const { sim, controls } = left();
    await until(() => atRest(sim));
    const [server, helper] = of(sim, 'background_tasks')[1].tasks;
    const row = of(sim, 'sub_agent')[0];
    // The call of its own that the helper is in.
    const own = of(sim, 'sub_agent_progress').filter((p) => p.tool_call_id).pop()!.tool_call_id!;
    const before = sim.log.length;
    controls.stopTask(eli(sim).id, helper.id);

    // As the adapter says a helper's stop: its row, the list without it, then the end of the call it was in.
    const after = sim.log.slice(before).filter((e) => e.agent_id === eli(sim).id).map((e) => e.kind);
    const order = ['sub_agent', 'background_tasks', 'tool_completed', 'turn_started'].map((type) => after.findIndex((k) => k.type === type));
    expect(order.every((at) => at >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(of(sim, 'sub_agent')[1]).toEqual({ ...row, status: 'stopped' });
    expect(of(sim, 'background_tasks')[2]).toEqual({ type: 'background_tasks', tasks: [server], ended: [{ ...helper, status: 'stopped' }] });
    expect(of(sim, 'tool_completed').find((c) => c.call_id === own)).toMatchObject({ ok: false });

    // The turn Claude Code starts by itself: no message of yours began it.
    expect(of(sim, 'turn_started').map((t) => t.input)).toEqual([expect.any(String), null]);
    await until(() => of(sim, 'turn_completed').length === 2);
    expect(of(sim, 'turn_completed')[1].status).toBe('completed');
    expect(said(sim).pop()).toBe('The helper was stopped before it reported, so I do not have its answer.');
    expect(eli(sim).status).toBe('idle');

    // A second Stop finds it over. The helper does not report later, and no third turn comes.
    expect(controls.stopTask(eli(sim).id, helper.id)).toBeNull();
    await vi.advanceTimersByTimeAsync(240_000);
    expect(of(sim, 'sub_agent').map((s) => s.status)).toEqual(['running', 'stopped']);
    expect(of(sim, 'background_tasks')).toHaveLength(3);
    expect(of(sim, 'turn_started')).toHaveLength(2);
  });

  it('left alone, the helper reports after about three minutes, and Eli tells you in a turn of its own', async () => {
    const { sim } = left();
    await until(() => of(sim, 'sub_agent').length === 2);
    expect(of(sim, 'sub_agent')[1].status).toBe('completed');
    const [server, helper] = of(sim, 'background_tasks')[1].tasks;
    expect(of(sim, 'background_tasks')[2]).toEqual({ type: 'background_tasks', tasks: [server], ended: [{ ...helper, status: 'completed' }] });
    // Each call of its own ended well: none was cut.
    const calls = of(sim, 'sub_agent_progress').flatMap((p) => (p.tool_call_id ? [p.tool_call_id] : []));
    expect(calls).toHaveLength(3);
    for (const id of calls) expect(of(sim, 'tool_completed').find((c) => c.call_id === id)).toMatchObject({ ok: true });
    await until(() => of(sim, 'turn_completed').length === 2);
    expect(of(sim, 'turn_started')[1].input).toBeNull();
    expect(said(sim).pop()).toBe('The helper is done. All 42 links in `docs/` answer.');
  });

  it('a turn you stop does not end the helper in the background', async () => {
    const { sim, story } = left();
    // Eli's turn still runs when the helper is listed: stop it there.
    for (let i = 0; i < 400 && of(sim, 'background_tasks').length < 2; i++) await vi.advanceTimersByTimeAsync(50);
    expect(sim.actor(eli(sim).id).inTurn).toBe(true);
    story.interrupt(eli(sim).id);
    await until(() => of(sim, 'turn_completed').length === 1);
    expect(of(sim, 'turn_completed')[0].status).toBe('interrupted');
    await vi.advanceTimersByTimeAsync(10_000);
    expect(of(sim, 'sub_agent').map((s) => s.status)).toEqual(['running']);
    expect(of(sim, 'background_tasks')).toHaveLength(2);
    // It works on to its report.
    await until(() => of(sim, 'sub_agent').length === 2);
    expect(of(sim, 'sub_agent')[1].status).toBe('completed');
  });

  it('a stop of the agent takes its background work with it', async () => {
    const { sim, story, controls } = left();
    await until(() => atRest(sim));
    const helper = of(sim, 'background_tasks')[1].tasks[1];
    expect(sheet(sim, eli(sim))).toEqual([[SERVER, 'running'], [HELPER, 'running']]);
    const run = eli(sim).run_id;
    story.interrupt(eli(sim).id, true);
    // As in the app: the run ended with the process, so the sheet has no row of it (none with a timer that runs on).
    expect(eli(sim).run_id).toBeNull();
    expect(sheet(sim, eli(sim))).toEqual([]);
    expect(() => controls.stopTask(eli(sim).id, helper.id)).toThrow('Eli is not running');
    // The process is gone: no list, no report and no turn come from it.
    const before = sim.log.length;
    await vi.advanceTimersByTimeAsync(240_000);
    expect(sim.log.slice(before).filter((e) => e.agent_id === eli(sim).id)).toEqual([]);

    // Its next turn starts its process again: a new run, with nothing in the background.
    await sim.actor(eli(sim).id).turn('Are you there?', async () => {});
    expect(eli(sim).run_id).toEqual(expect.any(String));
    expect(eli(sim).run_id).not.toBe(run);
    expect(sheet(sim, eli(sim))).toEqual([]);
    expect(of(sim, 'turn_started').pop()).toEqual({ type: 'turn_started', input: 'Are you there?' });
    expect(sim.log.filter((e) => e.kind.type === 'turn_started' && e.agent_id === eli(sim).id).pop()!.run_id).toBe(eli(sim).run_id);
  });
});
