// The "Helpers" section of the agent sheet. Before, every helper that was not running counted
// as "finished": also one that you stopped, and one that failed.
import { describe, expect, it } from 'vitest';
import type { BackgroundWork } from '../../lib/state/session';
import { SAME_DESCRIPTION, helperEndWord, helperStop, helpersSummary } from '../helperRows';

const helpers = (...status: string[]) => status.map((s) => ({ status: s }));

describe('how a helper ended, in the word of its row', () => {
  it('"completed" is finished; "stopped" and "killed" are stopped; "failed" is failed', () => {
    expect(['completed', 'stopped', 'killed', 'failed'].map(helperEndWord)).toEqual(['finished', 'stopped', 'stopped', 'failed']);
  });

  it('another word of a CLI stays as it is, on one short line', () => {
    expect(helperEndWord('interrupted')).toBe('interrupted');
    expect(helperEndWord('timed\nout')).toBe('timed out');
    expect(helperEndWord('')).toBe('ended');
  });
});

describe('the line of the "Helpers" section', () => {
  it('counts the helpers by how they stand: a stopped one is not finished', () => {
    expect(helpersSummary(helpers('running'))).toBe('1 running');
    expect(helpersSummary(helpers('completed', 'stopped', 'completed'))).toBe('2 finished · 1 stopped');
    expect(helpersSummary(helpers('failed'))).toBe('1 failed');
    expect(helpersSummary(helpers('killed', 'stopped'))).toBe('2 stopped');
  });

  it('says what runs first, then what finished, stopped and failed, then a CLI\'s other words', () => {
    expect(helpersSummary(helpers('started', 'failed', 'stopped', 'completed', 'running', 'running'))).toBe('2 running · 1 finished · 1 stopped · 1 failed · 1 started');
    expect(helpersSummary([])).toBe('');
  });

  it('the line of the ended ones says how they ended, not "1 ended"', () => {
    expect(helpersSummary(helpers('stopped'))).toBe('1 stopped');
  });
});

// Recorded (docs/research/cli-claude-stream-json-2.1.311-part4.md, section 2): `stop_task` ends a
// helper its agent still waits for, and the agent's turn goes on. Before, only a helper of the
// background list had "Stop".
describe('the “Stop” of a helper’s row', () => {
  const helper = (callId: string, description: string, status = 'running') => ({ callId, description, status });
  const listed = (id: string, description: string, status = 'running'): BackgroundWork => ({ id, kind: 'helper', description, status, runId: 'r1', startedAt: 1, endedAt: null });

  /** The row's "Stop" when `h` is the only helper at work. */
  const alone = (h: ReturnType<typeof helper>, background: BackgroundWork[], turnRuns = false) => helperStop(h, [h], background, 'Eli', turnRuns);

  it('a helper its agent waits for is stopped by its call id, and its turn goes on', () => {
    expect(alone(helper('toolu_1', 'Look at the docs'), [])).toEqual({ id: 'toolu_1', hint: 'Stops this helper. Eli’s turn goes on.' });
  });

  it('a helper in the background list is stopped by its id there, with the hint it has there', () => {
    expect(alone(helper('toolu_2', 'Run the tests'), [listed('task-9', 'Run the tests')])).toEqual({
      id: 'task-9',
      hint: 'Stops this helper. Eli is told and goes on with a turn of its own.',
    });
    // While a turn of the agent runs, it is told in that turn.
    expect(alone(helper('toolu_2', 'Run the tests'), [listed('task-9', 'Run the tests')], true)?.hint).toBe('Stops this helper. Eli is told in the turn that runs.');
    // An entry of the list that ended, or a command of the same words, is not this helper.
    expect(alone(helper('toolu_2', 'Run the tests'), [listed('task-9', 'Run the tests', 'stopped'), { ...listed('cmd', 'Run the tests'), kind: 'command' }])?.id).toBe('toolu_2');
  });

  it('a helper that ended has no “Stop”', () => {
    for (const status of ['completed', 'stopped', 'failed']) expect(alone(helper('toolu_3', 'x', status), [])).toBeNull();
  });

  it('two listed helpers of the same words: which is which is not known, and the row says where to stop them', () => {
    const h = helper('toolu_4', 'Same');
    expect(helperStop(h, [h, helper('toolu_5', 'Same')], [listed('a', 'Same'), listed('b', 'Same')], 'Eli', false)).toEqual({ id: null, hint: SAME_DESCRIPTION });
  });

  // Before, only the listed entries were counted: with one helper of the description in the list
  // and one its agent waits for, the waited helper's row got the listed id, and its "Stop"
  // stopped the other helper.
  it('two helpers of one description, one listed and one waited for: neither row stops the wrong one', () => {
    const inList = helper('toolu_6', 'Look at the docs');
    const waited = helper('toolu_7', 'Look at the docs');
    const both = [inList, waited];
    for (const h of both) expect(helperStop(h, both, [listed('task-1', 'Look at the docs')], 'Eli', false)).toEqual({ id: null, hint: SAME_DESCRIPTION });
    // None listed: each is stopped by its own call id, as today.
    for (const h of both) expect(helperStop(h, both, [], 'Eli', false)?.id).toBe(h.callId);
    // One running helper of the description and one entry: the entry is that helper.
    expect(helperStop(inList, [inList, { ...waited, status: 'completed' }], [listed('task-1', 'Look at the docs')], 'Eli', false)?.id).toBe('task-1');
  });
});
