// The games are played for real: goals both ways at foosball, turns and potted balls at pool,
// a whole match on the console. Seeded, so these runs are the same every time.
import { describe, expect, it } from 'vitest';
import { aroundTable, ConsoleSim, FoosballSim, PoolSim } from '../gfx/games';

const DT = 1 / 60;

describe('foosball', () => {
  it('both sides score over a few minutes, the ball stays on the table, cues follow the ball', () => {
    const sim = new FoosballSim(42);
    let now = 0;
    let shots = 0;
    let lastShot = -1;
    for (let f = 0; f < 60 * 240; f++) {
      now += DT * 1000;
      sim.step(now, DT, [true, true]);
      expect(Math.abs(sim.ball.x)).toBeLessThanOrEqual(0.43);
      expect(Math.abs(sim.ball.z)).toBeLessThanOrEqual(0.3);
      if (sim.lastShot && sim.lastShot.at !== lastShot) {
        shots++;
        lastShot = sim.lastShot.at;
      }
    }
    expect(sim.score[0]).toBeGreaterThan(0);
    expect(sim.score[1]).toBeGreaterThan(0);
    expect(shots).toBeGreaterThan(20);
    // The two players see the ball on opposite sides (they face each other).
    sim.ball.z = 0.2;
    expect(Math.sign(sim.cue(0, -1).lean)).toBe(-Math.sign(sim.cue(1, 1).lean));
  });
});

describe('pool', () => {
  it('two players take turns, balls go down, the table is re-racked, and everyone reacts', () => {
    const sim = new PoolSim(7);
    let now = 0;
    const turns = new Set<number>();
    const reactions = new Set<string>();
    let potted = 0;
    for (let f = 0; f < 60 * 600; f++) {
      now += DT * 1000;
      sim.step(now, DT, 2, true);
      turns.add(sim.turn);
      if (sim.result) reactions.add(sim.result.kind);
      potted = Math.max(potted, sim.balls.filter((b, i) => i > 0 && b.in).length);
      for (const b of sim.balls) {
        expect(Math.abs(b.x)).toBeLessThanOrEqual(0.9);
        expect(Math.abs(b.z)).toBeLessThanOrEqual(0.46);
      }
    }
    expect(turns).toEqual(new Set([0, 1]));
    expect(potted).toBeGreaterThan(0);
    expect(reactions).toEqual(new Set(['cheer', 'groan']));
  });

  it('the shooter stands behind the cue ball, outside the table, facing the shot', () => {
    const shot = new PoolSim(3).plan();
    const out = Math.abs(shot.spot.x) > 1.0 || Math.abs(shot.spot.z) > 0.55;
    expect(out).toBe(true);
    expect(Math.abs(shot.spot.face - Math.atan2(shot.vx, shot.vz))).toBeLessThan(1e-9);
  });

  it('walks round the table, never across it', () => {
    expect(aroundTable([0, -1.2], [0.3, -1.2], 1.4, 0.95)).toEqual([]);
    const via = aroundTable([0, -1.2], [0.2, 1.2], 1.4, 0.95);
    expect(via).toHaveLength(2);
    for (const [x, z] of via) expect([Math.abs(x), Math.abs(z)]).toEqual([1.4, 0.95]);
  });
});

describe('console', () => {
  it('a match reaches full time with goals and reactions, then a new one starts', () => {
    const sim = new ConsoleSim(5);
    let now = 0;
    const reacts = new Set<string>();
    while (!sim.fullTime && now < 120_000) {
      now += DT * 1000;
      sim.step(now, DT);
      const r = sim.cue(0, now).react;
      if (r) reacts.add(r.kind);
    }
    expect(sim.fullTime).not.toBeNull();
    expect(now / 1000).toBeGreaterThan(70);
    expect(reacts.size).toBeGreaterThan(0);
    for (let f = 0; f < 60 * 5; f++) {
      now += DT * 1000;
      sim.step(now, DT);
    }
    expect(sim.fullTime).toBeNull();
    expect(sim.clock).toBeLessThan(0.02);
  });
});
