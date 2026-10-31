/// <reference types="node" />
// One-off: writes src/office/lots/studio.json from the hand-built studio (layout.ts `build()`),
// so the Studio becomes a lot like any other. Run with MAKE_STUDIO=1 npx vitest run make-studio.
import { writeFileSync } from 'node:fs';
import { it } from 'vitest';
import type { Lot } from '../../lib/gen/Lot';
import type { LotWall } from '../../lib/gen/LotWall';
import type { LotDoor } from '../../lib/gen/LotDoor';
import type { LotWindow } from '../../lib/gen/LotWindow';
import type { Placed } from '../../lib/gen/Placed';
import RAW from '../furniture.json';
import { officeLayout, type Item } from '../layout';

const KIND = { exterior: 'exterior', interior: 'solid', glass: 'glass', low: 'half' } as const;

it.runIf(process.env.MAKE_STUDIO)('writes the studio lot', () => {
  const L = officeLayout();
  const walls: LotWall[] = [];
  const doors: LotDoor[] = [];
  const windows: LotWindow[] = [];
  const all = [...L.walls];
  // The studio opens straight onto the lobby: a wall made of one wide doorway, so the two are
  // two rooms (doors close rooms) and nothing shows.
  all.splice(12, 0, { x0: 13, z0: 16.5, x1: 21.5, z1: 16.5, kind: 'interior', gaps: [[0, 8.5]] });
  all.forEach((w, i) => {
    const id = `w${i + 1}`;
    walls.push({ id, x0: w.x0, z0: w.z0, x1: w.x1, z1: w.z1, kind: KIND[w.kind] });
    for (const [a, b] of w.gaps) {
      const front = w.z0 === 24 && w.z1 === 24;
      doors.push({ id: `d${doors.length + 1}`, wall: id, at: round((a + b) / 2), width: round(b - a), kind: front ? 'single' : 'open' });
    }
    for (const [a, b] of w.windows ?? []) windows.push({ id: `win${windows.length + 1}`, wall: id, at: round((a + b) / 2), width: round(b - a), kind: 'standard' });
  });

  const ids = RAW as { id: string; builtin?: string; w?: number; d?: number; variant?: number }[];
  const idOf = (it: Item) => {
    const same = ids.filter((f) => f.builtin === it.type);
    const hit =
      same.find((f) => f.w === it.w && f.d === it.d && (f.variant ?? 0) === (it.variant ?? 0)) ??
      same.find((f) => f.w === it.w && f.d === it.d && f.variant === undefined) ??
      same.find((f) => (f.variant ?? 0) === (it.variant ?? 0) && it.type !== 'rug' && it.type !== 'bench') ??
      (it.type === 'plant' || it.type === 'shelf' || it.type === 'tallPlant' ? same[0] : undefined);
    if (!hit) throw new Error(`no catalog piece for ${it.type} ${it.w}×${it.d} v${it.variant}`);
    return hit.id;
  };
  const furniture: Placed[] = [];
  const put = (item: string, x: number, z: number, rot: number, y = 0) => furniture.push({ uid: `s${furniture.length + 1}`, item, x: round(x), y, z: round(z), rot });
  for (const it of L.items) {
    // Made by their desk or table (chairs, monitors, lamps) or by the front door.
    if (it.desk != null && it.type !== 'desk' && it.type !== 'leadDesk') continue;
    if (it.type === 'chair' || it.type === 'door') continue;
    put(idOf(it), it.x, it.z, it.rot);
  }
  // Wall lights between the Leads' windows and in the lounge; the clock over the credenza.
  put('office.sconce', 20.0, 0.14, 0, 1.55);
  put('office.sconce', 23.25, 0.14, 0, 1.55);
  put('office.sconce', 0.14, 17.6, Math.PI / 2, 1.55);
  put('office.clock', 0.1, 11.65, Math.PI / 2, 1.55);

  const lot: Lot = {
    v: 1,
    template: 'studio',
    size: { w: 28, d: 24 },
    walls,
    doors,
    windows,
    floors: { tile: 0.5, default: 'oak', rects: L.rooms.map((r) => ({ ...r.rect, m: r.floor })) },
    paint: [],
    labels: L.rooms.map((r) => ({ x: r.label[0], z: r.label[1], name: null })),
    furniture,
    gift: [...walls.map((w) => w.id), ...doors.map((d) => d.id), ...windows.map((w) => w.id), ...furniture.map((p) => p.uid)],
  };
  writeFileSync(new URL('../lots/studio.json', import.meta.url), format(lot));
});

function round(v: number, digits = 3) {
  const k = 10 ** digits;
  return Math.round(v * k) / k;
}

/** One line per wall, door, piece…: diffs stay readable. */
function format(lot: Lot): string {
  const rows = (xs: unknown[]) => (xs.length ? `[\n    ${xs.map((x) => JSON.stringify(x)).join(',\n    ')}\n  ]` : '[]');
  const { floors, ...rest } = lot;
  const parts = Object.entries(rest).map(([k, v]) => `  ${JSON.stringify(k)}: ${Array.isArray(v) && k !== "gift" ? rows(v) : JSON.stringify(v)}`);
  parts.splice(6, 0, `  "floors": {"tile": ${floors.tile}, "default": ${JSON.stringify(floors.default)}, "rects": ${rows(floors.rects).replace(/\n {4}/g, '\n      ').replace(/\n {2}]/, '\n    ]')}}`);
  return `{\n${parts.join(',\n')}\n}\n`;
}
