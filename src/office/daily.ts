// The office's day: the morning edition of "The Daily Commit" and the standup, both from the
// core's digest of real events since the last edition (nothing is invented: no event, no line).

import type { DayReport } from '../lib/gen/DayReport';
import type { Digest } from '../lib/gen/Digest';

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** What it says at the standup: its real work since the last edition. */
export function standupLine(r: DayReport, period = 'Yesterday'): string {
  const s = r.stats;
  const parts = [
    s.merged && `merged ${s.merged}`,
    s.commits && plural(s.commits, 'commit'),
    s.checks_passed && `${plural(s.checks_passed, 'check')} passed`,
    s.reviews_done && `reviewed ${s.reviews_done}`,
    s.approved && `${plural(s.approved, 'approval')} on my work`,
  ].filter(Boolean);
  return parts.length ? `${period}: ${parts.join(', ')}` : `${period}: nothing landed, ready for today`;
}

export interface Edition {
  headline: string;
  /** Most XP since the last edition. */
  mvp: { id: string; xp: number } | null;
  /** Promotions, mentors and trust first, then the rest. */
  notes: { id: string; text: string }[];
  shipped: string[];
}

export function edition(d: Digest): Edition {
  const merged = d.reports.reduce((n, r) => n + r.stats.merged, 0);
  const headline =
    d.done.length > 0 ? `${plural(d.done.length, 'task')} shipped` : merged > 0 ? `${plural(merged, 'merge')} landed` : d.reports.length ? 'A day of groundwork' : 'A quiet day at the office';
  const top = d.reports[0];
  const big = (t: string) => /^(Promoted|Trusted|Mentored|Took)/.test(t);
  const notes = d.milestones.map(([id, m]) => ({ id, text: m.text })).sort((a, b) => Number(big(b.text)) - Number(big(a.text)));
  return { headline, mvp: top && top.xp > 0 ? { id: top.agent_id, xp: top.xp } : null, notes, shipped: d.done };
}

/** Local day key (YYYY-MM-DD): one edition per day. */
export const dayKey = (t: number) => {
  const d = new Date(t);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
};

/** The edition comes out once a day, after 5 am, covering the time since the last one (at most 3 days). */
export function editionDue(last: number | null, now: number): { since: number } | null {
  if (new Date(now).getHours() < 5) return null;
  if (last != null && dayKey(last) === dayKey(now)) return null;
  return { since: last == null ? now - 86_400_000 : Math.max(last, now - 3 * 86_400_000) };
}
