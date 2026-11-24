// Send back opens a new request when the core's refusal holds one phrase (actions.ts
// `NEW_REQUEST`). The core writes it in two places, and the demo answers with it too: one change
// of the words in any of them would make Send back show an error where it opened a request.
import { describe, expect, it } from 'vitest';
import reviewBackRs from '../../../crates/codly-core/src/orchestrator/review_back.rs?raw';
import workspaceRs from '../../../crates/codly-core/src/orchestrator/workspace.rs?raw';
import { NEW_REQUEST as DEMO_NEW_REQUEST, doneTaskRefusal } from '../demo/reviewTask';
import type { Sim } from '../demo/sim';
import type { Task } from '../gen/Task';
import { NEW_REQUEST } from '../state/actions';
import { rustStr } from './support/source';

describe('the phrase that sends review comments to a new request', () => {
  it('is the one the core refuses with', () => {
    expect(rustStr(reviewBackRs, 'NEW_REQUEST')).toBe(NEW_REQUEST);
    // `send_back_task` on a done task whose work is merged.
    expect(workspaceRs).toContain(`is done and its work is merged; ${NEW_REQUEST}"`);
  });

  it('is the one the demo refuses with', () => {
    expect(DEMO_NEW_REQUEST).toBe(NEW_REQUEST);
    const sim = { plans: new Map() } as unknown as Sim;
    expect(doneTaskRefusal(sim, { key: 't1', plan_id: null } as Task)).toBe(`t1 is done and its work is merged; ${NEW_REQUEST}`);
  });
});
