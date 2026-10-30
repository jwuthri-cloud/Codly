// What a piece of furniture is for, Sims-style: sit on a sofa, nap on it, read at a bookcase,
// get a coffee, watch TV, dance to the radio. Each use has spots around the piece (seats on it,
// or a place to stand in front). Pure: no three.js. Cosmetic: only idle agents use furniture.

import type { Pose, Prop } from './behavior';
import type { Piece } from './buy';
import type { Slot } from './layout';

export type Use = 'sit' | 'nap' | 'read' | 'coffee' | 'watch' | 'dance';

export const USE_LABEL: Record<Use, string> = { sit: 'Sit', nap: 'Nap', read: 'Read', coffee: 'Get a coffee', watch: 'Watch TV', dance: 'Dance' };

/** What the character does there. */
export const USE_POSE: Record<Use, { pose: Pose; prop: Prop }> = {
  sit: { pose: 'sit', prop: null },
  nap: { pose: 'sleep', prop: null },
  read: { pose: 'browse', prop: 'book' },
  coffee: { pose: 'drink', prop: 'mug' },
  watch: { pose: 'watch', prop: null },
  dance: { pose: 'dance', prop: null },
};

const RULES: [RegExp, Use[]][] = [
  [/sofa|couch|recliner|relax|armchair|loungeChair|loungeDesignChair/i, ['sit', 'nap']],
  [/chair|stool|bench|ottoman/i, ['sit']],
  [/bookcase|shelf|book_set|books/i, ['read']],
  [/coffee|fridge|kitchenSink|microwave|kitchenBar|cooler/i, ['coffee']],
  [/television/i, ['watch']],
  [/radio|speaker/i, ['dance']],
];

/** The uses of a catalog piece (furniture.json id). */
export function usesOf(item: string): Use[] {
  return RULES.find(([re]) => re.test(item))?.[1] ?? [];
}

const SEATED = new Set<Use>(['sit', 'nap']);

/** Where a character goes to use `p` (world floor-plan coordinates), one per person. */
export function spotsOf(p: Piece, use: Use): Slot[] {
  const at = (lx: number, lz: number, face: number, pose: Slot['pose'], seat?: number): Slot => ({
    x: p.x + Math.cos(p.rot) * lx + Math.sin(p.rot) * lz,
    z: p.z - Math.sin(p.rot) * lx + Math.cos(p.rot) * lz,
    face,
    pose,
    seat,
  });
  const { w, d, h } = p.dims;
  if (SEATED.has(use)) {
    // A stool's seat is its top; a chair or sofa seat is a bit under half its height.
    const seat = p.y + (/stool/i.test(p.item) ? h * 0.92 : Math.min(0.42, Math.max(0.28, h * 0.45)));
    const n = use === 'nap' ? 1 : Math.min(3, Math.max(1, Math.round(w / 0.7)));
    return Array.from({ length: n }, (_, i) => at(-w / 2 + ((i + 0.5) * w) / n, d * 0.05, p.rot, 'sit', seat));
  }
  const gap = use === 'watch' ? 1.2 : use === 'dance' ? 0.8 : 0.45;
  return [at(0, d / 2 + gap, p.rot + Math.PI, 'stand')];
}
