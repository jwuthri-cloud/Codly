import { describe, expect, it } from 'vitest';
import { oneAtATime } from '../util/oneAtATime';

/** A job the test ends by hand: `starts` counts its runs, `end()` ends the one that runs. */
function slowJob() {
  const ends: (() => void)[] = [];
  let starts = 0;
  let open = 0;
  let overlap = false;
  const job = () =>
    new Promise<void>((resolve) => {
      starts++;
      if (++open > 1) overlap = true;
      ends.push(() => {
        open--;
        resolve();
      });
    });
  const end = async () => {
    ends.shift()!();
    await new Promise((r) => setTimeout(r));
  };
  return { job, end, starts: () => starts, overlap: () => overlap };
}

describe('oneAtATime', () => {
  it('runs once for one call, and again for a later call', async () => {
    const j = slowJob();
    const run = oneAtATime(j.job);
    void run();
    expect(j.starts()).toBe(1);
    await j.end();
    void run();
    expect(j.starts()).toBe(2);
    await j.end();
    expect(j.overlap()).toBe(false);
  });

  it('a burst of calls during a run makes one more run, after it', async () => {
    const j = slowJob();
    const run = oneAtATime(j.job);
    void run();
    for (let i = 0; i < 20; i++) void run();
    expect(j.starts()).toBe(1); // nothing overlaps the run in flight
    await j.end();
    expect(j.starts()).toBe(2); // the one run that reads the state after the last call
    void run();
    await j.end();
    expect(j.starts()).toBe(3);
    await j.end();
    expect(j.starts()).toBe(3);
    expect(j.overlap()).toBe(false);
  });

  it('the promise of every call ends when the last run has ended', async () => {
    const j = slowJob();
    const run = oneAtATime(j.job);
    let done = 0;
    void run().then(() => done++);
    void run().then(() => done++);
    await j.end();
    expect(done).toBe(0);
    await j.end();
    expect(done).toBe(2);
  });
});
