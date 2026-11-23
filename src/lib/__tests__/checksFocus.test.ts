// "Check the work" asks the core again when Codly's window gets the focus back (a file you
// changed in your editor is not the checked one any more). Each question starts git in the
// core, so a window that gets the focus again and again does not ask each time.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import type { AgentCheck } from '../gen/AgentCheck';
import { agentChecks, FOCUS_READ_MS } from '../state/agentChecks.svelte';

const idle: AgentCheck = { running: false, last: null };

describe('the focus of the window and “Check the work”', () => {
  /** Each test has an agent of its own: the state keeps when each agent was last asked about. */
  let n = 0;
  let agent = '';
  /** The agents the core was asked about, in order. */
  const asked = () => vi.mocked(api.agentCheckState).mock.calls.map(([id]) => id);
  /** The clock moves on: the time the state reads, and the questions that wait for it. */
  const later = (ms: number) => vi.advanceTimersByTime(ms);

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(1_760_000_000_000);
    vi.spyOn(api, 'agentCheckState').mockResolvedValue(idle);
    agent = `focus-${++n}`;
  });
  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
    vi.restoreAllMocks();
    agentChecks.state = {};
    agentChecks.failed = {};
    agentChecks.unread = {};
  });

  it('asks at once when the agent was not asked about for a while', () => {
    agentChecks.focused(agent);
    expect(asked()).toEqual([agent]);
    later(FOCUS_READ_MS);
    agentChecks.focused(agent);
    expect(asked()).toEqual([agent, agent]);
  });

  it('asks once for many focuses in a row, and once more when the time is over', () => {
    agentChecks.focused(agent);
    for (const ms of [50, 850, 3100, 5999]) {
      later(ms);
      agentChecks.focused(agent);
    }
    // 9999 ms after the first question: still the only one.
    expect(asked()).toEqual([agent]);
    // The focuses in between are not dropped: a file changed before the last of them is seen
    // when the time is over, with one question for all of them.
    later(1);
    expect(asked()).toEqual([agent, agent]);
    // Nothing waits any more: no further question comes without a focus.
    later(FOCUS_READ_MS * 3);
    expect(asked()).toEqual([agent, agent]);
  });

  it('counts the time from the last question, whoever asked it', async () => {
    // The Git tab opened and read the worktree: the focus right after it waits its turn.
    await agentChecks.load(agent);
    later(2000);
    agentChecks.focused(agent);
    expect(asked()).toEqual([agent]);
    later(FOCUS_READ_MS - 2001);
    expect(asked()).toEqual([agent]);
    later(1);
    expect(asked()).toEqual([agent, agent]);
  });

  // Before, the stored timer handle was only a flag: the focus's question was asked anyway.
  it('a focus that waits asks nothing when a newer question answered meanwhile', async () => {
    agentChecks.focused(agent);
    later(2000);
    agentChecks.focused(agent); // too soon: it waits until 10 s after the first question
    later(1000);
    // The Git tab read the worktree again: its answer is about the files as they are after that focus.
    await agentChecks.load(agent);
    expect(asked()).toEqual([agent, agent]);
    later(FOCUS_READ_MS * 2);
    expect(asked()).toEqual([agent, agent]);
  });

  it('a focus that waits still asks when the only answer since is to an older question', async () => {
    let answer: (v: AgentCheck) => void = () => {};
    agentChecks.focused(agent);
    // The first question has not answered yet when the second focus comes.
    vi.mocked(api.agentCheckState).mockReturnValueOnce(new Promise((ok) => (answer = ok)));
    later(1000);
    const old = agentChecks.load(agent); // asked before the focus below
    later(1000);
    agentChecks.focused(agent);
    answer(idle);
    await old;
    expect(asked()).toEqual([agent, agent]);
    later(FOCUS_READ_MS);
    expect(asked()).toEqual([agent, agent, agent]);
  });

  it('is by agent: one agent’s focus does not hold back another’s', () => {
    agentChecks.focused(agent);
    agentChecks.focused(`${agent}-other`);
    expect(asked()).toEqual([agent, `${agent}-other`]);
  });

  it('a click on “Check the work” always reads, also right after a focus', async () => {
    const run = vi.spyOn(api, 'checkAgent').mockResolvedValue([]);
    agentChecks.focused(agent);
    await agentChecks.run(agent);
    expect(run).toHaveBeenCalledTimes(1);
    // The focus asked, and the run asked again when it ended.
    expect(asked()).toEqual([agent, agent]);
    await agentChecks.run(agent);
    expect(run).toHaveBeenCalledTimes(2);
    expect(asked()).toEqual([agent, agent, agent]);
  });
});
