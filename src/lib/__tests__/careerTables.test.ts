// The career tables are written twice: in the core (career.rs), which decides, and here, for
// labels and the browser demo. A number or a trait changed on one side only would show the wrong
// rank or let the demo pick a pair the core refuses. This reads the core's tables from its source.
import { describe, expect, it } from 'vitest';
import core from '../../../crates/codly-core/src/career.rs?raw';
import { CLASH, TRAITS as OFFICE_TRAITS } from '../../office/traits';
import { RANKS, TRAITS } from '../util/career';

/** The source text of one `const NAME: … = […];` in career.rs. */
function table(name: string): string {
  const start = core.indexOf(`const ${name}:`);
  expect(start, name).toBeGreaterThanOrEqual(0);
  return core.slice(start, core.indexOf('];', start));
}

describe('the career tables match the core', () => {
  it('XP needed for each rank', () => {
    const need = [...table('RANKS').matchAll(/\(Rank::(\w+), (\d+)\)/g)].map((m) => ({ rank: m[1].toLowerCase(), xp: Number(m[2]) }));
    expect(need).toHaveLength(3);
    expect(RANKS.slice(1).map(({ rank, xp }) => ({ rank, xp }))).toEqual(need);
    expect(RANKS[0]).toMatchObject({ rank: 'intern', xp: 0 });
  });

  it('traits, and which of them change the work', () => {
    const traits = [...table('TRAITS').matchAll(/\("(\w+)", "([^"]+)", (Some|None)/g)].map((m) => ({ id: m[1], label: m[2], rule: m[3] === 'Some' }));
    expect(traits).toHaveLength(8);
    expect(TRAITS.map((t) => ({ id: t.id, label: t.label, rule: t.rule !== null }))).toEqual(traits);
    const labels = (list: { id: string; label: string }[]) => Object.fromEntries(list.map((t) => [t.id, t.label]));
    expect(labels(Object.entries(OFFICE_TRAITS).map(([id, t]) => ({ id, label: t.label })))).toEqual(labels(traits));
  });

  it('pairs of traits that do not go together', () => {
    const pairs = [...table('CLASH').matchAll(/\("(\w+)", "(\w+)"\)/g)].map((m) => [m[1], m[2]]);
    expect(pairs).toHaveLength(3);
    expect(CLASH).toEqual(pairs);
  });
});
