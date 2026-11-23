// A task held since Codly restarted is told apart from one that starts.
import { describe, expect, it } from 'vitest';
import resumeRs from '../../../../crates/codly-core/src/orchestrator/resume.rs?raw';
import { HELD, isHeld } from '../held';

describe('isHeld', () => {
  it('is a ready task with the core’s reason', () => {
    expect(isHeld({ status: 'ready', status_reason: HELD })).toBe(true);
    expect(isHeld({ status: 'ready', status_reason: null })).toBe(false);
    expect(isHeld({ status: 'running', status_reason: HELD })).toBe(false);
  });

  it('uses the core’s words', () => {
    expect(resumeRs).toContain(`pub const HELD: &str = "${HELD}";`);
  });
});
