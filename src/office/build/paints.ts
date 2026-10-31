// Build mode's swatches: the floors (the materials textures.ts paints, as the core allows them),
// wall colours, and a few wallpapers. A wall side's paint is a colour ("#rrggbb") or a wallpaper
// id ("paper:…"), drawn by gfx/building.ts.

import type { FloorKind } from '../layout';

export const FLOORS: { id: FloorKind; label: string; swatch: string }[] = [
  { id: 'oak', label: 'Oak', swatch: '#c89a64' },
  { id: 'walnut', label: 'Walnut herringbone', swatch: '#7a5236' },
  { id: 'walnut-planks', label: 'Walnut planks', swatch: '#5e3f2a' },
  { id: 'carpet-sage', label: 'Sage carpet', swatch: '#9fb08f' },
  { id: 'carpet-blue', label: 'Blue carpet', swatch: '#6f86a6' },
  { id: 'tile-cool', label: 'Cool tile', swatch: '#c9d0d4' },
  { id: 'tile-warm', label: 'Terracotta tile', swatch: '#cf9a74' },
  { id: 'terrazzo', label: 'Terrazzo', swatch: '#e2dcd2' },
  { id: 'concrete', label: 'Concrete', swatch: '#a9a59e' },
];

export const COLOURS: { id: string; label: string }[] = [
  { id: '#f4efe6', label: 'Chalk' },
  { id: '#e8d6b8', label: 'Sand' },
  { id: '#b9cdb0', label: 'Sage' },
  { id: '#b8cde0', label: 'Sky' },
  { id: '#e8c4bc', label: 'Blush' },
  { id: '#d9a35f', label: 'Mustard' },
  { id: '#c98e6b', label: 'Clay' },
  { id: '#3d4f6e', label: 'Navy' },
  { id: '#4a4a52', label: 'Charcoal' },
];

/** A wallpaper: stripes (two colours, `w` metres each), a two-tone wall with a rail (`low` below
 *  `rail` metres), or checks. */
export type Paper =
  | { id: string; label: string; look: 'stripes'; a: string; b: string; w: number }
  | { id: string; label: string; look: 'dado'; a: string; b: string; rail: number }
  | { id: string; label: string; look: 'checks'; a: string; b: string; w: number };

export const PAPERS: Paper[] = [
  { id: 'paper:stripe-sage', label: 'Sage stripes', look: 'stripes', a: '#c9d8bf', b: '#a9bf9d', w: 0.25 },
  { id: 'paper:stripe-rose', label: 'Rose stripes', look: 'stripes', a: '#f0d6cf', b: '#e2b8ad', w: 0.18 },
  { id: 'paper:dado-navy', label: 'Navy dado', look: 'dado', a: '#ece4d6', b: '#3d4f6e', rail: 0.8 },
  { id: 'paper:dado-green', label: 'Green dado', look: 'dado', a: '#f1ead9', b: '#5f7a5c', rail: 0.8 },
  { id: 'paper:checks-sand', label: 'Sand checks', look: 'checks', a: '#eadcc4', b: '#d8c4a2', w: 0.3 },
];

export const paperOf = (id: string) => PAPERS.find((p) => p.id === id) ?? null;

/** A paint's label and a CSS background for its swatch. */
export function paintLook(id: string): { label: string; css: string } {
  const p = paperOf(id);
  if (p?.look === 'stripes') return { label: p.label, css: `repeating-linear-gradient(90deg, ${p.a} 0 6px, ${p.b} 6px 12px)` };
  if (p?.look === 'dado') return { label: p.label, css: `linear-gradient(${p.a} 0 55%, #f4efe6 55% 62%, ${p.b} 62%)` };
  if (p?.look === 'checks') return { label: p.label, css: `repeating-conic-gradient(${p.a} 0 25%, ${p.b} 0 50%) 0 0 / 12px 12px` };
  return { label: COLOURS.find((c) => c.id === id)?.label ?? id, css: id };
}

export const floorLabel = (m: string) => FLOORS.find((f) => f.id === m)?.label ?? m;
