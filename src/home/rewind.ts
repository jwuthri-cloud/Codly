// "Undo a turn": the list over the composer that puts an agent's files back to how they were
// before one of its turns (Esc Esc, `/undo`, the palette), and the "Put this file back" of the
// sheet's Last turn. What the list shows, what its keys do and what it says before a restore
// are here, away from the components, so they can be tested. Files only: the conversation
// inside the CLI stays, and the core tells the agent which files went back.
import type { AgentCheckpoints } from '../lib/gen/AgentCheckpoints';
import type { LastTurn } from '../lib/gen/LastTurn';
import { ago, plural } from '../lib/util/format';
import { noTurn } from './agentTurns';

/** The list shows the newest turns only: older ones are under Checkpoints in the agent's sheet. */
export const REWIND_ROWS = 10;
/** How many file names a row and the confirm line show before "and N more". */
const NAMES = 3;

/** One turn of the list: the files as they were before it. */
export type RewindRow = {
  /** The checkpoint taken before the turn. */
  n: number;
  /** The first words of the message that started the turn. */
  label: string;
  age: string;
  /** How many files a restore would change. 0: nothing to put back, and the row cannot be picked. */
  changed: number;
  /** The first of those files. */
  files: string[];
};

/** The turn checkpoints of the answer, newest first, at most `REWIND_ROWS`. A restore's own point is not a turn. */
export function rewindRows(kept: Pick<AgentCheckpoints, 'checkpoints'>, now: number): RewindRow[] {
  return kept.checkpoints
    .filter((c) => c.kind === 'turn')
    .slice(0, REWIND_ROWS)
    .map((c) => ({ n: c.n, label: c.label, age: ago(c.ts, now), changed: c.changed, files: c.files }));
}

/** Why the list has no turn to offer (the core's reason, or that no turn was kept yet); null: it has. */
export function rewindOff(kept: Pick<AgentCheckpoints, 'off' | 'checkpoints'>, name: string): string | null {
  return noTurn({ off: kept.off, checkpoints: kept.checkpoints.filter((c) => c.kind === 'turn') }, name);
}

/** The first file names, cut: "a.ts, b.ts, c.ts and 2 more". `changed`: how many files there are in all. */
export function namesLine(files: string[], changed: number): string {
  const shown = files.slice(0, NAMES);
  const more = changed - shown.length;
  return more > 0 ? `${shown.join(', ')} and ${more} more` : shown.join(', ');
}

/** What the list asks before it restores: the whole turn (`path` null), or one file of it. */
export function confirmLine(agent: string, row: Pick<RewindRow, 'changed' | 'files'>, path: string | null): string {
  const stays = (what: string) => `The conversation stays: ${agent} is told which ${what} went back.`;
  if (path !== null) return `Put ${path} back to how it was before this turn? ${stays('file')}`;
  const one = row.changed === 1;
  return `Put ${plural(row.changed, 'file')} back to how ${one ? 'it was' : 'they were'} before this turn? ${namesLine(row.files, row.changed)}. ${stays(one ? 'file' : 'files')}`;
}

/** The toast after a restore. */
export const putBackTitle = (files: number, label: string): string => `${plural(files, 'file')} put back to before “${label}”`;

/** A row of the list the keys move over: a turn, a file of the open turn, and "All checkpoints…" last. */
export type RewindItem = { kind: 'turn'; n: number } | { kind: 'file'; n: number; path: string } | { kind: 'all' };

/**
 * Where the keys are in the list. `at`: the index of the active row in `rewindItems`; `open`: the
 * turn whose files are listed; `asking`: the restore whose confirm line shows (`path` null: the
 * whole turn).
 */
export type RewindNav = { at: number; open: number | null; asking: { n: number; path: string | null } | null };

/** What a key asks of the list's component. */
export type RewindAct = { kind: 'restore'; n: number; path: string | null } | { kind: 'close' } | { kind: 'sheet' } | null;

/** The rows the keys move over, in order: each turn, the files of the open one under it, then "All checkpoints…". */
export function rewindItems(rows: RewindRow[], open: number | null): RewindItem[] {
  const items: RewindItem[] = [];
  for (const r of rows) {
    items.push({ kind: 'turn', n: r.n });
    if (r.n === open) for (const path of r.files) items.push({ kind: 'file', n: r.n, path });
  }
  items.push({ kind: 'all' });
  return items;
}

/** A turn with nothing to put back cannot be picked; every other row can. */
const pickable = (rows: RewindRow[], it: RewindItem): boolean => it.kind !== 'turn' || (rows.find((r) => r.n === it.n)?.changed ?? 0) > 0;

/** Where the keys start: on the newest turn that has something to put back. */
export function rewindStart(rows: RewindRow[]): RewindNav {
  const items = rewindItems(rows, null);
  return { at: Math.max(0, items.findIndex((it) => pickable(rows, it))), open: null, asking: null };
}

/** The turn's files listed under it, or no more: the keys stay on that turn. */
export function toggleFiles(nav: RewindNav, rows: RewindRow[], n: number): RewindNav {
  const row = rows.find((r) => r.n === n);
  if (!row?.changed) return nav;
  const open = nav.open === n ? null : n;
  return { at: rewindItems(rows, open).findIndex((it) => it.kind === 'turn' && it.n === n), open, asking: null };
}

/** The confirm line for row `at` (a turn or one of its files); "All checkpoints…" opens the sheet. */
export function pick(nav: RewindNav, rows: RewindRow[], at: number): { nav: RewindNav; act: RewindAct } {
  const it = rewindItems(rows, nav.open)[at];
  if (!it || !pickable(rows, it)) return { nav, act: null };
  if (it.kind === 'all') return { nav: { ...nav, at }, act: { kind: 'sheet' } };
  return { nav: { ...nav, at, asking: { n: it.n, path: it.kind === 'file' ? it.path : null } }, act: null };
}

/**
 * A key in the list. ↑ ↓ move over the rows that can be picked, → lists the files of the turn
 * (← and Esc close them), ↵ shows the confirm line in place and a second ↵ restores. Esc closes
 * one level: the confirm line, then the files, then the list.
 */
export function rewindKey(nav: RewindNav, rows: RewindRow[], key: string): { nav: RewindNav; act: RewindAct } {
  const items = rewindItems(rows, nav.open);
  const here = items[nav.at];
  if (nav.asking) {
    if (key === 'Enter') return { nav, act: { kind: 'restore', ...nav.asking } };
    if (key === 'Escape') return { nav: { ...nav, asking: null }, act: null };
    return { nav, act: null };
  }
  if (key === 'ArrowDown' || key === 'ArrowUp') {
    const step = key === 'ArrowDown' ? 1 : -1;
    for (let i = 1; i <= items.length; i++) {
      const at = (((nav.at + step * i) % items.length) + items.length) % items.length;
      if (pickable(rows, items[at])) return { nav: { ...nav, at }, act: null };
    }
    return { nav, act: null };
  }
  if (key === 'ArrowRight' && here?.kind === 'turn' && nav.open !== here.n) return { nav: toggleFiles(nav, rows, here.n), act: null };
  if ((key === 'ArrowLeft' || key === 'Escape') && nav.open !== null) return { nav: toggleFiles(nav, rows, nav.open), act: null };
  if (key === 'Escape') return { nav, act: { kind: 'close' } };
  if (key === 'Enter') return pick(nav, rows, nav.at);
  return { nav, act: null };
}

/**
 * The checkpoint "Put this file back" of the sheet's Last turn restores from: the newest turn
 * checkpoint of the list, only when the list is of the worktree the turn shown was read in (the
 * `task` of both answers agree) and it is that turn's point (taken at the turn's start). Null:
 * no button, Checkpoints below says the rest.
 */
export function turnPoint(turn: Pick<LastTurn, 'ts' | 'task'>, kept: Pick<AgentCheckpoints, 'off' | 'checkpoints' | 'task'>): number | null {
  if (kept.off || kept.task !== turn.task) return null;
  const newest = kept.checkpoints.find((c) => c.kind === 'turn');
  return newest && newest.ts === turn.ts ? newest.n : null;
}
