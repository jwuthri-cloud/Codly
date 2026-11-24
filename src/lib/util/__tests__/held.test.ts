// A task held since Codly restarted is told apart from one that starts. Pins the `HELD` reason
// of crates/codly-core/src/orchestrator/resume.rs: the UI knows a held task by the core's exact
// words, so the two must be the same text (read as a Rust string, whatever its line breaks).
import { describe, expect, it } from 'vitest';
import resumeRs from '../../../../crates/codly-core/src/orchestrator/resume.rs?raw';
import { rustStr } from '../../__tests__/support/source';
import { HELD, isHeld } from '../held';

describe('isHeld', () => {
  it('is a ready task with the core’s reason', () => {
    expect(isHeld({ status: 'ready', status_reason: HELD })).toBe(true);
    expect(isHeld({ status: 'ready', status_reason: null })).toBe(false);
    expect(isHeld({ status: 'running', status_reason: HELD })).toBe(false);
  });

  it('uses the core’s words', () => {
    expect(rustStr(resumeRs, 'HELD')).toBe(HELD);
  });
});
