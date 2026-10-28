// Needs, moodlets, skills and relationships come only from real events and data.
import { describe, expect, it } from 'vitest';
import type { Agent } from '../../lib/gen/Agent';
import type { EventRow } from '../../lib/gen/EventRow';
import type { Task } from '../../lib/gen/Task';
import { levelOf, SimsTracker, skillFor } from '../sims';
import { breakDelay, breakPreference, reactionTo, traitsOf } from '../traits';
import { BreakRoom } from '../breakroom';

const agent = (id: string, o: Partial<Agent> = {}): Agent =>
  ({ id, name: id, provider: 'claude', role: 'worker', account: null, thread: null, usage: { input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, context_used: 50_000, context_window: 200_000, cost_usd: 0.4 }, ...o }) as Agent;
const ev = (agent_id: string, kind: EventRow['kind'], ts = 1000, task_id: string | null = null): EventRow => ({ id: 1, ts, agent_id, task_id, run_id: null, kind });

describe('sims', () => {
  it('Focus is the context window left; Energy is unknown without account data; morale starts fine', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')]);
    const sh = s.stats('ada', 2000, null);
    expect(sh.needs.focus).toBeCloseTo(0.75);
    expect(sh.needs.energy).toBeNull();
    expect(sh.mood).toBe('Fine');
  });

  it('Energy is the quota left on its account', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada', { account: 'claude' })]);
    const acc = { id: 'claude', provider: 'claude', limits: [{ window: 'five_hour', used_fraction: 0.8, resets_at: null, status: 'allowed', seen_at: 0 }] } as never;
    expect(s.stats('ada', 0, null, [acc]).needs.energy).toBeCloseTo(0.2);
  });

  it('passing tests lift morale with a moodlet and grow Testing; failing ones do the opposite; morale drifts back', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada')]);
    s.ingest(ev('ada', { type: 'tool_completed', call_id: 'c', ok: true, output: null, exit_code: 0, duration_ms: 1, tests_passed: 12, tests_failed: 0 }));
    let sh = s.stats('ada', 1000, null);
    expect(sh.needs.morale).toBeGreaterThan(0.7);
    expect(sh.moodlets[0]).toMatchObject({ label: 'Tests passed', tone: 'good' });
    expect(sh.skills[0].name).toBe('Testing');
    s.ingest(ev('ada', { type: 'tool_completed', call_id: 'd', ok: false, output: null, exit_code: 1, duration_ms: 1, tests_passed: 3, tests_failed: 2 }, 2000));
    sh = s.stats('ada', 2000, null);
    expect(sh.moodlets[0].label).toBe('Tests failing');
    expect(s.stats('ada', 2000 + 3_600_000, null).needs.morale).toBeCloseTo(0.65, 1); // an hour later
  });

  it('skills come from the files it changes', () => {
    expect(skillFor('src/app/x.tsx')).toBe('TypeScript');
    expect(skillFor('crates/a/src/lib.rs')).toBe('Rust');
    expect(skillFor('src/a.test.ts')).toBe('Testing');
    const s = new SimsTracker();
    s.ingest(ev('ada', { type: 'files_changed', call_id: null, paths: ['a.ts', 'b.ts', 'c.ts', 'd.ts'], source: 'tool_input' }));
    expect(s.stats('ada', 0, null).skills[0]).toMatchObject({ name: 'TypeScript', level: 2 });
    expect(levelOf(100)).toEqual({ level: 10, progress: 1 });
  });

  it('a reviewer and the author it reviews get to know each other', () => {
    const s = new SimsTracker();
    s.setAgents([agent('ada'), agent('rev', { role: 'reviewer' })]);
    s.setTasks([{ id: 't1', assignee: 'ada', status: 'reviewing' } as Task]);
    s.ingest(ev('rev', { type: 'turn_completed', status: 'completed', result_text: null, duration_ms: 1 }, 1000, 't1'));
    expect(s.stats('ada', 0, null).relations[0]).toMatchObject({ agentId: 'rev' });
    expect(s.stats('rev', 0, null).relations[0].score).toBeGreaterThan(0);
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
    expect(breakPreference(['bookworm', 'clumsy'], { energy: 0.1, morale: 0.7 })).toBe('coffee');
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
