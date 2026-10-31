import { describe, expect, it } from 'vitest';
import { everyLabel, inLabel, nextRunAt } from '../util/triggers';
import { saveTriggers, seedTriggers } from '../demo/triggers';
import type { Schedule } from '../gen/Schedule';

const MIN = 60_000;
const sched = (o: Partial<Schedule> = {}): Schedule => ({ id: 's', name: 'n', prompt: 'p', every_minutes: 60, enabled: true, created_at: 0, last_fired_at: null, ...o });

describe('triggers', () => {
  it('next run is one interval after the last firing, or after creation', () => {
    expect(nextRunAt(sched())).toBe(60 * MIN);
    expect(nextRunAt(sched({ last_fired_at: 100 * MIN }))).toBe(160 * MIN);
  });

  it('labels intervals and countdowns', () => {
    expect(everyLabel(30)).toBe('30 min');
    expect(everyLabel(60)).toBe('hour');
    expect(everyLabel(90)).toBe('1 h 30 min');
    expect(everyLabel(240)).toBe('4 hours');
    expect(everyLabel(1440)).toBe('day');
    expect(inLabel(0, 5 * MIN)).toBe('due now');
    expect(inLabel(35 * MIN, 0)).toBe('in 35 min');
    expect(inLabel(150 * MIN, 0)).toBe('in 2 h 30 min');
  });

  it('the mock keeps the core’s clocks and clamps what the UI sends', () => {
    const old = seedTriggers(1_000 * MIN);
    const next = structuredClone(old);
    next.schedules[0].last_fired_at = 5; // the UI cannot move it
    next.schedules.push(sched({ id: '', name: ' ', prompt: ' standup ', every_minutes: 1 }));
    const out = saveTriggers(old, next, 1_000 * MIN);
    expect(out.schedules[0].last_fired_at).toBe(old.schedules[0].last_fired_at);
    expect(out.schedules[2].id).not.toBe('');
    expect(out.schedules[2].every_minutes).toBe(5);
    expect(out.schedules[2].name).toBe('standup');
    expect(out.state).toBe(old.state);
  });
});
