// Number, time and money formatting. Everything that updates live is rendered with tabular
// numerals by the caller (`.num`).

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' });
const timeSecFmt = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit', second: '2-digit' });
const dateFmt = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });
const fullFmt = new Intl.DateTimeFormat(undefined, {
  month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit', second: '2-digit',
});

export const clock = (ts: number) => timeFmt.format(ts);
export const clockSec = (ts: number) => timeSecFmt.format(ts);
export const fullDate = (ts: number) => fullFmt.format(ts);

/** "just now", "12 s ago", "4 min ago", "2 h ago", "Oct 24". */
export function ago(ts: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 5) return 'just now';
  if (s < 60) return `${s} s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} h ago`;
  return dateFmt.format(ts);
}

/** Compact duration: "0.4 s", "41 s", "4m 12s", "1h 03m". */
export function dur(ms: number | null | undefined): string {
  if (ms == null || !isFinite(ms)) return '–';
  if (ms < 1000) return `${Math.max(0, ms / 1000).toFixed(1)} s`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${String(m % 60).padStart(2, '0')}m`;
}

/** Elapsed timer "0:41", "4:12", "1:03:09". */
export function timer(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

/** Token counts: 940, 12.4k, 38k, 1.2M. */
export function tokens(n: number | null | undefined): string {
  if (n == null) return '–';
  if (n < 1000) return String(n);
  if (n < 10_000) return (n / 1000).toFixed(1).replace(/\.0$/, '') + 'k';
  if (n < 1_000_000) return Math.round(n / 1000) + 'k';
  return (n / 1_000_000).toFixed(1).replace(/\.0$/, '') + 'M';
}

/** $0.0037, $0.42, $12.40 */
export function usd(n: number | null | undefined): string {
  if (n == null) return '–';
  if (n === 0) return '$0';
  if (n < 0.01) return '$' + n.toFixed(4);
  if (n < 100) return '$' + n.toFixed(2);
  return '$' + Math.round(n).toLocaleString();
}

export const pct = (f: number | null | undefined) => (f == null ? '–' : `${Math.round(f * 100)}%`);

export const plural = (n: number, one: string, many = one + 's') => `${n} ${n === 1 ? one : many}`;

export const shortSha = (sha: string | null | undefined) => (sha ? sha.slice(0, 7) : '');

/** "in 2 h 14 min" / "at 14:00" for rate-limit resets. */
export function resetsIn(ts: number | null | undefined, now = Date.now()): string {
  if (!ts) return '';
  const ms = ts - now;
  if (ms <= 0) return 'now';
  const m = Math.round(ms / 60000);
  if (m < 60) return `in ${m} min`;
  const h = Math.floor(m / 60);
  return `in ${h} h ${m % 60} min`;
}
