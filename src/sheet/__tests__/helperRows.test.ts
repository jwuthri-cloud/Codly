// The "Helpers" section of the agent sheet. Before, every helper that was not running counted
// as "finished": also one that you stopped, and one that failed.
import { describe, expect, it } from 'vitest';
import { helperEndWord, helpersSummary } from '../helperRows';

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
});
