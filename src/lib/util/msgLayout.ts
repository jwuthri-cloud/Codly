// Geometry for the message graph. No physics: nodes sit on an ellipse in the order given
// (so the picture only changes when the team does), and edges are quadratic curves bent by a
// per-edge offset so A→B and B→A, or two acts between the same pair, never overlap.
// ponytail: a circle reads fine up to ~12 agents; past that, group by request thread.

export type Pt = { x: number; y: number };

/** `ids` evenly on an ellipse inside w×h (pad from the edges), first at the top, clockwise. */
export function circleLayout(ids: string[], w: number, h: number, pad = 56): Map<string, Pt> {
  const out = new Map<string, Pt>();
  const n = ids.length;
  const cx = w / 2;
  const cy = h / 2;
  if (n === 1) out.set(ids[0], { x: cx, y: cy });
  else {
    const rx = Math.max(10, w / 2 - pad);
    const ry = Math.max(10, h / 2 - pad);
    ids.forEach((id, i) => {
      const a = -Math.PI / 2 + (2 * Math.PI * i) / n;
      out.set(id, { x: cx + rx * Math.cos(a), y: cy + ry * Math.sin(a) });
    });
  }
  return out;
}

/**
 * Offset (px) of each edge from the straight line between its two nodes. Edges of the same
 * unordered pair are spread symmetrically around it, in a stable order; the offset is signed
 * against the pair's canonical direction (smaller id → larger id), so both directions bend
 * apart. Returns one offset per edge, in the order given.
 */
export function edgeOffsets(edges: { from: string; to: string; act: string }[], gap = 20): number[] {
  const groups = new Map<string, number[]>();
  edges.forEach((e, i) => {
    const key = e.from < e.to ? `${e.from}\u0000${e.to}` : `${e.to}\u0000${e.from}`;
    (groups.get(key) ?? groups.set(key, []).get(key)!).push(i);
  });
  const out = new Array<number>(edges.length).fill(0);
  for (const idx of groups.values()) {
    idx.sort((a, b) => (edges[a].from + edges[a].act < edges[b].from + edges[b].act ? -1 : 1));
    idx.forEach((i, k) => {
      const o = (k - (idx.length - 1) / 2) * gap;
      // A reverse edge is drawn in its own direction, whose "left" is the canonical "right".
      out[i] = edges[i].from < edges[i].to ? o : -o;
    });
  }
  return out;
}

/** Path of one edge from node `a` to node `b` (radius r), bent by `offset`, plus its label point. */
export function edgePath(a: Pt, b: Pt, offset: number, r: number): { d: string; mid: Pt } {
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2;
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  // Control point: the midpoint pushed along the unit normal (offset 0 = a straight edge).
  const c: Pt = { x: mx + (-dy / len) * offset, y: my + (dx / len) * offset };
  const toward = (from: Pt, to: Pt, d: number): Pt => {
    const l = Math.hypot(to.x - from.x, to.y - from.y) || 1;
    return { x: from.x + ((to.x - from.x) / l) * d, y: from.y + ((to.y - from.y) / l) * d };
  };
  const s = toward(a, c, r + 1);
  // Stop short of the node so the arrowhead is visible.
  const e = toward(b, c, r + 5);
  const f = (n: number) => n.toFixed(1);
  return { d: `M${f(s.x)},${f(s.y)} Q${f(c.x)},${f(c.y)} ${f(e.x)},${f(e.y)}`, mid: { x: (s.x + 2 * c.x + e.x) / 4, y: (s.y + 2 * c.y + e.y) / 4 } };
}
