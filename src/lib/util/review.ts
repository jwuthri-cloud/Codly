// Reviewing a diff: where a comment sits, the one message "Send back" builds from all of them,
// which files open collapsed, how many lines a file draws, and where a file opens in the editor.
// Pure functions; what you wrote and ticked in this session lives in review.svelte.ts.
import type { Agent } from '../gen/Agent';
import type { Plan } from '../gen/Plan';
import type { Task } from '../gen/Task';
import { parseDiff, type DiffFile, type DiffHunk, type DiffLine } from './diff';

/** Where a comment sits: a line of a file's diff (in the new file, or in the old one for a line
 *  the change removes), or the whole file (`line` null). `quote` is that line as the diff shows it. */
export type Anchor = { path: string; line: number | null; side: 'old' | 'new' | null; quote: string | null };
export type ReviewComment = Anchor & { text: string };

/** The place of a comment on the whole file. */
export const FILE_PLACE = 'file';
/** One comment per place: this names the place inside its file. */
export const placeId = (a: Pick<Anchor, 'line' | 'side'>): string => (a.line == null ? FILE_PLACE : `${a.side}:${a.line}`);
const sameSpot = (a: Anchor, b: Anchor) => a.path === b.path && placeId(a) === placeId(b);

export const fileAnchor = (path: string): Anchor => ({ path, line: null, side: null, quote: null });

/** `placeId` of a diff line, without building its anchor (the view asks for every line it draws).
 *  Null: not a line of the file (git's "no newline at end of file" note), so no comment sits there. */
export function linePlace(l: DiffLine): string | null {
  if (l.kind === 'meta') return null;
  return l.kind === 'del' ? `old:${l.oldNo}` : `new:${l.newNo}`;
}

/** The place of a diff line that takes comments (`linePlace` is not null). */
export function lineAnchor(path: string, l: DiffLine): Anchor {
  if (l.kind === 'del') return { path, line: l.oldNo, side: 'old', quote: `-${l.text}` };
  return { path, line: l.newNo, side: 'new', quote: `${l.kind === 'add' ? '+' : ' '}${l.text}` };
}

/** Add a comment, or replace the one at the same place. Empty text removes it. */
export function withComment(list: ReviewComment[], at: Anchor, text: string): ReviewComment[] {
  const rest = list.filter((c) => !sameSpot(c, at));
  const t = text.trim();
  return t ? [...rest, { ...at, text: t }] : rest;
}

/** Reading order: the diff's file order; in a file, its own comment first, then by line (a removed
 *  line before the new line with the same number). A file that left the diff comes last. */
export function inOrder(list: ReviewComment[], paths: string[]): ReviewComment[] {
  const rank = new Map(paths.map((p, i) => [p, i]));
  const file = (c: ReviewComment) => rank.get(c.path) ?? paths.length;
  const side = (c: ReviewComment) => (c.side === 'old' ? 0 : 1);
  return [...list].sort((a, b) => file(a) - file(b) || a.path.localeCompare(b.path) || (a.line ?? 0) - (b.line ?? 0) || side(a) - side(b));
}

/** A quoted line is context, not the whole file: a very long one (minified code) is cut. */
const QUOTE_MAX = 200;

/** Every comment as one message. For each: the file and line, the quoted line, the comment. */
export function reviewMessage(list: ReviewComment[], paths: string[]): string {
  const items = inOrder(list, paths).map((c, i) => {
    const where = c.line == null ? `\`${c.path}\` (the whole file)` : `\`${c.path}:${c.line}\`${c.side === 'old' ? ' (a line the change removes)' : ''}`;
    const quote = c.quote == null ? [] : [`   > ${c.quote.length > QUOTE_MAX ? `${c.quote.slice(0, QUOTE_MAX - 1)}…` : c.quote}`];
    return [`${i + 1}. ${where}`, ...quote, ...c.text.split('\n').map((l) => `   ${l}`)].join('\n');
  });
  return `Review comments (${items.length}):\n\n${items.join('\n\n')}`;
}

/** The request that a review of merged or finished work opens: what you reviewed, where its work
 *  is, then the comments. A new Lead reads it, so it names everything. */
export function followUpRequest(task: Task | null, plan: Plan | null, list: ReviewComment[], paths: string[]): string {
  const what = task ? `task ${task.key} “${task.title}”${plan ? ` of the plan “${plan.summary}”` : ''}` : `the plan “${plan?.summary ?? ''}”`;
  const branch = plan?.integration_branch ?? task?.branch;
  const where = plan?.merged_sha ? ['Its work is merged.'] : branch ? [`Its work is on the branch ${branch}.`] : [];
  return `${[`I reviewed ${what}.`, ...where, 'Please make these changes.'].join(' ')}\n\n${reviewMessage(list, paths)}`;
}

// ───────────── large diffs ─────────────

/** A file with more changed lines than this opens collapsed. */
export const LARGE_FILE = 400;
/** Lines a file draws at first; "Show more" draws this many again. */
export const LINE_STEP = 500;

const LOCKFILE = /(^|\/)(package-lock\.json|npm-shrinkwrap\.json|pnpm-lock\.yaml|yarn\.lock|bun\.lockb?|Cargo\.lock|Gemfile\.lock|composer\.lock|poetry\.lock|uv\.lock|Pipfile\.lock|go\.sum|flake\.lock)$/;
const GENERATED = /(^|\/)(node_modules|vendor|dist|__generated__)\/|\.min\.(js|css)$|\.(map|snap)$|\.pb\.go$|_pb2\.py$|\.generated\.[a-z]+$/;

/** Why a file opens collapsed: nobody reads these line by line, and drawing them is what makes a
 *  large diff slow. Null: it opens like any file. */
export function heavy(f: DiffFile): 'lockfile' | 'generated' | 'large' | null {
  if (LOCKFILE.test(f.path)) return 'lockfile';
  if (GENERATED.test(f.path)) return 'generated';
  return f.added + f.removed > LARGE_FILE ? 'large' : null;
}

export const HEAVY_LABEL: Record<NonNullable<ReturnType<typeof heavy>>, string> = { lockfile: 'Lockfile', generated: 'Generated', large: 'Large' };

export const lineCount = (f: DiffFile): number => f.hunks.reduce((n, h) => n + h.lines.length, 0);

/** The first `limit` lines of a file's diff, hunk by hunk (the last hunk may be cut). */
export function firstLines(f: DiffFile, limit: number): { hunk: DiffHunk; lines: DiffLine[] }[] {
  const out: { hunk: DiffHunk; lines: DiffLine[] }[] = [];
  let left = limit;
  for (const hunk of f.hunks) {
    if (left <= 0) break;
    out.push({ hunk, lines: hunk.lines.length <= left ? hunk.lines : hunk.lines.slice(0, left) });
    left -= hunk.lines.length;
  }
  return out;
}

/** How many lines a file draws: `limit`, or as far as its last commented line when that is
 *  further down, so a comment you wrote is never out of sight. `commented`: the places with one. */
export function linesToDraw(f: DiffFile, limit: number, commented: { size: number; has(place: string): boolean }): number {
  if (!commented.size) return limit;
  let n = 0;
  let last = 0;
  for (const hunk of f.hunks) {
    for (const l of hunk.lines) {
      n++;
      const place = n > limit ? linePlace(l) : null;
      if (place && commented.has(place)) last = n;
    }
  }
  return Math.max(limit, last);
}

/** Each file's part of a diff text → the files parsed from it. */
export type DiffParts = Map<string, DiffFile[]>;

/** `parseDiff`, keeping the file objects of the last parse (`prev`) for every file whose part of
 *  the text did not change: the view then builds again only the files that did change. */
export function parseDiffKeeping(text: string, prev: DiffParts): { files: DiffFile[]; parts: DiffParts } {
  const parts: DiffParts = new Map();
  const files: DiffFile[] = [];
  // A file's part starts at its "diff --git" line; no line inside a hunk can start that way.
  for (const part of text.replace(/\r\n/g, '\n').split(/^(?=diff --git )/m)) {
    const parsed = prev.get(part) ?? parseDiff(part);
    parts.set(part, parsed);
    files.push(...parsed);
  }
  return { files, parts };
}

/** What a "viewed" tick stands for: this very diff of the file. A file that changed since is
 *  not viewed any more. */
export function fileMark(f: DiffFile): string {
  let h = 5381;
  for (const hunk of f.hunks) {
    for (const l of hunk.lines) {
      h = (h * 33) ^ l.kind.charCodeAt(0);
      for (let i = 0; i < l.text.length; i++) h = (h * 33) ^ l.text.charCodeAt(i);
    }
  }
  return `${f.status}:${f.added}:${f.removed}:${f.hunks.length}:${h >>> 0}`;
}

// ───────────── open in the editor ─────────────

/** Where a file of a diff opens in the editor: the worktree of `agentId` (the repository itself
 *  for a Lead, which has no worktree), or nowhere, with the reason. */
export type EditorPlace = { agentId: string | null; tip: string };

/**
 * `task`: the task whose work the file is (the diff's task, or the one that wrote the file in a
 * plan's diff). `worker`: its agent. `lead`: a Lead of the open repository.
 * - The task's worktree is still its worker's, on this task: the file opens there, as the worker
 *   left it. (A merged task's worktree is removed, and a team member's moves on to its next task.)
 * - The plan is merged: the file is in your branch, so it opens in the repository.
 * - Else the work is only on a branch that no editor is open on.
 */
export function editorPlace(task: Task | null, plan: Plan | null, worker: Agent | null, lead: Agent | null): EditorPlace {
  const kept = task && (task.status !== 'done' || !task.plan_id) ? task : null;
  if (kept && worker?.worktree && kept.worktree === worker.worktree && (worker.task_id == null || worker.task_id === kept.id)) {
    return { agentId: worker.id, tip: `Opens the file in ${worker.name}’s worktree` };
  }
  if (plan?.merged_sha && lead && !lead.worktree) return { agentId: lead.id, tip: 'Opens the file in your repository: the plan is merged' };
  const branch = plan?.integration_branch ?? task?.branch;
  return { agentId: null, tip: branch ? `This work is only on the branch ${branch} for now` : 'No worktree has this file' };
}
