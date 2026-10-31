// Needs, moodlets and relationships come only from real events and data.
import { describe, expect, it } from 'vitest';
import type { Agent } from '../../lib/gen/Agent';
import type { EventRow } from '../../lib/gen/EventRow';
import { activityFor, CALLED_IN, headline, levelOf, SimsTracker } from '../sims';
import { breakDelay, breakPreference, reactionTo, traitsOf } from '../traits';
import { BreakRoom } from '../breakroom';
import { emptyCareer } from '../../lib/util/career';

const agent = (id: string, o: Partial<Agent> = {}): Agent =>
  ({ id, name: id, provider: 'claude', role: 'worker', account: null, thread: null, usage: { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, context_used: 50_000, context_window: 200_000, cost_usd: 0.4 }, ...o }) as Agent;
const MIN = 60_000;
const HOUR = 60 * MIN;
const ev = (agent_id: string, kind: EventRow['kind'], ts = 1000, task_id: string | null = null): EventRow => ({ id: 1, ts, agent_id, task_id, run_id: null, kind });

describe('sims', () => {
  it('Focus is the context window left; Energy and morale start full-ish', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')]);
    const sh = s.stats('ada', 2000, null);
    expect(sh.needs.focus).toBeCloseTo(0.75);
    expect(sh.needs.energy).toBe(1);
    expect(sh.mood).toBe('Fine');
  });

  it('work makes it hungry and thirsty; the cooler, the fridge and home refill them; a break goes to eat', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')], 0);
    s.setActivity('ada', 0, 'work');
    const sh = s.stats('ada', 3 * HOUR, null);
    expect(sh.needs.thirst).toBeCloseTo(0.01, 2);
    expect(sh.needs.hunger).toBeCloseTo(0.4, 3);
    expect(sh.moodlets.map((m) => m.key)).toContain('thirsty');
    expect(breakPreference([], { ...sh.needs, energy: 1, morale: 0.65 })).toBe('water');
    s.setActivity('ada', 3 * HOUR, activityFor('water', 'drink'));
    expect(s.stats('ada', 3 * HOUR, null).needs.thirst).toBeCloseTo(0.51, 2);
    s.setActivity('ada', 3 * HOUR, activityFor('fridge', 'drink'));
    expect(s.stats('ada', 3 * HOUR, null).needs.hunger).toBeCloseTo(0.9, 3);
    s.sendHome('ada', 3 * HOUR);
    expect(s.stats('ada', 3 * HOUR + MIN, null).needs).toMatchObject({ hunger: 1, thirst: 1 });
  });

  it('working drains energy over a ~4 h shift (3× slower at stamina 100); idle a little, waiting on you not at all', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada'), agent('leg', { career: { ...emptyCareer(), stamina: 100 } })], 0);
    s.setActivity('ada', 0, 'work');
    s.setActivity('leg', 0, 'work');
    expect(s.stats('ada', 2 * HOUR, null).needs.energy).toBeCloseTo(0.5, 3);
    expect(s.stats('ada', 4 * HOUR, null).needs.energy).toBe(0);
    expect(1 - s.stats('leg', 4 * HOUR, null).needs.energy).toBeCloseTo(0.32, 3); // 8% an hour
    const t = new SimsTracker();
    t.setAgents([agent('ada'), agent('bo')], 0);
    t.setActivity('ada', 0, 'idle');
    t.setActivity('bo', 0, 'wait');
    expect(t.stats('ada', HOUR, null).needs.energy).toBeCloseTo(0.95, 3);
    expect(t.stats('bo', HOUR, null).needs.energy).toBe(1);
  });

  it('a nap on a sofa rests at a third of home speed; a PS5 session lifts morale', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')], 0);
    s.setActivity('ada', 0, 'work');
    s.setActivity('ada', 2 * HOUR, 'nap');
    expect(s.stats('ada', 2 * HOUR + 10 * MIN, null).needs.energy).toBeCloseTo(0.5 + 10 / 30, 3);
    s.setActivity('ada', 3 * HOUR, 'console');
    expect(s.stats('ada', 3 * HOUR + 10 * MIN, null).needs.morale).toBeGreaterThan(0.8);
  });

  it('under 30% it is Tired and morale slides; at 0% Exhausted, it finishes its task, then goes home', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')], 0);
    s.setActivity('ada', 0, 'work');
    let sh = s.stats('ada', 3 * HOUR, null);
    expect(sh.needs.energy).toBeCloseTo(0.25, 3);
    expect(headline(sh)).toMatchObject({ label: 'Tired', emoji: '😴' });
    expect(sh.needs.morale).toBeLessThan(0.62);
    sh = s.stats('ada', 4 * HOUR, null);
    expect(headline(sh)).toMatchObject({ label: 'Exhausted', why: 'Goes home once its task is done' });
    s.tick(4 * HOUR, () => true); // still on its task
    expect(s.stats('ada', 4 * HOUR, null).rest.home).toBeNull();
    s.tick(4 * HOUR + 1000, () => false);
    sh = s.stats('ada', 4 * HOUR + 1000, null);
    expect(sh.rest.home).toMatchObject({ until: 4 * HOUR + 1000 + 10 * MIN, sick: false });
    expect(headline(sh)).toMatchObject({ emoji: '🏠', why: 'Back in 10 min' });
    expect(s.drain()).toEqual({ home: [{ id: 'ada', until: 4 * HOUR + 1000 + 10 * MIN }], rest: [{ id: 'ada', sick: false }], tired: [{ id: 'ada', tired: true }] });
  });

  it('home refills it in ~10 min at stamina 0 and ~3 min at stamina 100; then it walks back in', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada'), agent('leg', { career: { ...emptyCareer(), stamina: 100 } })], 0);
    for (const id of ['ada', 'leg']) {
      s.setEnergy(id, 0, 0);
      expect(s.sendHome(id, 0)).toBe(true);
    }
    expect(s.stats('ada', 0, null).rest.home!.until).toBe(10 * MIN);
    expect(s.stats('leg', 0, null).rest.home!.until).toBe(3 * MIN);
    s.drain();
    s.tick(3 * MIN, () => false);
    expect(s.stats('leg', 3 * MIN, null)).toMatchObject({ needs: { energy: 1 }, rest: { home: null } });
    expect(s.stats('ada', 3 * MIN, null).rest.home).not.toBeNull();
    s.tick(10 * MIN, () => false);
    const sh = s.stats('ada', 10 * MIN, null);
    expect(sh.needs.energy).toBeCloseTo(1, 5);
    expect(sh.moodlets.some((m) => m.label === 'Rested')).toBe(true);
    expect(s.drain().home).toEqual([{ id: 'leg', until: null }, { id: 'ada', until: null }]);
  });

  it('calling it in early is Overtime: morale −0.15 and half the energy it banked at home', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')], 0);
    s.setEnergy('ada', 0, 0);
    s.sendHome('ada', 0);
    const before = s.stats('ada', 5 * MIN, null);
    expect(before.needs.energy).toBeCloseTo(0.5, 3);
    expect(s.callIn('ada', 5 * MIN)).toBe(true);
    const sh = s.stats('ada', 5 * MIN, null);
    expect(sh.needs.energy).toBeCloseTo(0.25, 3);
    expect(sh.needs.morale).toBeCloseTo(before.needs.morale - 0.15, 2);
    expect(sh.moodlets.find((m) => m.key === 'rest')).toMatchObject({ label: 'Overtime', emoji: '😩', why: 'Called in early from home' });
    expect(sh.rest.home).toBeNull();
    // At home and given work by you: it comes in (on overtime) to do it.
    s.sendHome('ada', 6 * MIN);
    s.tick(7 * MIN, () => true);
    expect(s.stats('ada', 7 * MIN, null).rest.home).toBeNull();
    expect(s.drain().home.at(-1)).toEqual({ id: 'ada', until: null });
  });

  it('a queued task that needs it at home calls it in, and says why; one starting as it comes back does not', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')], 0);
    s.setEnergy('ada', 0, 0);
    s.sendHome('ada', 0);
    s.tick(4 * MIN, (id) => (id === 'ada' ? 'task' : false));
    const sh = s.stats('ada', 4 * MIN, null);
    expect(sh.rest.home).toBeNull();
    expect(sh.moodlets.find((m) => m.key === 'rest')).toMatchObject({ label: 'Overtime', why: CALLED_IN });
    expect(CALLED_IN).toBe('Called in: nobody else was free');
    // Back on time: the task the core starts as it walks in is no Overtime.
    s.setEnergy('ada', 5 * MIN, 0);
    s.sendHome('ada', 5 * MIN);
    s.tick(15 * MIN, () => 'task');
    expect(s.stats('ada', 15 * MIN, null).moodlets.find((m) => m.key === 'rest')).toMatchObject({ label: 'Rested' });
  });

  it('tells the core once when it gets tired (under 30%) and once when it is rested again', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada'), agent('lead', { role: 'lead' })], 0);
    s.setActivity('ada', 0, 'work');
    s.setActivity('lead', 0, 'work');
    s.tick(2 * HOUR, () => true);
    expect(s.drain().tired).toEqual([]);
    s.tick(3 * HOUR, () => true);
    s.tick(3 * HOUR + 2000, () => true);
    expect(s.drain().tired).toEqual([{ id: 'ada', tired: true }]); // once, and never the Lead
    // The core's row now says so: nothing more to tell, until it is rested again.
    s.setAgents([agent('ada', { tired_since: 3 * HOUR }), agent('lead', { role: 'lead' })], 3 * HOUR);
    s.tick(3 * HOUR + 4000, () => true);
    expect(s.drain().tired).toEqual([]);
    s.setEnergy('ada', 3 * HOUR + 5000, 1);
    s.tick(3 * HOUR + 6000, () => true);
    expect(s.drain().tired).toEqual([{ id: 'ada', tired: false }]);
    // Going home is resting: a tired agent sent home is told rested.
    const t = new SimsTracker();
    t.setAgents([agent('bo', { tired_since: 1 })], 0);
    t.setEnergy('bo', 0, 0.1);
    t.sendHome('bo', 0);
    t.tick(1000, () => false);
    expect(t.drain().tired).toEqual([{ id: 'bo', tired: false }]);
  });

  it('the Lead never goes home: a tired Lead only looks tired', () => {
    const s = new SimsTracker();
    s.setAgents([agent('lead', { role: 'lead', away_until: HOUR })], 0);
    s.setActivity('lead', 0, 'work');
    expect(s.sendHome('lead', 0)).toBe(false);
    s.tick(5 * HOUR, () => false);
    const sh = s.stats('lead', 5 * HOUR, null);
    expect(sh.rest).toMatchObject({ home: null, canGoHome: false, exhausted: true });
    expect(headline(sh)).toMatchObject({ label: 'Exhausted', why: 'Running on empty' });
    expect(s.drain()).toEqual({ home: [], rest: [], tired: [] });
  });

  it('coffee: +15% and a cup of caffeine; 3 cups make it jittery, and they wear off in an hour', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')], 0);
    s.setEnergy('ada', 0, 0.5);
    const cup = (t: number) => {
      s.setActivity('ada', t, 'coffee');
      s.setActivity('ada', t + 30_000, 'wait');
    };
    cup(0);
    expect(s.stats('ada', 30_000, null).needs.energy).toBeCloseTo(0.65, 3);
    cup(2 * MIN);
    expect(s.stats('ada', 3 * MIN, null).rest).toMatchObject({ caffeine: 2, jittery: false });
    cup(4 * MIN);
    const sh = s.stats('ada', 5 * MIN, null);
    expect(sh.rest.jittery).toBe(true);
    expect(headline(sh)).toMatchObject({ label: 'Jittery', emoji: '⚡' });
    expect(s.stats('ada', 61 * MIN, null).rest.jittery).toBe(false);
    // A Legend takes more: jittery at 6 cups.
    const l = new SimsTracker();
    l.setAgents([agent('leg', { career: { ...emptyCareer(), stamina: 100 } })], 0);
    for (let i = 0; i < 5; i++) l.setActivity('leg', i * 2 * MIN, 'coffee'), l.setActivity('leg', i * 2 * MIN + 1000, 'wait');
    expect(l.stats('leg', 10 * MIN, null).rest).toMatchObject({ caffeine: 5, jittery: false });
    l.setActivity('leg', 10 * MIN, 'coffee');
    expect(l.stats('leg', 10 * MIN, null).rest.jittery).toBe(true);
  });

  it('5 cups since it last went home make it Sick: home for 30 min, then 60% at most for a shift', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')], 0);
    for (let i = 0; i < 5; i++) {
      s.setActivity('ada', i * 20 * MIN, 'coffee');
      s.setActivity('ada', i * 20 * MIN + 1000, 'wait');
    }
    const now = 81 * MIN;
    let sh = s.stats('ada', now, null);
    expect(sh.rest.sick).toBe(true);
    expect(sh.moodlets.find((m) => m.key === 'sick')).toMatchObject({ label: 'Sick', why: 'Too much coffee, not enough sleep', emoji: '🤒' });
    expect(s.drain().rest).toEqual([{ id: 'ada', sick: true }]);
    s.tick(now, () => false);
    expect(s.stats('ada', now, null).rest.home).toEqual({ until: now + 30 * MIN, sick: true });
    s.tick(now + 30 * MIN, () => false);
    sh = s.stats('ada', now + 30 * MIN, null);
    expect(sh.rest).toMatchObject({ home: null, sick: false, cap: 0.6 });
    expect(sh.needs.energy).toBeCloseTo(0.6, 5);
    s.setActivity('ada', now + 30 * MIN, 'nap');
    expect(s.stats('ada', now + 2 * HOUR, null).needs.energy).toBeCloseTo(0.6, 5); // capped this shift
    s.sendHome('ada', now + 2 * HOUR);
    s.tick(now + 3 * HOUR, () => false);
    expect(s.stats('ada', now + 3 * HOUR, null)).toMatchObject({ needs: { energy: 1 }, rest: { cap: 1 } }); // a normal night: the cap is gone
  });

  it('two shifts in a row without going home make it sick too', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')], 0);
    s.setActivity('ada', 0, 'work');
    s.stats('ada', 4 * HOUR, null);
    s.setEnergy('ada', 4 * HOUR, 1); // say, naps and coffee kept it going
    expect(s.stats('ada', 7 * HOUR, null).rest.sick).toBe(false);
    expect(s.stats('ada', 8 * HOUR, null).rest.sick).toBe(true);
  });

  it('dev hooks: fill or drain energy, skip time (home timers follow); a reload keeps it all', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada'), agent('bo')], 0);
    s.setEnergy('ada', 0, 0);
    expect(s.stats('ada', 0, null).rest.exhausted).toBe(true);
    s.sendHome('ada', 0);
    s.drain();
    s.skip(4, 0);
    expect(s.drain().home).toEqual([{ id: 'ada', until: 6 * MIN }]);
    expect(s.stats('ada', 0, null).needs.energy).toBeCloseTo(0.4, 3);
    s.setActivity('bo', 0, 'work');
    s.skip(60, 0);
    expect(s.stats('bo', 0, null).needs.energy).toBeCloseTo(0.75, 3);
    const saved = s.save();
    const t = new SimsTracker();
    t.setAgents([agent('ada'), agent('bo')], 10 * HOUR);
    t.load(saved, 10 * HOUR);
    expect(t.stats('bo', 10 * HOUR, null).needs.energy).toBeCloseTo(0.75, 3); // the office was closed: no drain
    expect(t.stats('ada', 10 * HOUR, null).rest.home).not.toBeNull();
    t.tick(10 * HOUR, () => false);
    expect(t.stats('ada', 10 * HOUR, null)).toMatchObject({ needs: { energy: 1 }, rest: { home: null } });
    // The core's away_until (another window, a restart without saved state) puts it at home too.
    const u = new SimsTracker();
    u.setAgents([agent('ada', { away_until: 5 * MIN })], 0);
    expect(u.stats('ada', 0, null).rest.home).toMatchObject({ until: 5 * MIN });
  });

  it('activityFor maps the office spots and poses to what the agent is doing', () => {
    expect(activityFor('nap.armchair', 'sleep')).toBe('nap');
    expect(activityFor('kitchen', 'drink')).toBe('coffee');
    expect(activityFor('games.console', 'gamepad')).toBe('console');
    expect(activityFor(null, 'type')).toBe('work');
    expect(activityFor(null, 'sit')).toBe('idle');
  });

  it('passing tests lift morale with a moodlet; failing ones do the opposite; morale drifts back', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')]);
    s.ingest(ev('ada', { type: 'tool_completed', call_id: 'c', ok: true, output: null, exit_code: 0, duration_ms: 1, tests_passed: 12, tests_failed: 0 }));
    let sh = s.stats('ada', 1000, null);
    expect(sh.needs.morale).toBeGreaterThan(0.7);
    expect(sh.moodlets[0]).toMatchObject({ label: 'Tests passed', tone: 'good' });
    s.ingest(ev('ada', { type: 'tool_completed', call_id: 'd', ok: false, output: null, exit_code: 1, duration_ms: 1, tests_passed: 3, tests_failed: 2 }, 2000));
    sh = s.stats('ada', 2000, null);
    expect(sh.moodlets[0].label).toBe('Tests failing');
    expect(s.stats('ada', 2000 + 3_600_000, null).needs.morale).toBeCloseTo(0.65, 1); // an hour later
  });

  it('two failed checks in a row make it Frustrated; a pass clears it; a merge makes it Proud', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada'), agent('rev')]);
    s.setTasks([{ id: 't1', assignee: 'ada', status: 'running' } as never]);
    const check = (ok: boolean, ts: number) => s.ingest(ev('ada', { type: 'verified', command: 'pnpm test', ok, exit_code: ok ? 0 : 1, tail: '' }, ts, 't1'));
    check(false, 1000);
    expect(s.stats('ada', 1000, null).moodlets[0].label).toBe('Checks failing');
    check(false, 2000);
    let sh = s.stats('ada', 2000, null);
    expect(sh.moodlets[0]).toMatchObject({ label: 'Frustrated', why: '2 failures in a row' });
    expect(headline(sh)?.emoji).toBe('😤');
    check(true, 3000);
    sh = s.stats('ada', 3000, null);
    expect(sh.moodlets[0].label).toBe('Checks passed');
    expect(headline(sh)).toBeNull();
    // The review is the reviewer's event, but the feeling is the author's.
    s.ingest(ev('rev', { type: 'reviewed', verdict: 'approve', summary: '' }, 4000, 't1'));
    expect(headline(s.stats('ada', 4000, null))?.label).toBe('Approved');
    expect(s.stats('rev', 4000, null).moodlets).toHaveLength(0);
    s.ingest(ev('lead', { type: 'merged', sha: 'abc', branch: 'main' }, 5000, 't1'));
    sh = s.stats('ada', 5000, null);
    expect(headline(sh)).toMatchObject({ label: 'Proud', emoji: '🏆' });
    expect(headline(s.stats('ada', 5000 + 16 * 60_000, null))?.label).not.toBe('Proud'); // it fades
  });

  it('skill levels grow like a Sims skill bar', () => {
    expect(levelOf(4)).toEqual({ level: 2, progress: 0 });
    expect(levelOf(100)).toEqual({ level: 10, progress: 1 });
  });

  it('friends are the reviews saved on its career', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada', { career: { ...emptyCareer(), friends: { rev: 3, bob: 1 } } }), agent('rev')]);
    const r = s.stats('ada', 0, null).relations;
    expect(r.map((x) => x.agentId)).toEqual(['rev', 'bob']);
    expect(r[0]).toMatchObject({ why: '3 reviews between them' });
    expect(s.stats('rev', 0, null).relations).toEqual([]);
  });
});

describe('traits', () => {
  it('two stable traits per agent, never a clashing pair', () => {
    for (const id of ['a', 'b', 'c', 'agent-42', 'zzz']) {
      const t = traitsOf(id);
      expect(t).toEqual(traitsOf(id));
      expect(t.length).toBe(2);
      expect(t.includes('lazy') && t.includes('perfectionist')).toBe(false);
    }
  });

  it('needs come first, then personality; reactions and break timing follow traits', () => {
    expect(breakPreference(['bookworm', 'clumsy'], { energy: 0.1, morale: 0.7 })).toBe('nap');
    expect(breakPreference(['bookworm', 'clumsy'], { energy: 0.4, morale: 0.7 })).toBe('coffee');
    expect(breakPreference(['bookworm', 'clumsy'], { energy: 0.4, morale: 0.7, jittery: true })).toBe('library'); // no 4th cup
    expect(breakPreference(['bookworm', 'clumsy'], { energy: 0.9, morale: 0.2 })).toBe('console');
    expect(breakPreference(['bookworm', 'clumsy'], null)).toBe('library');
    expect(reactionTo(['hotheaded', 'social'], false)).toBe('thump');
    expect(reactionTo(['hotheaded', 'social'], true)).toBe('wave');
    expect(breakDelay(['lazy', 'social'])).toBeLessThan(1);
    expect(breakDelay(['nightowl', 'clumsy'], 9)).toBeLessThan(breakDelay(['nightowl', 'clumsy'], 22));
  });

  it('a bookworm reads in the library and a tired agent gets coffee, while the others still pair up', () => {
    const r = new BreakRoom();
    const s = r.update([{ id: 'book', prefer: 'library' }, { id: 'tired', prefer: 'coffee' }, 'a', 'b'], 0);
    expect(s.get('book')).toEqual({ game: 'library', slot: 0 });
    expect(s.get('tired')).toEqual({ game: 'coffee', slot: 0 });
    expect(s.get('a')!.game).toBe(s.get('b')!.game);
  });
});
