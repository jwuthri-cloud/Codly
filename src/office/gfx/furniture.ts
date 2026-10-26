// Furniture, drawn in code: every piece is a few boxes, cylinders and blobs with flat colors
// (warm wood, felt, brass, plants). Each function builds into a Builder whose frame is the
// item's placement: origin on the floor at the item's center, front facing +z, width along x.

import { DESK_H } from '../layout';
import { BOOKS, lin, MAT, mix, type RGB } from '../palette';
import { rng } from '../text';
import type { Builder } from './builder';

const C = new Map<string, RGB>();
export const c = (hex: string): RGB => {
  let v = C.get(hex);
  if (!v) {
    v = lin(hex);
    C.set(hex, v);
  }
  return v;
};

// ───────────── desks (instanced prototypes) ─────────────

/** A pod desk: birch top on a slim charcoal frame; the occupant sits at −z. */
export function desk(b: Builder) {
  const top = DESK_H;
  b.box(0, top - 0.02, 0, 1.3, 0.04, 0.66, c(MAT.birch));
  b.box(0, top - 0.045, 0, 1.24, 0.012, 0.6, c('#d9c6a6'));
  for (const sx of [-0.6, 0.6]) {
    b.box(sx, (top - 0.04) / 2, 0, 0.04, top - 0.04, 0.56, c(MAT.charcoal));
    b.box(sx, 0.015, 0, 0.06, 0.03, 0.62, c(MAT.charcoal));
  }
  // Pedestal drawers on the right.
  b.box(0.38, 0.22, 0.02, 0.36, 0.44, 0.52, c('#e9e2d6'));
  b.box(0.38, 0.33, -0.245, 0.3, 0.012, 0.012, c(MAT.metal));
  b.box(0.38, 0.17, -0.245, 0.3, 0.012, 0.012, c(MAT.metal));
  // Keyboard and mouse in front of the monitor; a notepad.
  b.box(-0.04, top + 0.008, -0.1, 0.4, 0.016, 0.13, c('#3a3c43'));
  b.box(-0.04, top + 0.018, -0.1, 0.37, 0.006, 0.105, c('#55585f'));
  b.box(0.25, top + 0.01, -0.1, 0.06, 0.02, 0.09, c('#3a3c43'));
  b.box(-0.47, top + 0.006, -0.12, 0.16, 0.012, 0.2, c(MAT.paper), 0.18);
}

/** The Leads' desk: a warm walnut writing desk. */
export function leadDesk(b: Builder) {
  const top = DESK_H + 0.02;
  b.box(0, top - 0.025, 0, 1.7, 0.05, 0.8, c(MAT.walnut));
  b.box(0, top - 0.06, 0, 1.64, 0.02, 0.74, c(MAT.walnutDark));
  for (const sx of [-0.78, 0.78]) b.box(sx, (top - 0.05) / 2, 0, 0.08, top - 0.05, 0.72, c(MAT.walnutDark));
  b.box(0, 0.3, 0.33, 1.5, 0.42, 0.04, c(MAT.walnutDark));
  b.box(-0.05, top + 0.008, -0.14, 0.42, 0.016, 0.13, c('#3a3c43'));
  b.box(-0.05, top + 0.018, -0.14, 0.39, 0.006, 0.105, c('#55585f'));
  // A leather blotter, a pen pot and a little succulent.
  b.box(0.45, top + 0.004, -0.08, 0.42, 0.008, 0.3, c(MAT.leather));
  b.cyl(0.62, top, 0.22, 0.04, 0.1, c(MAT.brass), 10);
  b.cyl(-0.68, top, 0.22, 0.055, 0.07, c(MAT.potWhite), 10);
  b.blob(-0.68, top + 0.11, 0.22, 0.07, 0.06, 0.07, c(MAT.leafLight), 0);
}

/** Desk chair (0), Lead chair (1), meeting chair (2). The sitter faces +z. */
export function chair(b: Builder, variant: number) {
  if (variant === 2) {
    const wood = c(MAT.oakDark);
    b.box(0, 0.3, 0, 0.42, 0.05, 0.4, c(MAT.fabricOlive));
    for (const [x, z] of [
      [-0.17, -0.16],
      [0.17, -0.16],
      [-0.17, 0.16],
      [0.17, 0.16],
    ])
      b.box(x, 0.14, z, 0.035, 0.28, 0.035, wood);
    b.box(0, 0.52, -0.19, 0.4, 0.2, 0.035, wood);
    b.box(-0.17, 0.42, -0.19, 0.03, 0.22, 0.03, wood);
    b.box(0.17, 0.42, -0.19, 0.03, 0.22, 0.03, wood);
    return;
  }
  const lead = variant === 1;
  const fab = c(lead ? MAT.leather : MAT.fabricTeal);
  b.box(0, 0.3, 0, 0.44, 0.07, 0.42, fab);
  b.box(0, lead ? 0.5 : 0.45, -0.22, lead ? 0.44 : 0.4, lead ? 0.3 : 0.2, 0.06, fab);
  b.box(0, 0.37, -0.22, 0.06, 0.08, 0.04, c(MAT.charcoal));
  b.cyl(0, 0.05, 0, 0.025, 0.22, c(MAT.charcoal), 8);
  b.box(0, 0.035, 0, 0.5, 0.03, 0.05, c(MAT.charcoal));
  b.box(0, 0.035, 0, 0.05, 0.03, 0.5, c(MAT.charcoal));
  if (lead) {
    for (const sx of [-0.24, 0.24]) b.box(sx, 0.42, -0.02, 0.04, 0.04, 0.3, c(MAT.charcoal));
  }
}

/** Monitor (front +z faces the occupant). The screen is a separate dynamic mesh. */
export function monitor(b: Builder, variant: number) {
  const top = DESK_H + (variant === 1 ? 0.02 : 0);
  b.box(0, top + 0.008, -0.02, 0.22, 0.016, 0.14, c(MAT.monitor));
  b.box(0, top + 0.12, -0.04, 0.04, 0.22, 0.03, c(MAT.monitor));
  b.box(0, top + 0.27, 0, 0.58, 0.36, 0.03, c(MAT.monitor));
}

/** Desk lamp body (the shade is instanced separately so it can light up). */
export function lampBody(b: Builder) {
  const top = DESK_H;
  b.cyl(0, top, 0, 0.06, 0.015, c(MAT.charcoal), 12);
  b.cyl(0, top, 0, 0.012, 0.3, c(MAT.brass), 6);
}

export function lampShade(b: Builder) {
  b.cyl(0, 0, 0, 0.085, 0.09, [1, 1, 1], 12, 0.05);
}

/** The pod's felt divider with a wooden cap. */
export function divider(b: Builder) {
  b.box(0, DESK_H + 0.17, 0, 2.62, 0.3, 0.04, c('#8fa48d'));
  b.box(0, DESK_H + 0.33, 0, 2.62, 0.025, 0.06, c(MAT.oakDark));
}

// ───────────── library ─────────────

export function shelf(b: Builder, len: number, seed: number) {
  const r = rng(seed);
  const H = 1.95;
  const Dp = 0.42;
  const wood = c(MAT.shelf);
  const back = mix(wood, [0, 0, 0], 0.25);
  b.box(0, H / 2, -Dp / 2 + 0.02, len, H, 0.03, back);
  for (const sx of [-len / 2 + 0.03, len / 2 - 0.03]) b.box(sx, H / 2, 0, 0.06, H, Dp, wood);
  b.box(0, H - 0.03, 0, len, 0.06, Dp + 0.02, wood);
  b.box(0, 0.06, 0, len, 0.12, Dp, mix(wood, [0, 0, 0], 0.15));
  const levels = [0.12, 0.56, 1.0, 1.44];
  for (const y of levels.slice(1)) b.box(0, y, 0, len - 0.06, 0.035, Dp - 0.02, wood);
  for (const y0 of levels) {
    let x = -len / 2 + 0.08;
    const end = len / 2 - 0.08;
    while (x < end - 0.05) {
      const roll = r();
      if (roll < 0.07 && x < end - 0.3) {
        // A small plant or a box on the shelf.
        if (r() < 0.5) {
          b.cyl(x + 0.08, y0 + 0.02, 0.02, 0.055, 0.09, c(MAT.potTerracotta), 8);
          b.blob(x + 0.08, y0 + 0.16, 0.02, 0.09, 0.08, 0.08, c(MAT.leaf), 0, r() * 3);
        } else b.box(x + 0.1, y0 + 0.09, 0.02, 0.18, 0.14, 0.24, c('#d8c9ad'));
        x += 0.22;
        continue;
      }
      if (roll < 0.14) {
        x += 0.06 + r() * 0.08;
        continue;
      }
      const w = 0.045 + r() * 0.045;
      const h = 0.2 + r() * 0.17;
      const col = c(BOOKS[Math.floor(r() * BOOKS.length)]);
      if (roll > 0.94 && x < end - 0.25) {
        // A leaning book.
        b.box(x + 0.06, y0 + 0.02 + h * 0.42, 0.03, w, h, 0.26 + r() * 0.06, col, 0);
        x += 0.18;
        continue;
      }
      b.box(x + w / 2, y0 + 0.02 + h / 2, 0.02 + r() * 0.02, w, h, 0.26 + r() * 0.06, col);
      b.box(x + w / 2, y0 + 0.02 + h * 0.75, 0.15 + 0.02, w * 0.9, 0.025, 0.01, mix(col, [1, 1, 1], 0.45));
      x += w + 0.006;
    }
  }
}

export function armchair(b: Builder, variant: number) {
  const fab = c(variant === 1 ? MAT.fabricTeal : MAT.fabricRust);
  const dark = mix(fab, [0, 0, 0], 0.2);
  b.box(0, 0.16, 0, 0.8, 0.22, 0.74, dark);
  b.box(0, 0.32, 0.04, 0.58, 0.1, 0.6, fab);
  b.box(0, 0.58, -0.3, 0.78, 0.56, 0.16, fab);
  for (const sx of [-0.35, 0.35]) b.box(sx, 0.38, 0.02, 0.12, 0.26, 0.7, fab);
  for (const [x, z] of [
    [-0.33, -0.3],
    [0.33, -0.3],
    [-0.33, 0.3],
    [0.33, 0.3],
  ])
    b.cyl(x, 0, z, 0.025, 0.06, c(MAT.walnutDark), 6);
}

export function sideTable(b: Builder) {
  b.cyl(0, 0, 0, 0.16, 0.03, c(MAT.walnutDark), 14);
  b.cyl(0, 0.03, 0, 0.03, 0.4, c(MAT.walnutDark), 8);
  b.cyl(0, 0.43, 0, 0.24, 0.035, c(MAT.walnut), 16);
  b.box(-0.05, 0.48, 0.02, 0.2, 0.05, 0.15, c(BOOKS[2]));
  b.cyl(0.1, 0.465, -0.06, 0.04, 0.08, c(MAT.potWhite), 10);
}

/** Floor lamp body; the shade glows (separate emissive mesh, see lampShades). */
export function floorLamp(b: Builder) {
  b.cyl(0, 0, 0, 0.15, 0.03, c(MAT.charcoal), 14);
  b.cyl(0, 0.03, 0, 0.018, 1.32, c(MAT.brass), 6);
}

export function readingTable(b: Builder) {
  const wood = c(MAT.walnut);
  b.box(0, 0.5, 0, 1.2, 0.04, 0.6, wood);
  for (const sx of [-0.55, 0.55]) for (const sz of [-0.25, 0.25]) b.box(sx, 0.24, sz, 0.04, 0.48, 0.04, c(MAT.walnutDark));
  b.box(-0.25, 0.53, 0.05, 0.3, 0.03, 0.22, c(MAT.paper));
  b.box(0.2, 0.55, -0.02, 0.24, 0.06, 0.18, c(BOOKS[0]));
  b.box(0.22, 0.6, -0.01, 0.2, 0.04, 0.16, c(BOOKS[3]));
}

// ───────────── meeting ─────────────

export function meetingTable(b: Builder) {
  const top = 0.52;
  b.box(0, top - 0.025, 0, 3.6, 0.05, 1.3, c(MAT.oak));
  b.box(0, top - 0.06, 0, 3.5, 0.02, 1.2, c(MAT.oakDark));
  for (const sx of [-1.2, 1.2]) {
    b.box(sx, (top - 0.06) / 2, 0, 0.12, top - 0.06, 0.7, c(MAT.charcoal));
    b.box(sx, 0.02, 0, 0.18, 0.04, 0.9, c(MAT.charcoal));
  }
  // Papers, a laptop, a jug and glasses.
  b.box(-0.9, top + 0.005, -0.25, 0.3, 0.01, 0.22, c(MAT.paper), 0.3);
  b.box(0.7, top + 0.005, 0.28, 0.3, 0.01, 0.22, c(MAT.paper), -0.2);
  b.box(-0.1, top + 0.012, 0.3, 0.38, 0.02, 0.26, c('#b9bcc3'));
  b.cyl(0.15, top, -0.15, 0.07, 0.2, c('#cfe6ec'), 12);
  b.cyl(0.38, top, -0.2, 0.035, 0.08, c('#e6f2f4'), 10);
  b.cyl(-0.4, top, 0.05, 0.035, 0.08, c('#e6f2f4'), 10);
}

export function whiteboardFrame(b: Builder) {
  const W = 5.2;
  const y0 = 0.6;
  const y1 = 2.05;
  const fr = c(MAT.frame);
  b.box(0, y0 - 0.02, 0.02, W, 0.04, 0.05, fr);
  b.box(0, y1 + 0.02, 0.02, W, 0.04, 0.05, fr);
  for (const sx of [-W / 2, W / 2]) b.box(sx, (y0 + y1) / 2, 0.02, 0.04, y1 - y0 + 0.08, 0.05, fr);
  b.box(0, y0 - 0.05, 0.07, W * 0.6, 0.025, 0.1, fr);
  for (const [x, col] of [
    [-0.6, '#2f6fd6'],
    [-0.45, '#d64b3b'],
    [-0.3, '#2e9a5a'],
  ] as const)
    b.box(x, y0 - 0.025, 0.08, 0.11, 0.025, 0.025, c(col));
  b.box(0.4, y0 - 0.02, 0.08, 0.14, 0.04, 0.07, c('#d8d4ca'));
}

/** A freestanding kanban board on two easel legs (the cork face is dynamic). */
export function taskBoardFrame(b: Builder) {
  const W = 6.1;
  const y0 = 0.42;
  const y1 = 1.72;
  const fr = c(MAT.walnut);
  b.box(0, (y0 + y1) / 2, -0.03, W + 0.1, y1 - y0 + 0.1, 0.05, mix(fr, [0, 0, 0], 0.2));
  b.box(0, y0 - 0.03, 0.0, W + 0.12, 0.06, 0.08, fr);
  b.box(0, y1 + 0.03, 0.0, W + 0.12, 0.06, 0.08, fr);
  for (const sx of [-W / 2 - 0.03, W / 2 + 0.03]) {
    b.box(sx, (y0 + y1) / 2, 0.0, 0.07, y1 - y0 + 0.12, 0.08, fr);
    b.box(sx, (y0 - 0.06) / 2, -0.02, 0.07, y0 - 0.06, 0.07, c(MAT.walnutDark));
    b.box(sx, 0.02, -0.02, 0.1, 0.04, 0.42, c(MAT.walnutDark));
  }
  // A marker tray and a few spare pins.
  b.box(0, y0 - 0.07, 0.07, 1.6, 0.025, 0.08, fr);
  for (const [x, col] of [
    [-0.3, '#d64b3b'],
    [-0.1, '#2f6fd6'],
    [0.1, '#f2c14e'],
  ] as const)
    b.ball(x, y0 - 0.045, 0.08, 0.025, 0.025, 0.025, c(col), 8, 6);
}

/** A framed poster (our own little abstract prints). */
export function poster(b: Builder, variant: number) {
  const fr = c(MAT.walnutDark);
  const [bg, a, d] = [
    ['#f1e6d2', '#e0a458', '#4f7c8a'],
    ['#2f3b4a', '#e8b65a', '#d97757'],
  ][variant % 2];
  b.box(0, 1.35, 0, 1.1, 0.8, 0.03, fr);
  b.box(0, 1.35, 0.016, 0.98, 0.68, 0.01, c(bg));
  b.ball(-0.15, 1.42, 0.022, 0.18, 0.18, 0.006, c(a), 16, 4);
  b.box(0.18, 1.25, 0.022, 0.36, 0.08, 0.008, c(d));
  b.box(0.18, 1.12, 0.022, 0.26, 0.05, 0.008, c(d));
}

/** A low walnut credenza with books and plants. */
export function credenza(b: Builder) {
  const wood = c(MAT.walnut);
  b.box(0, 0.3, 0, 3.2, 0.56, 0.5, wood);
  b.box(0, 0.6, 0, 3.24, 0.04, 0.52, c(MAT.walnutDark));
  for (let i = 0; i < 4; i++) b.box(-1.2 + i * 0.8, 0.3, 0.255, 0.7, 0.46, 0.012, mix(wood, [1, 1, 1], 0.08));
  for (let i = 0; i < 4; i++) b.box(-1.2 + i * 0.8, 0.42, 0.27, 0.12, 0.02, 0.02, c(MAT.brass));
  b.cyl(-1.1, 0.62, 0, 0.12, 0.2, c(MAT.potWhite), 10, 0.1);
  b.blob(-1.1, 0.95, 0, 0.2, 0.22, 0.2, c(MAT.leafLight), 0);
  for (let i = 0; i < 6; i++) b.box(0.4 + i * 0.07, 0.75, 0, 0.055, 0.26 - (i % 3) * 0.03, 0.2, c(BOOKS[i % BOOKS.length]));
  b.cyl(1.25, 0.62, 0.05, 0.09, 0.14, c(MAT.potTerracotta), 10);
  b.blob(1.25, 0.86, 0.05, 0.14, 0.14, 0.14, c(MAT.leaf), 0);
}

// ───────────── server room ─────────────

export function rack(b: Builder) {
  const W = 1.2;
  const H = 1.72;
  const Dp = 0.75;
  b.box(0, H / 2, 0, W, H, Dp, c(MAT.rack));
  b.box(0, H - 0.02, 0, W + 0.02, 0.04, Dp + 0.02, c('#3a3f4b'));
  // Two cabinet doors with grilles.
  for (const sx of [-W / 4, W / 4]) {
    b.box(sx, H / 2, Dp / 2 + 0.006, W / 2 - 0.04, H - 0.14, 0.012, c(MAT.rackFront));
    for (let k = 0; k < 8; k++) b.box(sx, 0.2 + k * 0.17, Dp / 2 + 0.014, W / 2 - 0.12, 0.012, 0.006, c('#454b58'));
    b.box(sx + (sx < 0 ? 0.24 : -0.24), H / 2, Dp / 2 + 0.02, 0.02, 0.3, 0.02, c(MAT.metal));
  }
  b.box(0, 0.03, 0, W + 0.04, 0.06, Dp + 0.04, c('#33363f'));
}

export function testMonitorBody(b: Builder) {
  const w = 2.3;
  const h = 1.2;
  const yc = 1.45;
  b.box(0, yc, 0, w, h, 0.07, c('#1f2128'));
  for (const sx of [-0.7, 0.7]) {
    b.box(sx, (yc - h / 2) / 2, -0.02, 0.06, yc - h / 2, 0.06, c(MAT.charcoal));
    b.box(sx, 0.015, 0.0, 0.1, 0.03, 0.45, c(MAT.charcoal));
  }
}

// ───────────── mail room ─────────────

export function pigeonholes(b: Builder, cols: number, rows: number) {
  const W = 3.0;
  const H = 1.05;
  const Dp = 0.4;
  const base = 0.25;
  const wood = c(MAT.door);
  b.box(0, base / 2, 0, W, base, Dp, mix(wood, [0, 0, 0], 0.15));
  b.box(0, base + H / 2, -Dp / 2 + 0.015, W, H, 0.03, mix(wood, [0, 0, 0], 0.3));
  for (let i = 0; i <= cols; i++) b.box(-W / 2 + (i * W) / cols, base + H / 2, 0, 0.03, H, Dp, wood);
  for (let j = 0; j <= rows; j++) b.box(0, base + (j * H) / rows, 0, W, 0.03, Dp, wood);
  // Brass name plates under each cubby.
  for (let i = 0; i < cols; i++) for (let j = 0; j < rows; j++) b.box(-W / 2 + ((i + 0.5) * W) / cols, base + (j * H) / rows + 0.04, Dp / 2 + 0.005, 0.14, 0.03, 0.008, c(MAT.brass));
}

export function mailbox(b: Builder) {
  const red = c(MAT.mailRed);
  b.box(0, 0.04, 0, 0.5, 0.08, 0.5, c(MAT.charcoal));
  b.cyl(0, 0.08, 0, 0.22, 0.68, red, 16);
  b.ball(0, 0.76, 0, 0.22, 0.12, 0.22, red, 16, 8);
  b.box(0, 0.62, 0.215, 0.22, 0.03, 0.02, c('#2a1a14'));
  b.box(0, 0.4, 0.215, 0.16, 0.12, 0.012, c(MAT.paper));
}

export function sortingTable(b: Builder) {
  b.box(0, 0.48, 0, 1.6, 0.04, 0.8, c(MAT.oak));
  for (const sx of [-0.74, 0.74]) for (const sz of [-0.34, 0.34]) b.box(sx, 0.23, sz, 0.05, 0.46, 0.05, c(MAT.oakDark));
  b.box(0, 0.12, 0, 1.5, 0.02, 0.7, c(MAT.oakDark));
  b.box(-0.45, 0.62, -0.1, 0.36, 0.24, 0.3, c('#c9a77a'));
  b.box(-0.45, 0.745, -0.1, 0.37, 0.012, 0.06, c('#e8d9b8'));
  b.box(0.05, 0.56, 0.1, 0.28, 0.12, 0.22, c('#d4b48a'));
  for (let i = 0; i < 4; i++) b.box(0.5 + (i % 2) * 0.05, 0.505 + i * 0.012, -0.05 + i * 0.03, 0.28, 0.01, 0.18, c(i % 2 ? '#f4efe3' : '#e9dcc4'), i * 0.2);
  b.box(-0.2, 0.2, 0.1, 0.4, 0.14, 0.3, c('#c9a77a'));
}

// ───────────── archive ─────────────

export function cabinet(b: Builder) {
  const W = 0.9;
  const H = 1.15;
  const Dp = 0.6;
  b.box(0, H / 2, 0, W, H, Dp, c(MAT.cabinet));
  b.box(0, H - 0.01, 0, W + 0.02, 0.02, Dp + 0.02, c(MAT.cabinetDark));
  for (let k = 0; k < 3; k++) {
    const y = 0.2 + k * 0.36;
    b.box(0, y, Dp / 2 + 0.008, W - 0.08, 0.32, 0.016, c(MAT.cabinetDark));
    b.box(0, y + 0.02, Dp / 2 + 0.03, 0.22, 0.03, 0.03, c(MAT.brass));
  }
}

export function catalog(b: Builder) {
  const W = 1.4;
  const H = 0.85;
  const Dp = 0.7;
  const wood = c(MAT.walnut);
  b.box(0, H / 2 + 0.08, 0, W, H, Dp, wood);
  for (const sx of [-0.62, 0.62]) for (const sz of [-0.28, 0.28]) b.box(sx, 0.04, sz, 0.06, 0.08, 0.06, c(MAT.walnutDark));
  for (let i = 0; i < 6; i++)
    for (let j = 0; j < 4; j++) {
      const x = -W / 2 + 0.12 + i * 0.235;
      const y = 0.2 + j * 0.2;
      b.box(x, y, Dp / 2 + 0.006, 0.2, 0.16, 0.012, c(MAT.walnutDark));
      b.box(x, y + 0.03, Dp / 2 + 0.015, 0.08, 0.025, 0.006, c(MAT.paper));
      b.ball(x, y - 0.03, Dp / 2 + 0.02, 0.015, 0.015, 0.015, c(MAT.brass), 6, 4);
    }
  b.box(0, H + 0.1, 0, W + 0.04, 0.04, Dp + 0.04, c(MAT.walnutDark));
}

// ───────────── lounge ─────────────

export function counter(b: Builder) {
  const W = 4.4;
  const H = 0.6;
  const Dp = 0.66;
  b.box(0, H / 2, -0.01, W, H - 0.04, Dp - 0.04, c(MAT.counterCab));
  b.box(0, H, 0, W + 0.04, 0.05, Dp, c(MAT.counter));
  for (let i = 0; i < 6; i++) b.box(-W / 2 + 0.37 + i * 0.73, H * 0.55, Dp / 2 - 0.02, 0.02, 0.18, 0.02, c(MAT.brass));
  for (let i = 1; i < 6; i++) b.box(-W / 2 + i * 0.733, H / 2, Dp / 2 - 0.03, 0.01, H - 0.06, 0.01, mix(c(MAT.counterCab), [0, 0, 0], 0.25));
  // Sink, tap, a kettle, a fruit bowl, jars.
  b.box(0.7, H + 0.02, 0.02, 0.5, 0.02, 0.36, c('#c9ced3'));
  b.cyl(0.7, H + 0.02, -0.2, 0.015, 0.18, c(MAT.metal), 6);
  b.box(0.7, H + 0.19, -0.13, 0.02, 0.02, 0.15, c(MAT.metal));
  b.cyl(-0.4, H + 0.025, 0, 0.08, 0.17, c('#e05d44'), 12);
  b.cyl(-1.2, H + 0.025, 0.05, 0.14, 0.06, c('#e9e2d6'), 14, 0.17);
  for (const [x, col] of [
    [-1.25, '#f0b43c'],
    [-1.15, '#d9542f'],
    [-1.22, '#8cbf4a'],
  ] as const)
    b.ball(x, H + 0.11, 0.05 + (x + 1.2) * 0.8, 0.045, 0.045, 0.045, c(col), 8, 6);
  for (let i = 0; i < 3; i++) b.cyl(1.6 + i * 0.16, H + 0.025, -0.18, 0.05, 0.12 + i * 0.03, c(['#d8c9ad', '#c2714f', '#9fb08f'][i]), 10);
}

export function fridge(b: Builder) {
  b.box(0, 0.78, 0, 0.78, 1.56, 0.72, c(MAT.fridge));
  b.box(0, 1.08, 0.362, 0.74, 0.01, 0.01, c('#c9c5bd'));
  b.box(0.3, 1.3, 0.38, 0.03, 0.3, 0.03, c(MAT.metal));
  b.box(0.3, 0.75, 0.38, 0.03, 0.3, 0.03, c(MAT.metal));
  b.box(-0.15, 1.35, 0.365, 0.12, 0.09, 0.006, c('#f2c14e'), 0.1);
  b.box(0.05, 1.38, 0.365, 0.1, 0.1, 0.006, c('#7fb46a'), -0.15);
}

export function coffee(b: Builder) {
  const y = 0.62;
  b.box(0, y + 0.17, 0, 0.3, 0.34, 0.3, c('#2f3136'));
  b.box(0, y + 0.36, 0, 0.32, 0.04, 0.32, c('#45484f'));
  b.box(0, y + 0.18, 0.13, 0.1, 0.05, 0.08, c(MAT.metal));
  b.cyl(0, y, 0.13, 0.04, 0.07, c(MAT.potWhite), 10);
  b.cyl(0.25, y, 0.05, 0.04, 0.08, c('#d97757'), 10);
  b.cyl(0.25, y, -0.07, 0.04, 0.08, c('#4b8bf5'), 10);
}

export function roundTable(b: Builder) {
  b.cyl(0, 0, 0, 0.26, 0.03, c(MAT.charcoal), 16);
  b.cyl(0, 0.03, 0, 0.04, 0.45, c(MAT.charcoal), 8);
  b.cyl(0, 0.48, 0, 0.55, 0.04, c(MAT.oak), 20);
  b.cyl(0.12, 0.52, -0.05, 0.06, 0.16, c('#cfe6ec'), 10);
  b.blob(0.12, 0.74, -0.05, 0.11, 0.09, 0.11, c('#f0b43c'), 0);
  b.cyl(-0.2, 0.52, 0.15, 0.045, 0.09, c(MAT.potWhite), 10);
}

export function stool(b: Builder) {
  b.cyl(0, 0.3, 0, 0.17, 0.05, c(MAT.fabricMustard), 14);
  for (const a of [0.4, 2.5, 4.6]) b.box(Math.cos(a) * 0.11, 0.15, Math.sin(a) * 0.11, 0.03, 0.3, 0.03, c(MAT.oakDark));
}

export function sofa(b: Builder, leather = false) {
  const W = leather ? 2.4 : 2.2;
  const fab = c(leather ? MAT.leather : MAT.fabricMustard);
  const dark = mix(fab, [0, 0, 0], 0.22);
  b.box(0, 0.17, 0, W, 0.24, 0.86, dark);
  for (const sx of [-W / 4 + 0.05, W / 4 - 0.05]) b.box(sx, 0.33, 0.06, W / 2 - 0.2, 0.1, 0.66, fab);
  b.box(0, 0.56, -0.34, W - 0.1, 0.5, 0.2, fab);
  for (const sx of [-W / 2 + 0.08, W / 2 - 0.08]) b.box(sx, 0.38, 0.02, 0.16, 0.3, 0.84, fab);
  b.box(-W / 4, 0.48, -0.17, 0.36, 0.28, 0.12, c(leather ? '#d9c7a7' : '#4f7c8a'), 0.15);
  for (const sx of [-W / 2 + 0.12, W / 2 - 0.12]) for (const sz of [-0.33, 0.33]) b.cyl(sx, 0, sz, 0.03, 0.06, c(MAT.walnutDark), 6);
}

// ───────────── greenery & misc ─────────────

export function plant(b: Builder, variant: number, seed: number) {
  const r = rng(seed);
  const pot = c([MAT.potTerracotta, MAT.potWhite, MAT.potCharcoal][variant % 3]);
  b.cyl(0, 0, 0, 0.17, 0.3, pot, 12, 0.2);
  b.cyl(0, 0.29, 0, 0.19, 0.03, mix(pot, [0, 0, 0], 0.1), 12);
  b.cyl(0, 0.3, 0, 0.17, 0.01, c('#4a3a2c'), 12);
  const leaves = [c(MAT.leaf), c(MAT.leafLight), c(MAT.leafDark)];
  for (let i = 0; i < 6; i++) {
    const a = (i / 6) * Math.PI * 2 + r();
    const d = 0.08 + r() * 0.08;
    b.blob(Math.cos(a) * d, 0.42 + r() * 0.2, Math.sin(a) * d, 0.13 + r() * 0.05, 0.11 + r() * 0.05, 0.13, leaves[i % 3], 0, r() * 6);
  }
}

export function tallPlant(b: Builder, variant: number, seed: number) {
  const r = rng(seed);
  const pot = c([MAT.potTerracotta, MAT.potWhite, MAT.potCharcoal][variant % 3]);
  b.cyl(0, 0, 0, 0.2, 0.42, pot, 12, 0.24);
  b.cyl(0, 0.41, 0, 0.2, 0.01, c('#4a3a2c'), 12);
  b.cyl(0, 0.4, 0, 0.022, 0.9, c('#6b5136'), 6);
  const leaves = [c(MAT.leaf), c(MAT.leafLight), c(MAT.leafDark)];
  for (let i = 0; i < 11; i++) {
    const t = i / 10;
    const a = i * 2.4 + r();
    const d = 0.1 + (1 - Math.abs(t - 0.45)) * 0.14;
    b.blob(Math.cos(a) * d, 0.75 + t * 0.85, Math.sin(a) * d, 0.17, 0.1, 0.12, leaves[i % 3], 0, a);
  }
}

export function coatRack(b: Builder) {
  b.cyl(0, 0, 0, 0.18, 0.03, c(MAT.walnutDark), 12);
  b.cyl(0, 0.03, 0, 0.025, 1.55, c(MAT.walnutDark), 6);
  b.ball(0, 1.6, 0, 0.04, 0.04, 0.04, c(MAT.brass), 8, 6);
  b.blob(0.1, 1.15, 0.05, 0.14, 0.38, 0.12, c('#6b7f9c'), 0, 0.3);
  b.ball(-0.08, 1.48, -0.04, 0.12, 0.06, 0.12, c('#c4543c'), 12, 6);
  b.box(0.05, 0.55, 0.14, 0.16, 0.18, 0.06, c('#b98a5e'));
}

export function bench(b: Builder) {
  const W = 1.7;
  for (let i = 0; i < 3; i++) b.box(0, 0.36, -0.15 + i * 0.15, W, 0.035, 0.12, c(MAT.oak));
  for (const sx of [-W / 2 + 0.1, W / 2 - 0.1]) b.box(sx, 0.17, 0, 0.06, 0.34, 0.4, c(MAT.charcoal));
}

export function cooler(b: Builder) {
  b.box(0, 0.42, 0, 0.36, 0.84, 0.36, c('#e9e5de'));
  b.box(0, 0.62, 0.185, 0.12, 0.06, 0.02, c(MAT.metal));
  b.cyl(0, 0.84, 0, 0.15, 0.36, c('#9fd0e6'), 14);
  b.ball(0, 1.2, 0, 0.15, 0.06, 0.15, c('#9fd0e6'), 14, 6);
  b.cyl(0.1, 0.84, -0.12, 0.035, 0.08, c(MAT.potWhite), 8);
}

export function printer(b: Builder) {
  b.box(0, 0.25, 0, 0.8, 0.5, 0.6, c(MAT.oakDark));
  b.box(0, 0.64, 0, 0.6, 0.28, 0.48, c('#d9d6cf'));
  b.box(0, 0.79, 0.04, 0.5, 0.02, 0.32, c('#bdb9b0'));
  b.box(0, 0.6, 0.25, 0.4, 0.015, 0.12, c(MAT.paper));
  b.box(0.2, 0.75, -0.18, 0.1, 0.02, 0.06, c('#5ee08f'));
}

export function doorFrame(b: Builder) {
  const fr = c(MAT.doorFrame);
  for (const sx of [-0.82, 0.82]) b.box(sx, 0.19, 0, 0.1, 0.38, 0.22, fr);
  b.box(0, 0.006, 0, 1.6, 0.012, 0.3, c('#bfb3a2'));
}

export function doormat(b: Builder) {
  b.box(0, 0.008, 0, 1.4, 0.016, 0.8, c('#8a6b4a'));
  b.box(0, 0.017, 0, 1.2, 0.004, 0.6, c('#a3825c'));
}
