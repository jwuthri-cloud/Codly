import { describe, expect, it } from 'vitest';
import { clashes, emptyCareer, makeApplicants, rankFor, rankProgress } from '../util/career';

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
