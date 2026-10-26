// Small fuzzy matcher for the command palette and mention menus.
// ponytail: a subsequence scorer with word-start/consecutive bonuses — enough for a few hundred
// items. Swap for @leeoniya/ufuzzy if the palette ever searches thousands of rows.

export type Match = { score: number; hits: number[] };

const isBoundary = (prev: string, ch: string) =>
  prev === '' || /[\s/_\-.:#@]/.test(prev) || (prev === prev.toLowerCase() && ch !== ch.toLowerCase());

export function fuzzy(query: string, text: string): Match | null {
  const q = query.trim().toLowerCase();
  if (!q) return { score: 0, hits: [] };
  const t = text.toLowerCase();
  // Fast path: substring.
  const at = t.indexOf(q);
  if (at >= 0) {
    const hits = Array.from({ length: q.length }, (_, i) => at + i);
    const boundary = isBoundary(at > 0 ? text[at - 1] : '', text[at]);
    return { score: 100 + (at === 0 ? 40 : 0) + (boundary ? 20 : 0) - at * 0.5 - (t.length - q.length) * 0.05, hits };
  }
  let qi = 0;
  let score = 0;
  let streak = 0;
  const hits: number[] = [];
  for (let i = 0; i < t.length && qi < q.length; i++) {
    if (t[i] === q[qi]) {
      hits.push(i);
      streak++;
      score += 1 + streak * 2 + (isBoundary(i > 0 ? text[i - 1] : '', text[i]) ? 6 : 0);
      qi++;
    } else {
      streak = 0;
      score -= 0.15;
    }
  }
  if (qi < q.length) return null;
  return { score, hits };
}

/** Split text into highlighted / plain runs for rendering match hits. */
export function highlight(text: string, hits: number[]): { s: string; hit: boolean }[] {
  if (!hits.length) return [{ s: text, hit: false }];
  const set = new Set(hits);
  const out: { s: string; hit: boolean }[] = [];
  for (let i = 0; i < text.length; i++) {
    const hit = set.has(i);
    const last = out[out.length - 1];
    if (last && last.hit === hit) last.s += text[i];
    else out.push({ s: text[i], hit });
  }
  return out;
}
