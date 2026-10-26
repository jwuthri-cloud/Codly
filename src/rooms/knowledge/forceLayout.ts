// Deterministic force-directed layout for the small knowledge graph (≤ ~150 nodes).
// ponytail: O(n²) Fruchterman–Reingold, recomputed on change. Fine below ~200 entities; past that,
// switch to Barnes–Hut or show the list view by default.
// Runs in an abstract unit space (Fruchterman–Reingold with gravity, so disconnected pieces stay
// close), then fits the result into the viewport. Computed once per change — no animation loop.
// Positions are cached in unit space so adding a triple doesn't reshuffle the picture.

export type FNode = { id: string; x: number; y: number };
export type FEdge = { s: string; t: string };

const cache = new Map<string, { x: number; y: number }>();

export function forceLayout(
  ids: string[],
  edges: FEdge[],
  w: number,
  h: number,
  opts: { pad?: number; padBottom?: number; iterations?: number } = {},
): Map<string, FNode> {
  const n = ids.length;
  const out = new Map<string, FNode>();
  if (!n) return out;
  const pad = opts.pad ?? 48;
  const padBottom = opts.padBottom ?? pad;
  const iterations = opts.iterations ?? 300;

  const nbr = new Map<string, string[]>();
  for (const e of edges) {
    if (e.s === e.t) continue;
    (nbr.get(e.s) ?? nbr.set(e.s, []).get(e.s)!).push(e.t);
    (nbr.get(e.t) ?? nbr.set(e.t, []).get(e.t)!).push(e.s);
  }

  // Unit space: k = ideal edge length.
  const k = 1;
  const pos = new Map<string, { x: number; y: number }>();
  ids.forEach((id, i) => {
    let p = cache.get(id);
    if (!p) {
      const near = (nbr.get(id) ?? []).map((m) => cache.get(m)).find(Boolean);
      const a = i * 2.399963; // golden angle: deterministic, well spread
      const r = 0.6 * Math.sqrt(i + 1);
      p = near ? { x: near.x + Math.cos(a) * 0.5, y: near.y + Math.sin(a) * 0.5 } : { x: Math.cos(a) * r, y: Math.sin(a) * r };
    }
    pos.set(id, { ...p });
  });

  let temp = 0.6;
  for (let it = 0; it < iterations; it++) {
    const disp = new Map(ids.map((id) => [id, { x: 0, y: 0 }]));
    for (let i = 0; i < n; i++)
      for (let j = i + 1; j < n; j++) {
        const a = pos.get(ids[i])!;
        const b = pos.get(ids[j])!;
        let dx = a.x - b.x;
        let dy = a.y - b.y;
        let d = Math.hypot(dx, dy);
        if (d < 1e-3) { dx = 0.01 * (i - j); dy = 0.01; d = 0.015; }
        const f = (k * k) / d;
        const da = disp.get(ids[i])!;
        const db = disp.get(ids[j])!;
        da.x += (dx / d) * f; da.y += (dy / d) * f;
        db.x -= (dx / d) * f; db.y -= (dy / d) * f;
      }
    for (const e of edges) {
      const a = pos.get(e.s);
      const b = pos.get(e.t);
      if (!a || !b || a === b) continue;
      const dx = a.x - b.x;
      const dy = a.y - b.y;
      const d = Math.max(1e-3, Math.hypot(dx, dy));
      const f = (d * d) / k;
      const da = disp.get(e.s)!;
      const db = disp.get(e.t)!;
      da.x -= (dx / d) * f; da.y -= (dy / d) * f;
      db.x += (dx / d) * f; db.y += (dy / d) * f;
    }
    for (const id of ids) {
      const p = pos.get(id)!;
      const d = disp.get(id)!;
      // gravity toward the origin keeps components together
      d.x -= p.x * 0.35;
      d.y -= p.y * 0.35;
      const len = Math.max(1e-6, Math.hypot(d.x, d.y));
      const step = Math.min(len, temp);
      p.x += (d.x / len) * step;
      p.y += (d.y / len) * step;
    }
    temp = Math.max(0.01, temp * 0.985);
  }

  for (const id of ids) cache.set(id, { ...pos.get(id)! });

  // Fit into the viewport, preserving aspect ratio.
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const p of pos.values()) {
    minX = Math.min(minX, p.x); maxX = Math.max(maxX, p.x);
    minY = Math.min(minY, p.y); maxY = Math.max(maxY, p.y);
  }
  const bw = Math.max(1e-3, maxX - minX);
  const bh = Math.max(1e-3, maxY - minY);
  const aw = Math.max(40, w - pad * 2);
  const ah = Math.max(40, h - pad - padBottom);
  const scale = Math.min(aw / bw, ah / bh, 150);
  const ox = pad + (aw - bw * scale) / 2;
  const oy = pad + (ah - bh * scale) / 2;
  for (const id of ids) {
    const p = pos.get(id)!;
    out.set(id, { id, x: ox + (p.x - minX) * scale, y: oy + (p.y - minY) * scale });
  }
  return out;
}
