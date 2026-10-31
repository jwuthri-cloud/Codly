import { describe, expect, it } from 'vitest';
import { clashes, contextLabel, emptyCareer, makeApplicants, rankFor, rankProgress } from '../util/career';
import { demoBiggerModel } from '../demo/office';
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
