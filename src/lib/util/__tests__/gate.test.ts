import { describe, expect, it } from 'vitest';
import type { JsonValue } from '../../gen/serde_json/JsonValue';
import type { Decision } from '../../state/store.svelte';
import { stoppedByChecks } from '../gate';

let n = 0;
const decision = (taskId: string | null, type: string, detail: JsonValue, method = 'rule'): Decision => {
  n += 1;
  return { id: `d${n}`, type, method, summary: '', detail, ts: n, agentId: null, taskId, eventId: n };
};

describe('stoppedByChecks', () => {
  it('is true when the gate stopped the task on its checks', () => {
    expect(stoppedByChecks([decision('t1', 'gate', { rounds: 2, failed_check: 'npm test' })], 't1')).toBe(true);
    expect(stoppedByChecks([decision('t1', 'gate', { timed_out: 'npm test' })], 't1')).toBe(true);
    expect(stoppedByChecks([decision('t1', 'gate', { held: [{ command: 'rm -rf dist', risks: ['deletes files'] }] })], 't1')).toBe(true);
  });

  it('is false for any other stop', () => {
    expect(stoppedByChecks([], 't1')).toBe(false);
    // Out of fix rounds on the write scope, or rejected by a reviewer: no check failed.
    expect(stoppedByChecks([decision('t1', 'gate', { rounds: 2, merge_aborted: false })], 't1')).toBe(false);
    expect(stoppedByChecks([decision('t1', 'review', { review: {} })], 't1')).toBe(false);
    // A check failed, then you retried the task and stopped it mid-run: your stop is on record last.
    const stoppedByYou = [decision('t1', 'gate', { rounds: 2, failed_check: 'npm test' }), decision('t1', 'schedule', {}, 'human')];
    expect(stoppedByChecks(stoppedByYou, 't1')).toBe(false);
  });

  it('reads only the task’s own decisions', () => {
    const other = [decision('t2', 'gate', { timed_out: 'npm test' }), decision(null, 'merge', {})];
    expect(stoppedByChecks(other, 't1')).toBe(false);
    expect(stoppedByChecks([decision('t1', 'gate', { timed_out: 'npm test' }), ...other], 't1')).toBe(true);
  });
});
