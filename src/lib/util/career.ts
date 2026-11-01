// Careers (mirror of crates/codly-core/src/career.rs): ranks, traits, applicants. The core
// keeps the real record; this is for labels and for the web demo, which has no core.

import type { Applicant } from '../gen/Applicant';
import type { Career } from '../gen/Career';
import type { Rank } from '../gen/Rank';
import { AVATARS } from './hire';

export const RANKS: { rank: Rank; label: string; xp: number }[] = [
  { rank: 'intern', label: 'Intern', xp: 0 },
  { rank: 'junior', label: 'Junior', xp: 60 },
  { rank: 'senior', label: 'Senior', xp: 250 },
  { rank: 'staff', label: 'Staff', xp: 800 },
];

/** `rule`: how it changes the agent's work (it rides on its prompts); null = personality only. */
export const TRAITS: { id: string; label: string; rule: string | null }[] = [
  { id: 'perfectionist', label: 'Perfectionist', rule: 'Re-reads its whole diff and runs the checks again before saying it is done.' },
  { id: 'bookworm', label: 'Bookworm', rule: 'Reads the code and docs around a change before editing.' },
  { id: 'lazy', label: 'Lazy', rule: 'Makes the smallest change that works; adds nothing that was not asked.' },
  { id: 'social', label: 'Social', rule: 'Asks the Lead or you when something is unclear instead of guessing.' },
  { id: 'clumsy', label: 'Clumsy', rule: null },
  { id: 'hotheaded', label: 'Hot-headed', rule: null },
  { id: 'competitive', label: 'Competitive', rule: null },
  { id: 'nightowl', label: 'Night owl', rule: null },
];
const CLASH: [string, string][] = [
  ['lazy', 'perfectionist'],
  ['lazy', 'competitive'],
  ['bookworm', 'social'],
];

export const emptyCareer = (): Career => ({
  xp: 0,
  rank: 'intern',
  stats: { commits: 0, checks_passed: 0, checks_failed: 0, approved: 0, changes_requested: 0, reviews_done: 0, merged: 0, shifts: 0, sick: 0 },
  skills: {},
  milestones: [],
  traits: [],
  trust: 'intern',
  friends: {},
  mentor: null,
  stamina: 0,
  legend: false,
  want: null,
});

/** How far it is into its want for the day (mirror of `career::want_progress`). */
export function wantProgress(c: Career): { label: string; done: number; goal: number; met: boolean } | null {
  const w = c.want;
  if (!w) return null;
  const stat = { commit: c.stats.commits, check: c.stats.checks_passed, review: c.stats.reviews_done, approved: c.stats.approved, merge: c.stats.merged }[w.kind];
  return { label: w.label, done: Math.min(w.goal, Math.max(0, stat - w.from)), goal: w.goal, met: w.done_at != null };
}

/** Stamina from its record and days on the crew (mirror of `career::stamina`, for the web demo). */
export function staminaOf(c: Career, days: number): number {
  const grow = (x: number, scale: number, cap: number) => cap * (1 - Math.exp(-Math.max(0, x) / scale));
  const s = grow(days, 45, 35) + grow(c.stats.merged, 75, 35) + grow(c.stats.shifts, 40, 30) - 2 * c.stats.sick;
  return Math.min(100, Math.max(0, Math.round(s)));
}

/** A context window, short: 1000000 → "1M", 272000 → "272k". */
export function contextLabel(tokens: number): string {
  if (tokens >= 1_000_000) return `${+(tokens / 1_000_000).toFixed(1)}M`;
  return `${Math.round(tokens / 1000)}k`;
}

/** "Legend" above Staff (`career.legend`), else the rank. */
export const titleOf = (c: Career) => (c.legend ? 'Legend' : rankLabel(c.rank));

/** What trusting a rank gives an agent (mirror of `career::PERKS`); you grant it, a promotion offers it. */
export const PERKS: { rank: Rank; perk: string }[] = [
  { rank: 'junior', perk: 'Edits files inside its own worktree without asking; commands still ask' },
  { rank: 'senior', perk: 'Works inside its own worktree without asking; risky actions still ask' },
  { rank: 'staff', perk: 'Its work merges without the cross-vendor review' },
];
/** Extra XP an Intern or Junior earns from a review by a Senior or Staff (`career::MENTORED_XP`). */
export const MENTORED_XP = 10;
/** XP a fulfilled want pays (mirror of `career::WANT_XP`). */
export const WANT_XP = 25;
const order = (r: Rank) => RANKS.findIndex((x) => x.rank === r);
/** At or above (mirror of `Rank`'s order). */
export const atLeast = (r: Rank, min: Rank) => order(r) >= order(min);

export const rankLabel = (r: Rank) => RANKS.find((x) => x.rank === r)?.label ?? 'Intern';

/** The rank this much XP earns (`career::rank_for`). Ranks never go down: take the max with the old one. */
export const rankFor = (xp: number): Rank => RANKS.findLast((x) => xp >= x.xp)?.rank ?? 'intern';

/** Progress to the next rank (1 at Staff). */
export function rankProgress(c: Career): { next: string | null; progress: number; toGo: number } {
  const i = RANKS.findIndex((x) => x.rank === c.rank);
  const next = RANKS[i + 1];
  if (!next) return { next: null, progress: 1, toGo: 0 };
  const lo = RANKS[i].xp;
  return { next: next.label, progress: Math.min(1, (c.xp - lo) / (next.xp - lo)), toGo: Math.max(0, next.xp - c.xp) };
}

export const clashes = (a: string, b: string) => CLASH.some(([x, y]) => (x === a && y === b) || (x === b && y === a));

const NAMES = ['Ada', 'Bo', 'Cleo', 'Dax', 'Eli', 'Fern', 'Gus', 'Hana', 'Ivo', 'Juno', 'Kai', 'Lark', 'Milo', 'Nia', 'Otto', 'Pia', 'Quin', 'Remy', 'Sol', 'Tess', 'Uma', 'Vik', 'Wren', 'Yara', 'Zeno'];
const PITCHES = [
  'I read the whole codebase before I touch anything.',
  'Ship small, ship often. That’s me.',
  'I’ll find the bug you didn’t know you had.',
  'Give me the gnarly ticket nobody wants.',
  'Tests first, then everything else.',
  'I ask questions early so we don’t redo it later.',
  'Clean diffs, short reviews.',
  'I work best when it’s quiet. And late.',
];

/** Applicants with free names and colors (the web demo; the app asks the core). */
export function makeApplicants(n: number, taken: string[], colors: number[], seed = Date.now()): Applicant[] {
  const names = [...taken];
  const used = [...colors];
  return Array.from({ length: n }, (_, i) => {
    const s = Math.floor(Math.abs(Math.sin(seed + i * 7.31)) * 1e6);
    const name = NAMES.find((x) => !names.some((t) => t.toLowerCase() === x.toLowerCase())) ?? `Agent ${names.length}`;
    names.push(name);
    const color = [0, 1, 2, 3, 4, 5, 6, 7].find((c) => !used.includes(c)) ?? used.length % 8;
    used.push(color);
    const a = TRAITS[s % 8].id;
    const b = TRAITS.map((t) => t.id).find((t, k) => k > (s % 8) && t !== a && !clashes(a, t)) ?? TRAITS.map((t) => t.id).find((t) => t !== a && !clashes(a, t))!;
    return { name, color, avatar: AVATARS[(s >> 3) % 8], traits: [a, b], pitch: PITCHES[(Math.floor(seed / 7) + i * 3) % 8] }; // distinct within an offer
  });
}
