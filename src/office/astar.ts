// A* on the office's walkable grid (design-world §9.7): 8-connected without corner cutting,
// octile heuristic, a small turn penalty for straighter, more human paths, then string-pulling
// (line-of-sight smoothing) so characters walk in straight lines between turns instead of
// zig-zagging along cells.

export type Cell = [number, number];

const DIRS: readonly (readonly [number, number, number])[] = [
  [1, 0, 1],
  [-1, 0, 1],
  [0, 1, 1],
  [0, -1, 1],
  [1, 1, Math.SQRT2],
  [1, -1, Math.SQRT2],
  [-1, 1, Math.SQRT2],
  [-1, -1, Math.SQRT2],
];
const TURN = 0.12;

/** Binary min-heap of node ids keyed by f. */
class MinHeap {
  private ids: number[] = [];
  private keys: number[] = [];
  get size() {
    return this.ids.length;
  }
  push(id: number, key: number) {
    const ids = this.ids;
    const keys = this.keys;
    let i = ids.length;
    ids.push(id);
    keys.push(key);
    while (i > 0) {
      const p = (i - 1) >> 1;
      if (keys[p] <= key) break;
      ids[i] = ids[p];
      keys[i] = keys[p];
      i = p;
    }
    ids[i] = id;
    keys[i] = key;
  }
  pop(): number {
    const ids = this.ids;
    const keys = this.keys;
    const top = ids[0];
    const lastId = ids.pop()!;
    const lastKey = keys.pop()!;
    const n = ids.length;
    if (n) {
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        if (l >= n) break;
        const r = l + 1;
        const c = r < n && keys[r] < keys[l] ? r : l;
        if (keys[c] >= lastKey) break;
        ids[i] = ids[c];
        keys[i] = keys[c];
        i = c;
      }
      ids[i] = lastId;
      keys[i] = lastKey;
    }
    return top;
  }
}

/**
 * Shortest path from (sx, sy) to (gx, gy) on a w×h grid, start excluded, goal included.
 * `passable(x, y)` says whether a cell can be walked. Returns null when unreachable.
 * The start cell is always considered passable (a character may stand inside furniture's
 * margin, e.g. on its chair).
 */
export function aStar(w: number, h: number, sx: number, sy: number, gx: number, gy: number, passable: (x: number, y: number) => boolean): Cell[] | null {
  if (sx === gx && sy === gy) return [];
  if (gx < 0 || gy < 0 || gx >= w || gy >= h) return null;
  const N = w * h;
  const id = (x: number, y: number) => y * w + x;
  const g = new Float32Array(N).fill(Infinity);
  const came = new Int32Array(N).fill(-1);
  const dir = new Int8Array(N).fill(-1);
  const closed = new Uint8Array(N);
  const heap = new MinHeap();
  const hz = (x: number, y: number) => {
    const dx = Math.abs(x - gx);
    const dy = Math.abs(y - gy);
    return (Math.max(dx, dy) + (Math.SQRT2 - 1) * Math.min(dx, dy)) * 1.001;
  };
  const ok = (x: number, y: number) => x >= 0 && y >= 0 && x < w && y < h && ((x === gx && y === gy) || passable(x, y));
  const s = id(sx, sy);
  g[s] = 0;
  heap.push(s, hz(sx, sy));
  while (heap.size) {
    const c = heap.pop();
    if (closed[c]) continue;
    closed[c] = 1;
    const cx = c % w;
    const cy = (c / w) | 0;
    if (cx === gx && cy === gy) {
      const out: Cell[] = [];
      for (let n = c; n !== s; n = came[n]) out.push([n % w, (n / w) | 0]);
      return out.reverse();
    }
    for (let d = 0; d < 8; d++) {
      const [dx, dy, cost] = DIRS[d];
      const nx = cx + dx;
      const ny = cy + dy;
      if (!ok(nx, ny)) continue;
      // No corner cutting: a diagonal needs both orthogonal neighbours free.
      if (dx && dy && (!ok(cx + dx, cy) || !ok(cx, cy + dy))) continue;
      const n = id(nx, ny);
      if (closed[n]) continue;
      const ng = g[c] + cost + (dir[c] !== -1 && dir[c] !== d ? TURN : 0);
      if (ng < g[n]) {
        g[n] = ng;
        came[n] = c;
        dir[n] = d;
        heap.push(n, ng + hz(nx, ny));
      }
    }
  }
  return null;
}

/** Every cell a segment between two cell centers touches is passable (supercover walk). */
export function lineOfSight(x0: number, y0: number, x1: number, y1: number, passable: (x: number, y: number) => boolean): boolean {
  let x = x0;
  let y = y0;
  const dx = Math.abs(x1 - x0);
  const dy = Math.abs(y1 - y0);
  const sx = x1 > x0 ? 1 : -1;
  const sy = y1 > y0 ? 1 : -1;
  let ix = 0;
  let iy = 0;
  while (ix < dx || iy < dy) {
    const a = (0.5 + ix) / Math.max(dx, 1e-9);
    const b = (0.5 + iy) / Math.max(dy, 1e-9);
    if (dx && dy && Math.abs(a - b) < 1e-9) {
      // Exactly through a corner: both neighbours must be free.
      if (!passable(x + sx, y) || !passable(x, y + sy)) return false;
      x += sx;
      y += sy;
      ix++;
      iy++;
    } else if (dy === 0 || (dx && a < b)) {
      x += sx;
      ix++;
    } else {
      y += sy;
      iy++;
    }
    if (!(x === x1 && y === y1) && !passable(x, y)) return false;
  }
  return true;
}

/** Drop intermediate cells that can be skipped in a straight line (start + path → corners). */
export function smooth(start: Cell, path: Cell[], passable: (x: number, y: number) => boolean): Cell[] {
  if (path.length <= 1) return path.slice();
  const pts = [start, ...path];
  const out: Cell[] = [];
  let anchor = 0;
  while (anchor < pts.length - 1) {
    let far = anchor + 1;
    for (let j = pts.length - 1; j > anchor + 1; j--) {
      if (lineOfSight(pts[anchor][0], pts[anchor][1], pts[j][0], pts[j][1], passable)) {
        far = j;
        break;
      }
    }
    out.push(pts[far]);
    anchor = far;
  }
  return out;
}
