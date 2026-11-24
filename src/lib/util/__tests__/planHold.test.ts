// The Plan room's eyebrow for a plan held since Codly restarted ("Plan · on hold since Codly restarted").
import { describe, expect, it } from 'vitest';
import { HELD } from '../held';
import { ON_HOLD, onHold } from '../planHold';

const held = { status: 'ready' as const, status_reason: HELD };
const done = { status: 'done' as const, status_reason: null };
const working = { status: 'running' as const, status_reason: null };

describe('onHold: a plan on hold since Codly restarted', () => {
  it('is a running plan with a held task and none that works', () => {
    expect(onHold('running', [done, held])).toBe(true);
    expect(onHold('integrating', [held])).toBe(true);
  });

  it('is not one with a task at work: that plan runs', () => {
    expect(onHold('running', [held, working])).toBe(false);
    expect(onHold('running', [held, { status: 'verifying', status_reason: null }])).toBe(false);
  });

  it('is not one with no held task', () => {
    expect(onHold('running', [done, { status: 'ready', status_reason: null }])).toBe(false);
  });

  it('is not a plan that is not running', () => {
    expect(onHold('awaiting_approval', [held])).toBe(false);
    expect(onHold('done', [held])).toBe(false);
  });

  it('uses the plan card’s words', () => {
    expect(ON_HOLD).toBe('on hold since Codly restarted');
  });
});
