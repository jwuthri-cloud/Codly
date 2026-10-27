import type { Memory } from '../gen/Memory';

/** What a memory card shows: headline, body, and the task key when the text carries one. */
export interface MemoryView {
  title: string;
  body: string;
  key: string | null;
}

const KEY = '([A-Za-z]{1,4}-?\\d+)';
const SUMMARY = new RegExp(`^${KEY}\\s+(.{3,120}?):\\s+([\\s\\S]+)$`);
const LESSON = new RegExp(`^${KEY}:\\s+([\\s\\S]+)$`);
const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1).trimEnd()}…` : s);

/**
 * Structured fields win; older memories (free text written before `title`/`tags`) are
 * split on the shapes Codly itself writes ("t1 Title: summary", "t1: `cmd` failed…") and
 * otherwise on their first sentence.
 */
export function memoryView(m: Memory): MemoryView {
  const text = m.text.trim();
  const key = m.tags?.[0] ?? null;
  if (m.title) {
    const prefix = [key && `${key} ${m.title}:`, `${m.title}:`].find((p) => p && text.startsWith(p));
    return { title: m.title, body: prefix ? text.slice(prefix.length).trim() : text, key };
  }
  const s = SUMMARY.exec(text);
  if (s) return { title: s[2], body: s[3].trim(), key: s[1] };
  const l = LESSON.exec(text);
  if (l) return { title: clip(l[2].split(/(?<=[.;!?])\s|\n/)[0], 80), body: l[2].trim(), key: l[1] };
  const first = text.split(/(?<=[.!?])\s|\n/)[0];
  return { title: clip(first, 80), body: text.length > first.length ? text.slice(first.length).trim() : '', key: null };
}
