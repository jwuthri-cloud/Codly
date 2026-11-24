// The browser demo's Send back on a finished plan (demo/reviewTask.ts), run on fake timers: like
// the core, the comments become one more task of the plan, for the agent that wrote the file,
// in the plan's worktree, and the plan is done again.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { sendBackPlanDemo } from '../demo/reviewTask';
import { REPO, Sim } from '../demo/sim';
import { Story } from '../demo/story';
import type { EventKind } from '../gen/EventKind';
import { DEFAULT_SETTINGS } from '../state/store.svelte';

function demo() {
  const sim = new Sim({ speed: 1, skipMs: 0, autoplay: true }, { ...DEFAULT_SETTINGS });
  const story = new Story(sim);
  story.openWorkspace(REPO);
  return { sim, story };
}

async function until(done: () => boolean, seconds = 400) {
  for (let i = 0; i < seconds * 2 && !done(); i++) await vi.advanceTimersByTimeAsync(500);
  expect(done(), 'the script got there in time').toBe(true);
}

const COMMENTS = 'Review comments (1):\n\n1. `x` (the whole file)\n   Say it twice.';

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date', 'performance'] });
  vi.spyOn(Math, 'random').mockReturnValue(0.5);
  vi.stubGlobal('OffscreenCanvas', class { getContext() { return null; } });
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('Send back on the demo’s finished plan', () => {
  it('makes the comments one more task of the plan, for the agent that wrote the file, and the plan is done again', async () => {
    const { sim } = demo();
    await until(() => [...sim.plans.values()].some((p) => p.thread === 'lead' && p.status === 'done'));
    const plan = [...sim.plans.values()].find((p) => p.thread === 'lead')!;
    // A file the plan's last finished task committed, and the agent of that task.
    const commits = sim.log.filter((e) => e.kind.type === 'committed' && e.task_id && sim.tasks.get(e.task_id)?.plan_id === plan.id);
    const last = commits[commits.length - 1];
    const file = (last.kind as Extract<EventKind, { type: 'committed' }>).files[0];
    const author = sim.tasks.get(last.task_id!)!;
    const before = [...sim.tasks.values()].filter((t) => t.plan_id === plan.id && t.status === 'done').length;

    const task = sendBackPlanDemo(sim, plan.id, COMMENTS, [file]);
    expect([task.key, task.title, task.assignee, task.branch, task.worktree]).toEqual(['review', 'Changes you asked for in review', author.assignee, plan.integration_branch, plan.integration_worktree]);
    expect(task.goal.endsWith(COMMENTS)).toBe(true);
    expect(sim.plans.get(plan.id)!.status).toBe('running');
    const said = sim.chats.filter((c) => c.thread === plan.thread);
    // Your comments are in Codly's line: no message of yours is added to the request.
    expect(said[said.length - 1].author).toBe('codly');
    expect(said.filter((c) => c.author === 'user').every((c) => c.text !== COMMENTS)).toBe(true);
    const name = sim.agents.get(author.assignee!)!.name;
    expect(said[said.length - 1].text.startsWith(`Your review comments are task review of this plan, for ${name} (who wrote ${author.key})`)).toBe(true);
    expect(said[said.length - 1].text.endsWith(`What you sent:\n\n${COMMENTS}`)).toBe(true);
    // A second press while it works makes no second task.
    expect(() => sendBackPlanDemo(sim, plan.id, COMMENTS, [file])).toThrow('the plan is still running; send your comments again when it is done');

    await until(() => sim.plans.get(plan.id)!.status === 'done');
    expect(sim.tasks.get(task.id)!.status).toBe('done');
    expect([...sim.tasks.values()].filter((t) => t.plan_id === plan.id && t.status === 'done')).toHaveLength(before + 1);
    // Its agent worked in the plan's worktree, and has its own again.
    const agent = sim.agents.get(author.assignee!)!;
    expect(sim.log.some((e) => e.agent_id === agent.id && e.kind.type === 'turn_started' && e.kind.input?.startsWith('Task review'))).toBe(true);
    expect([agent.task_id, agent.worktree === plan.integration_worktree]).toEqual([null, false]);
    expect(sim.log.some((e) => e.task_id === task.id && e.kind.type === 'committed')).toBe(true);
  });

  it('refuses in the core’s words', async () => {
    const { sim } = demo();
    await until(() => [...sim.plans.values()].some((p) => p.thread === 'lead' && p.status === 'done'));
    const plan = [...sim.plans.values()].find((p) => p.thread === 'lead')!;
    expect(() => sendBackPlanDemo(sim, plan.id, ' ', ['x'])).toThrow('there is no comment to send');
    sim.patchPlan(plan.id, { merged_sha: 'abc' });
    expect(() => sendBackPlanDemo(sim, plan.id, COMMENTS, ['x'])).toThrow('this plan is merged into your branch; ask for the changes in a new request');
  });
});
