// Display sanitization for untrusted text in the office overlay (file names, commands, model
// output, note text) — design-world §8.6. Labels are written with `textContent` only; this
// strips what textContent doesn't: ANSI escapes, C0/C1 controls, bidi overrides and
// zero-width characters; whitespace is collapsed and text is truncated by grapheme.

const ANSI = new RegExp('\\u001B(?:\\[[0-?]*[ -/]*[@-~]|\\][^\\u0007\\u001B]*(?:\\u0007|\\u001B\\\\)|[@-Z\\\\-_])', 'g');
const CONTROL = new RegExp('[\\u0000-\\u001F\\u007F-\\u009F]', 'g');
const BIDI = new RegExp('[\\u061C\\u200E\\u200F\\u202A-\\u202E\\u2066-\\u2069]', 'g');
const ZERO_WIDTH = new RegExp('[\\u200B-\\u200D\\u2060\\uFEFF]', 'g');
/** Line/paragraph separators (Zl/Zp) and lone surrogates (Cs). */
const SEPARATORS = new RegExp('[\\u2028\\u2029]|[\\uD800-\\uDBFF](?![\\uDC00-\\uDFFF])|(?<![\\uD800-\\uDBFF])[\\uDC00-\\uDFFF]', 'g');

let segmenter: Intl.Segmenter | null | undefined;
function graphemes(s: string): string[] {
  if (segmenter === undefined) segmenter = typeof Intl !== 'undefined' && 'Segmenter' in Intl ? new Intl.Segmenter(undefined, { granularity: 'grapheme' }) : null;
  return segmenter ? Array.from(segmenter.segment(s), (g) => g.segment) : Array.from(s);
}

/** One clean line: escapes/controls/bidi/zero-width stripped, whitespace collapsed, truncated. */
export function clean(s: string | null | undefined, max = 120): string {
  if (!s) return '';
  const t = String(s).replace(ANSI, '').replace(CONTROL, ' ').replace(SEPARATORS, ' ').replace(BIDI, '').replace(ZERO_WIDTH, '').replace(/\s+/g, ' ').trim();
  return truncate(t, max);
}

/** Truncate by grapheme cluster with an ellipsis (never splits an emoji or accent). */
export function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  const g = graphemes(s);
  if (g.length <= max) return s;
  return g.slice(0, Math.max(1, max - 1)).join('').trimEnd() + '…';
}

export function basename(path: string): string {
  const p = path.replace(/[\\/]+$/, '');
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
  return i >= 0 ? p.slice(i + 1) : p;
}

/** First `n` tokens of a shell command ("pnpm test -- --run" → "pnpm test …"). */
export function commandHead(cmd: string, n = 2): string {
  const c = clean(cmd, 200);
  // Leading env assignments and `cd x &&` prefixes say nothing about the intent.
  const stripped = c.replace(/^(?:cd\s+\S+\s*&&\s*)+/, '').replace(/^(?:[A-Z_][A-Z0-9_]*=\S*\s+)+/, '');
  const parts = stripped.split(' ').filter(Boolean);
  const head = parts.slice(0, n).join(' ');
  return truncate(head, 32) + (parts.length > n ? ' …' : '');
}

/** Host of a URL, for "Reading docs.rs". */
export function host(url: string): string {
  try {
    return clean(new URL(url).host, 40) || clean(url, 40);
  } catch {
    return clean(url, 40);
  }
}

/** "0:41", "12:05", "1:02:03" */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const mm = h ? String(m).padStart(2, '0') : String(m);
  return (h ? `${h}:` : '') + `${mm}:${String(sec).padStart(2, '0')}`;
}

/** "14:02" (local time). */
export function hhmm(ts: number): string {
  const d = new Date(ts);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/** FNV-1a 32-bit, for deterministic per-id variation. */
export function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Deterministic PRNG (mulberry32). */
export function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
