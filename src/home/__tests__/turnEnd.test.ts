// A row of thoughts only, in a turn that failed or was stopped before the agent answered, does
// not say "Thought it through" (fifth browser review, defect 19).
import { describe, expect, it } from 'vitest';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import { endedBeforeAnswer } from '../turnEnd';

let id = 0;
const row = (agent: string, kind: EventKind): EventRow => ({ id: ++id, ts: id * 10, agent_id: agent, kind }) as unknown as EventRow;
const turn = (agent: string, status: 'completed' | 'failed' | 'interrupted', said: boolean) => {
  const start = row(agent, { type: 'turn_started', input: 'hi' } as EventKind);
  const thought = row(agent, { type: 'reasoning', item_id: `r${id}`, text: 'Look at the tokens first' } as EventKind);
  const rows = [start, thought, ...(said ? [row(agent, { type: 'message', item_id: `m${id}`, text: 'Done.' } as EventKind)] : [])];
  return { rows: [...rows, row(agent, { type: 'turn_completed', status } as EventKind)], thought: thought.id };
};

describe('endedBeforeAnswer', () => {
  it('a turn that failed or was stopped before any answer: its thoughts were cut', () => {
    const failed = turn('ada', 'failed', false);
    expect(endedBeforeAnswer(failed.rows, failed.thought)).toBe(true);
    const stopped = turn('ada', 'interrupted', false);
    expect(endedBeforeAnswer(stopped.rows, stopped.thought)).toBe(true);
    // Its process went away with the turn.
    const start = row('bo', { type: 'turn_started', input: 'hi' } as EventKind);
    const thought = row('bo', { type: 'reasoning', item_id: 'r', text: 'x' } as EventKind);
    expect(endedBeforeAnswer([start, thought, row('bo', { type: 'exited', code: 1 } as EventKind)], thought.id)).toBe(true);
  });

  it('a turn that answered, that ended well, or that still runs: the thoughts read as before', () => {
    const answered = turn('ada', 'failed', true);
    expect(endedBeforeAnswer(answered.rows, answered.thought)).toBe(false);
    const ok = turn('ada', 'completed', false);
    expect(endedBeforeAnswer(ok.rows, ok.thought)).toBe(false);
    const running = turn('ada', 'failed', false);
    expect(endedBeforeAnswer(running.rows.slice(0, -1), running.thought)).toBe(false);
    expect(endedBeforeAnswer(running.rows, 99_999)).toBe(false);
  });

  it('reads the turn of the agent that thought, not another agent’s', () => {
    const ada = turn('ada', 'completed', false);
    const bo = turn('bo', 'failed', false);
    const log = [ada.rows[0], bo.rows[0], ada.rows[1], bo.rows[1], ada.rows[2], bo.rows[2]];
    expect(endedBeforeAnswer(log, ada.thought)).toBe(false);
    expect(endedBeforeAnswer(log, bo.thought)).toBe(true);
  });
});
