// Personality: two traits per agent, fixed by its id (the same agent is always the same
// person). Traits only change how it lives — when it takes a break and what it does, how it
// reacts to real results, its fidgets, how good it is at pool. Never the work: an agent never
// looks busier, lazier or better at its job than its events say.

import { hash } from './text';

export type Trait = 'clumsy' | 'hotheaded' | 'perfectionist' | 'lazy' | 'social' | 'competitive' | 'bookworm' | 'nightowl';

export const TRAITS: Record<Trait, { label: string; about: string }> = {
  clumsy: { label: 'Clumsy', about: 'Startles at failures; not great at pool.' },
  hotheaded: { label: 'Hot-headed', about: 'Thumps the desk when checks fail.' },
  perfectionist: { label: 'Perfectionist', about: 'Facepalms at failures; takes breaks late.' },
  lazy: { label: 'Lazy', about: 'First on the couch; yawns and stretches a lot.' },
  social: { label: 'Social', about: 'Waves at people; loves a two-player game.' },
  competitive: { label: 'Competitive', about: 'Fist pumps on a pass; goes for the tables.' },
  bookworm: { label: 'Bookworm', about: 'Spends breaks reading in the library.' },
  nightowl: { label: 'Night owl', about: 'Livelier late at night, sleepy in the morning.' },
};

const ALL = Object.keys(TRAITS) as Trait[];
/** Pairs that make no sense together. */
const CLASH: [Trait, Trait][] = [
  ['lazy', 'perfectionist'],
  ['lazy', 'competitive'],
  ['bookworm', 'social'],
];

/** Traits picked at hiring (the agent's career), by agent id; they win over the seeded ones. */
const chosen = new Map<string, Trait[]>();

export function setChosenTraits(id: string, traits: string[]) {
  const ts = traits.filter((t): t is Trait => t in TRAITS);
  if (ts.length) chosen.set(id, ts);
  else chosen.delete(id);
}

export function traitsOf(id: string): Trait[] {
  const picked = chosen.get(id);
  if (picked) return picked;
  const h = hash(id + ':traits');
  const a = ALL[h % ALL.length];
  for (let k = 1; k < ALL.length; k++) {
    const b = ALL[(Math.floor(h / ALL.length) + k) % ALL.length];
    if (b !== a && !CLASH.some(([x, y]) => (x === a && y === b) || (x === b && y === a))) return [a, b];
  }
  return [a];
}

/** What it does on a break, by preference (the break room decides what is free). */
export type Activity = 'any' | 'console' | 'table' | 'library' | 'coffee' | 'water' | 'snack' | 'meal' | 'nap';

export function breakPreference(traits: Trait[], needs: { energy: number | null; morale: number; jittery?: boolean; hunger?: number; thirst?: number } | null): Activity {
  // Needs first, like a Sim: exhausted → nap, tired → coffee (not once jittery), down → the couch.
  if (needs?.energy != null && needs.energy < 0.3) return 'nap';
  if (needs?.energy != null && needs.energy < 0.5 && !needs.jittery) return 'coffee';
  // The lower of hunger and thirst first; a lazy one snacks instead of cooking.
  const hunger = needs?.hunger ?? 1;
  const thirst = needs?.thirst ?? 1;
  if (Math.min(hunger, thirst) < 0.4) return thirst <= hunger ? 'water' : traits.includes('lazy') ? 'snack' : 'meal';
  if (needs && needs.morale < 0.4) return 'console';
  if (traits.includes('bookworm')) return 'library';
  if (traits.includes('lazy')) return 'console';
  if (traits.includes('competitive') || traits.includes('social')) return 'table';
  return 'any';
}

/** How long an idle agent waits before taking a break, relative to the default minute. */
export function breakDelay(traits: Trait[], hour = 12): number {
  const base = traits.includes('lazy') ? 0.6 : traits.includes('perfectionist') ? 1.8 : 1;
  // A night owl drags in the morning (breaks sooner) and keeps going late.
  if (traits.includes('nightowl')) return base * (hour < 12 ? 0.7 : hour >= 20 ? 1.4 : 1);
  return base;
}

export type Reaction = 'thump' | 'facepalm' | 'startle' | 'fistpump' | 'wave' | null;

/** Its reaction to a real check result (null: the usual celebrate / slump). */
export function reactionTo(traits: Trait[], ok: boolean): Reaction {
  if (ok) return traits.includes('competitive') ? 'fistpump' : traits.includes('social') ? 'wave' : null;
  return traits.includes('hotheaded') ? 'thump' : traits.includes('perfectionist') ? 'facepalm' : traits.includes('clumsy') ? 'startle' : null;
}

/** Chance a pool shot goes where it was aimed. */
export function poolSkill(traits: Trait[]): number {
  return traits.includes('clumsy') ? 0.35 : traits.includes('competitive') ? 0.7 : 0.55;
}
