// Layered DAG layout for the task graph (left → right), plus the critical path.
// ponytail: longest-path layering + 4 barycenter sweeps. Plenty for plans of ≤ 50 tasks;
// reach for @dagrejs/dagre only if real plans get big and tangled.

export type DagInput = { id: string; deps: string[]; done: boolean };
export type Placed = { id: string; layer: number; order: number; x: number; y: number };
export type Edge = { from: string; to: string; critical: boolean };

export type DagLayout = {
  nodes: Map<string, Placed>;
  edges: Edge[];
  critical: Set<string>;
  width: number;
  height: number;
};

export function layoutDag(
  input: DagInput[],
  o: { nodeW: number; nodeH: number; gapX: number; gapY: number },
): DagLayout {
  const byId = new Map(input.map((n) => [n.id, n]));
  const deps = new Map(input.map((n) => [n.id, n.deps.filter((d) => byId.has(d) && d !== n.id)]));
  const children = new Map<string, string[]>(input.map((n) => [n.id, []]));
  for (const [id, ds] of deps) for (const d of ds) children.get(d)!.push(id);

  // Kahn topological order (nodes stuck in a cycle are appended at the end).
  const indeg = new Map(input.map((n) => [n.id, deps.get(n.id)!.length]));
  const queue = input.filter((n) => indeg.get(n.id) === 0).map((n) => n.id);
  const topo: string[] = [];
  while (queue.length) {
    const id = queue.shift()!;
    topo.push(id);
    for (const c of children.get(id)!) {
      indeg.set(c, indeg.get(c)! - 1);
      if (indeg.get(c) === 0) queue.push(c);
    }
  }
  for (const n of input) if (!topo.includes(n.id)) topo.push(n.id);

  // Longest-path layering.
  const layer = new Map<string, number>();
  for (const id of topo) {
    const ds = deps.get(id)!.filter((d) => layer.has(d));
    layer.set(id, ds.length ? Math.max(...ds.map((d) => layer.get(d)!)) + 1 : 0);
  }
  const layers: string[][] = [];
  for (const id of topo) (layers[layer.get(id)!] ??= []).push(id);

  // Barycenter ordering sweeps to reduce crossings.
  const pos = new Map<string, number>();
  const index = () => layers.forEach((l) => l.forEach((id, i) => pos.set(id, i)));
  index();
  const bary = (ids: string[]) => (ids.length ? ids.reduce((a, d) => a + (pos.get(d) ?? 0), 0) / ids.length : -1);
  for (let sweep = 0; sweep < 4; sweep++) {
    const down = sweep % 2 === 0;
    const order = down ? layers.slice(1) : layers.slice(0, -1).reverse();
    for (const l of order) {
      const key = new Map(l.map((id) => [id, bary(down ? deps.get(id)! : children.get(id)!)]));
      l.sort((a, b) => {
        const ka = key.get(a)!;
        const kb = key.get(b)!;
        if (ka < 0 || kb < 0) return (pos.get(a) ?? 0) - (pos.get(b) ?? 0);
        return ka - kb;
      });
      l.forEach((id, i) => pos.set(id, i));
    }
  }

  // Coordinates: each layer centered on the tallest one.
  const maxRows = Math.max(1, ...layers.map((l) => l.length));
  const height = maxRows * o.nodeH + (maxRows - 1) * o.gapY;
  const width = layers.length * o.nodeW + Math.max(0, layers.length - 1) * o.gapX;
  const nodes = new Map<string, Placed>();
  layers.forEach((l, li) => {
    const h = l.length * o.nodeH + (l.length - 1) * o.gapY;
    const y0 = (height - h) / 2;
    l.forEach((id, i) =>
      nodes.set(id, { id, layer: li, order: i, x: li * (o.nodeW + o.gapX), y: y0 + i * (o.nodeH + o.gapY) }),
    );
  });

  // Critical path: longest chain weighted by unfinished work (all-done → longest chain overall).
  const allDone = input.every((n) => n.done);
  const w = (id: string) => (allDone || !byId.get(id)!.done ? 1 : 0.001);
  const dist = new Map<string, number>();
  const prev = new Map<string, string | null>();
  for (const id of topo) {
    let best = 0;
    let from: string | null = null;
    for (const d of deps.get(id)!) {
      const v = dist.get(d) ?? 0;
      if (v > best) { best = v; from = d; }
    }
    dist.set(id, best + w(id));
    prev.set(id, from);
  }
  const critical = new Set<string>();
  let end: string | null = null;
  let max = -1;
  for (const [id, v] of dist) if (v > max) { max = v; end = id; }
  if (input.length > 1) for (let c = end; c; c = prev.get(c) ?? null) critical.add(c);

  const edges: Edge[] = [];
  for (const [id, ds] of deps)
    for (const d of ds) edges.push({ from: d, to: id, critical: critical.has(d) && critical.has(id) && prev.get(id) === d });

  return { nodes, edges, critical, width, height };
}
