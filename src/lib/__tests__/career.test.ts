import { describe, expect, it } from 'vitest';
import { canTrust, clashes, contextLabel, emptyCareer, makeApplicants, rankFor, rankProgress, TRUST, trustOf } from '../util/career';
import { demoBiggerModel, demoTrust } from '../demo/office';
import type { Sim } from '../demo/sim';
import type { Agent } from '../gen/Agent';

describe('careers', () => {
  it('applicants are distinct, free and sensible', () => {
    const a = makeApplicants(3, ['Ada', 'Bo'], [0, 1], 42);
    expect(a.map((x) => x.name)).toEqual(['Cleo', 'Dax', 'Eli']);
    expect(new Set(a.map((x) => x.color)).size).toBe(3);
    for (const x of a) {
      expect(x.color).toBeGreaterThan(1);
      expect(x.traits).toHaveLength(2);
      expect(clashes(x.traits[0], x.traits[1])).toBe(false);
    }
  });

  it('ranks from XP, as the core', () => {
    expect([0, 59, 60, 249, 250, 800, 5000].map(rankFor)).toEqual(['intern', 'intern', 'junior', 'junior', 'senior', 'staff', 'staff']);
  });

  it('progress to the next rank', () => {
    const c = { ...emptyCareer(), xp: 30 };
    expect(rankProgress(c)).toEqual({ next: 'Junior', progress: 0.5, toGo: 30 });
    expect(rankProgress({ ...c, xp: 900, rank: 'staff' }).next).toBeNull();
  });
});

describe('a new Legend’s offer', () => {
  it('names the context window short', () => {
    expect(contextLabel(1_000_000)).toBe('1M');
    expect(contextLabel(1_500_000)).toBe('1.5M');
    expect(contextLabel(272_000)).toBe('272k');
  });
  it('the demo offers Claude’s 1M window only when its own is smaller, like the core', () => {
    const agents = new Map<string, Agent>();
    const sim = { agents } as unknown as Sim;
    const a = { id: 'a', provider: 'claude', model: 'sonnet', usage: { context_window: 200_000 } } as unknown as Agent;
    agents.set('a', a);
    expect(demoBiggerModel(sim, 'a')).toEqual({ model: 'opus[1m]', context_window: 1_000_000 });
    agents.set('a', { ...a, model: 'sonnet[1m]:high' });
    expect(demoBiggerModel(sim, 'a')).toBeNull();
    agents.set('a', { ...a, usage: { ...a.usage, context_window: 1_000_000 } });
    expect(demoBiggerModel(sim, 'a')).toBeNull();
    agents.set('a', { ...a, provider: 'gemini' });
    expect(demoBiggerModel(sim, 'a')).toBeNull();
  });
});

describe('trust is a safety setting, not a rank', () => {
  it('names every level in plain words, the safe one first', () => {
    expect(TRUST.map((t) => t.level)).toEqual(['intern', 'junior', 'senior', 'staff']);
    expect(trustOf('intern').label).toBe('No extra trust');
    for (const t of TRUST) {
      expect(t.label).not.toMatch(/intern|junior|senior|staff/i);
      expect(t.allows.length).toBeGreaterThan(20);
    }
    // The two middle levels only change anything where permissions are Balanced, and say so.
    expect(trustOf('junior').allows).toContain('Balanced');
    expect(trustOf('senior').allows).toContain('Balanced');
    expect(trustOf('staff').allows).toContain('without the cross-vendor review');
  });

  it('only agents that write on their own can be trusted', () => {
    expect((['lead', 'worker', 'reviewer', 'solo'] as const).filter(canTrust)).toEqual(['worker', 'solo']);
  });

  it('the demo sets any level whatever the rank, and records each change as a decision', () => {
    const agents = new Map<string, Agent>();
    const decisions: unknown[][] = [];
    const sim = { agents, now: () => 7, putAgent: (a: Agent) => agents.set(a.id, a), decision: (...d: unknown[]) => decisions.push(d) } as unknown as Sim;
    agents.set('ana', { id: 'ana', name: 'Ana', role: 'worker', career: emptyCareer() } as unknown as Agent);
    agents.set('lead', { id: 'lead', name: 'Lead', role: 'lead', career: emptyCareer() } as unknown as Agent);

    // An Intern with no XP, trusted all the way: no rank cap, and no scrapbook milestone.
    const a = demoTrust(sim, 'ana', 'staff');
    expect([a.career.rank, a.career.trust, a.career.milestones]).toEqual(['intern', 'staff', []]);
    expect(agents.get('ana')?.career.trust).toBe('staff');
    demoTrust(sim, 'ana', 'staff'); // the same level: nothing to record
    expect(demoTrust(sim, 'ana', 'intern').career.trust).toBe('intern');
    expect(decisions).toEqual([
      ['trust', 'human', 'you trust Ana to work inside its own worktree without asking, and to merge without the cross-vendor review', { agent: 'ana', name: 'Ana', from: 'intern', to: 'staff' }, { agent: 'ana' }],
      ['trust', 'human', 'you took back your trust in Ana: it asks like every other agent', { agent: 'ana', name: 'Ana', from: 'staff', to: 'intern' }, { agent: 'ana' }],
    ]);

    expect(() => demoTrust(sim, 'lead', 'junior')).toThrow('only a worker or a 1:1 agent can be trusted');
    expect(demoTrust(sim, 'lead', 'intern').career.trust).toBe('intern');
    expect(decisions).toHaveLength(2);
  });
});
