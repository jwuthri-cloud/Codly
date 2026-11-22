// The command palette's guard for rows that cannot be taken back (merge, cancel a running plan,
// answer an approval): the first Enter or click only arms the row and shows what it will do, the
// second one runs it. Any other key, or a different row, disarms. A held ↵ and a double-click
// are one decision, not two: their repeats do nothing on such a row.
import type { ApprovalSubject } from '../lib/gen/ApprovalSubject';
import { clean } from '../lib/util/sanitize';

/**
 * `armed`: the id of the row waiting for its second Enter (null: none). `again`: this press is
 * the auto-repeat of a held key, or a later click of a double-click.
 */
export function press(row: { id: string; confirm?: string }, armed: string | null, again: boolean): { run: boolean; armed: string | null } {
  if (!row.confirm) return { run: true, armed: null };
  if (again) return { run: false, armed };
  if (armed === row.id) return { run: true, armed: null };
  return { run: false, armed: row.id };
}

/** What an approval asks for, in full, after "<agent> wants to" (a row's title shows 50 characters of it). */
export function wants(s: ApprovalSubject): string {
  switch (s.kind) {
    case 'command':
      return `run ${clean(s.command)}${s.cwd ? `\nin ${clean(s.cwd)}` : ''}`;
    case 'file_change':
      return `change ${s.paths.map(clean).join(', ')}`;
    case 'tool':
      return `use the tool ${clean(s.name)}`;
    case 'question':
      return 'ask you a question';
    case 'permissions':
      return 'get more permissions';
  }
}
