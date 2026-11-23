// The model dialog's list: it opens with every model in view and the current one marked, and
// what you type narrows it (each word in the model's id or name). A name the CLI does not list
// can still be used: it is the last row. With nothing typed, the first row is the CLI's default,
// and a current model the CLI does not list comes next.
import type { ModelOption } from '../gen/ModelOption';
import { optionLabel } from './models';

/** `id`: what is switched to ('' = the CLI's default model). */
export type PickRow = { id: string; label: string; current: boolean; typed: boolean };

export function pickRows(models: ModelOption[], query: string, current: string): PickRow[] {
  const q = query.trim();
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const hit = (m: ModelOption) => words.every((w) => m.id.toLowerCase().includes(w) || m.name.toLowerCase().includes(w));
  const rows: PickRow[] = [];
  if (!q) rows.push({ id: '', label: 'CLI default', current: current === '', typed: false });
  // A model the catalog does not list (a full name, typed earlier) is still the one it runs on.
  if (!q && current && !models.some((m) => m.id === current)) rows.push({ id: current, label: current, current: true, typed: false });
  for (const m of models) if (hit(m)) rows.push({ id: m.id, label: optionLabel(m), current: m.id === current, typed: false });
  if (q && !models.some((m) => m.id === q)) rows.push({ id: q, label: `Use “${q}”`, current: q === current, typed: true });
  return rows;
}

/** The row highlighted when the list changes: the current model's, else the first. */
export const firstRow = (rows: PickRow[], typed: boolean): number => (typed ? 0 : Math.max(0, rows.findIndex((r) => r.current)));
