// Reviewing a diff: where a comment sits, the one message "Send back" builds from all of them,
// which files open collapsed, how many lines a file draws, when a task's last turn is read
// again, where a file opens in the editor, what the Changes room lists (plans, tasks, 1:1
// agents), and where Send back takes each comment of a plan's diff.
// Pure functions; what you wrote and ticked in this session lives in review.svelte.ts.
import type { Agent } from '../gen/Agent';
import type { AgentStatus } from '../gen/AgentStatus';
import type { Plan } from '../gen/Plan';
import type { PrState } from '../gen/PrState';
import type { Task } from '../gen/Task';
import type { TaskStatus } from '../gen/TaskStatus';
import { parseDiff, type DiffFile, type DiffHunk, type DiffLine } from './diff';
import { uniqueKeys } from './keys';

/** Where a comment sits: a line of a file's diff (in the new file, or in the old one for a line
 *  the change removes), or the whole file (`line` null). `quote` is that line as the diff shows it. */
export type Anchor = { path: string; line: number | null; side: 'old' | 'new' | null; quote: string | null };
export type ReviewComment = Anchor & { text: string };

/** The place of a comment on the whole file. */
export const FILE_PLACE = 'file';
/** This names a place inside its file. */
export const placeId = (a: Pick<Anchor, 'line' | 'side'>): string => (a.line == null ? FILE_PLACE : `${a.side}:${a.line}`);
/** One comment per place as you saw it: a line that reads differently by now is another place,
 *  so a new comment there does not replace the one you wrote on the old line. */
export const sameSpot = (a: Anchor, b: Anchor) => a.path === b.path && placeId(a) === placeId(b) && a.quote === b.quote;

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

/** A file's comments by where they sit in its diff now. `placed` (place → comment): on the file,
 *  or on a line the diff still shows as you quoted it. `lost`: on a line that left the diff or
 *  reads differently by now (the worker changed the file since); no line of the diff is theirs. */
export function placeComments(f: DiffFile, list: ReviewComment[]): { placed: Map<string, ReviewComment>; lost: ReviewComment[] } {
  const placed = new Map<string, ReviewComment>();
  const onLines = list.filter((c) => c.line != null);
  const onFile = list.find((c) => c.line == null);
  if (onFile) placed.set(FILE_PLACE, onFile);
  if (!onLines.length) return { placed, lost: onLines };
  const wanted = new Set(onLines.map(placeId));
  for (const hunk of f.hunks) {
    for (const l of hunk.lines) {
      const place = linePlace(l);
      if (!place || !wanted.has(place)) continue;
      const quote = lineAnchor(f.path, l).quote;
      const c = onLines.find((c) => placeId(c) === place && c.quote === quote);
      if (c) placed.set(place, c);
    }
  }
  return { placed, lost: onLines.filter((c) => placed.get(placeId(c)) !== c) };
}

/** The comments on files that are not in the diff (`paths`: its files): a file the worker
 *  changed back, or removed from its work, since you wrote them. */
export function offDiff(list: ReviewComment[], paths: string[]): ReviewComment[] {
  const inDiff = new Set(paths);
  return list.filter((c) => !inDiff.has(c.path));
}

/**
 * A key for the row of each comment, in the list's order: no two the same, and a comment keeps
 * its key when another one is removed, so its row (and a button in it that has the focus) goes
 * with it and is never handed to the next comment. A comment has no id and no time of its own:
 * its file, its place and the line as you quoted it name it (`sameSpot`, one comment per such
 * place). Should two ever share all three, the later one adds its position among them.
 */
export function commentKeys(list: readonly ReviewComment[]): string[] {
  // As JSON: the three parts cannot run into each other, whatever a path or a quoted line holds.
  return uniqueKeys(list, (c) => JSON.stringify([c.path, placeId(c), c.quote]));
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

/** How the message names a comment, above your words: its file and line, then the quoted line. */
function commentHead(c: ReviewComment): string {
  const where = c.line == null ? `\`${c.path}\` (the whole file)` : `\`${c.path}:${c.line}\`${c.side === 'old' ? ' (a line the change removes)' : ''}`;
  return c.quote == null ? where : `${where}\n   > ${c.quote.length > QUOTE_MAX ? `${c.quote.slice(0, QUOTE_MAX - 1)}…` : c.quote}`;
}

/** Every comment as one message. For each: the file and line, the quoted line, the comment. */
export function reviewMessage(list: ReviewComment[], paths: string[]): string {
  const items = inOrder(list, paths).map((c, i) => [`${i + 1}. ${commentHead(c)}`, ...c.text.split('\n').map((l) => `   ${l}`)].join('\n'));
  return `Review comments (${items.length}):\n\n${items.join('\n\n')}`;
}

/**
 * The comments of a message Send back built (`comments`) that a message you sent (`text`) still
 * names: the file and line, and the quoted line, as Send back wrote them. You may have changed
 * the words around them and your own words under them before you sent it: a comment that is
 * still named went with the message. One whose lines you took out of it did not.
 */
export const sentIn = (text: string, comments: ReviewComment[]): ReviewComment[] => comments.filter((c) => text.includes(commentHead(c)));

/** The request that a review of merged work opens: what you reviewed, where its work is, then
 *  the comments. A new Lead reads it, so it names everything. */
export function followUpRequest(task: Task | null, plan: Plan | null, list: ReviewComment[], paths: string[]): string {
  const what = task ? `task ${task.key} “${task.title}”${plan ? ` of the plan “${plan.summary}”` : ''}` : `the plan “${plan?.summary ?? ''}”`;
  const branch = plan?.integration_branch ?? task?.branch;
  const where = plan?.merged_sha ? ['Its work is merged.'] : branch ? [`Its work is on the branch ${branch}.`] : [];
  return `${[`I reviewed ${what}.`, ...where, 'Please make these changes.'].join(' ')}\n\n${reviewMessage(list, paths)}`;
}

/**
 * Where Send back takes the comments on a plan's finished work (a done task's diff, or the files
 * of a plan's diff that no task at work takes):
 * - `plan`: the plan is finished and not in your branch: they become one more task of it, for
 *   the agent that wrote the files (`send_back_plan`);
 * - `wait`: the plan still runs and will finish by itself: they stay on the diff until it is
 *   done, and you press Send back again then (nothing is sent by itself);
 * - `request`: no plan, or one that is merged, kept on its branch, failed or cancelled, or one
 *   that runs but does not move on without you (each task left is blocked or failed): a new
 *   request;
 * - `pull request`: the plan is finished and has a pull request, open or merged (`pr`: its state
 *   as Codly last read it; null when none is on record or it is not read yet): a new request too,
 *   as the core says (`send_back_plan` refuses such a plan).
 * `tasks`: the tasks of the board (those of `plan` are read).
 */
export type FinishedGoes = 'plan' | 'wait' | 'request' | 'pull request';

/** The task statuses of a plan that still moves on by itself. */
const MOVING: readonly TaskStatus[] = ['pending', 'ready', 'running', 'verifying', 'reviewing', 'changes_requested', 'merging'];

export function finishedWorkGoes(plan: Plan | null, tasks: readonly Task[], pr: PrState | null): FinishedGoes {
  if (!plan || plan.merged_sha || plan.kept) return 'request';
  if (plan.status === 'done') return pr === 'open' || pr === 'merged' ? 'pull request' : 'plan';
  if (plan.status === 'integrating') return 'wait';
  if (plan.status !== 'running') return 'request';
  const own = tasks.filter((t) => t.plan_id === plan.id);
  const stuck = !own.some((t) => MOVING.includes(t.status)) && own.some((t) => t.status === 'blocked' || t.status === 'failed');
  return stuck ? 'request' : 'wait';
}

/**
 * What the review bar says after "N comments not sent." on a task's or a plan's diff. `worker`:
 * the agent of a task that is not done (it gets them itself). Else `goes` says where the
 * comments on finished work go. `inRequest` as for `reviewBarWords`: on a finished plan, a
 * request holds them when the core sent them there before (the plan has a pull request), and
 * Send back takes them there again. `writerOnTeam`: the agent that wrote the commented files is
 * still on the team (`writerOnTeam`), so the new task is for it; else an agent of the team takes it.
 */
export function sendBackBarWords(total: number, worker: string | null, goes: FinishedGoes, inRequest: number | null, writerOnTeam: boolean): string {
  const them = total === 1 ? 'it' : 'them';
  const forWhom = writerOnTeam ? 'the agent that wrote these files' : 'an agent of the team';
  if (worker || goes === 'request') return reviewBarWords(total, worker, inRequest);
  if (goes === 'wait') return `This plan is still running, and nothing is sent yet: when it is done, press Send back and it makes ${them} one more task of it, for ${forWhom}.`;
  if (inRequest != null) return reviewBarWords(total, null, inRequest);
  if (goes === 'pull request') return 'Send back opens a new request: this plan has a pull request.';
  return `Send back makes ${them} one more task of this plan, for ${forWhom}.`;
}

/**
 * Is the agent that the core gives a plan's review comments to (review_back.rs `wrote_most`)
 * still on the team? Of the plan's finished tasks that wrote a commented file (`owners`: which
 * task wrote each file of the diff), the one with most of the commented files, the first one on
 * a tie; a task on the plan's own branch (an earlier review, a resolution) does not count.
 * `onTeam`: that task's agent is still on the team (in the window's agent map, not let go).
 */
export function writerOnTeam(list: readonly ReviewComment[], owners: ReadonlyMap<string, Task>, plan: Plan | null, onTeam: (agentId: string) => boolean): boolean {
  const count = new Map<Task, number>();
  for (const path of new Set(list.map((c) => c.path))) {
    const t = owners.get(path);
    const inPlace = !!plan?.integration_branch && t?.branch === plan.integration_branch;
    if (t && t.status === 'done' && !inPlace) count.set(t, (count.get(t) ?? 0) + 1);
  }
  let best: { task: Task; n: number } | null = null;
  for (const [task, n] of count) if (!best || n > best.n) best = { task, n };
  return !!best?.task.assignee && onTeam(best.task.assignee);
}

/**
 * What the review bar says after "N comments not sent.": where Send back takes them. `worker`:
 * the agent of a task that is not done (it gets them itself). `inRequest`: for finished work
 * whose comments are in a new request already, how many of them that request's message holds
 * as they are now (one you wrote or changed since Send back is not in it yet); null when no
 * request holds them.
 */
export function reviewBarWords(total: number, worker: string | null, inRequest: number | null): string {
  const one = total === 1;
  if (worker) return `Send back gives ${one ? 'it' : 'them'} to ${worker}; Codly checks the work again after the changes.`;
  if (inRequest == null) return `This work is finished: Send back opens a new request with your ${one ? 'comment' : 'comments'}, ready to send.`;
  const rest = total - inRequest;
  if (!rest) return `${one ? 'It is' : 'They are'} in a new request, ready to send, and ${one ? 'stays' : 'stay'} here until you send its message. Send back opens that request.`;
  if (!inRequest) return `A new request holds your ${one ? 'comment as it was' : 'comments as they were'} at Send back, not as ${one ? 'it is' : 'they are'} now. Send back brings that request up to date.`;
  return `${inRequest} ${inRequest === 1 ? 'is' : 'are'} in a new request, ready to send; ${rest} ${rest === 1 ? 'is' : 'are'} not in it yet. Send back puts ${rest === 1 ? 'it' : 'them'} in that request too.`;
}

/**
 * Where a diff's comments are besides the diff, when you are about to discard them. `seeded`: a
 * draft request holds them as Send back put them, and Discard takes them out of it. `edited`:
 * you changed that message since, so the draft is yours and stays. Null: nowhere else.
 */
export type ReviewDraft = 'seeded' | 'edited' | null;

/** What the review bar says while Discard waits for your second press. */
export function discardWords(total: number, draft: ReviewDraft): string {
  const one = total === 1;
  if (draft === 'seeded') return `Discard also takes ${one ? 'it' : 'them'} out of the new request.`;
  if (draft === 'edited') return `${one ? 'It is' : 'They are'} also in a draft request, in a message you changed: that draft stays as you wrote it.`;
  return `${one ? 'It is' : 'They are'} not kept anywhere else.`;
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

// ───────────── the last turn of a task ─────────────

const AT_REST: AgentStatus[] = ['idle', 'done', 'failed', 'offline'];

/** Is a turn of the agent on? It starts, thinks, works, or waits (for you, or for a limit). */
export const inTurn = (status: AgentStatus | undefined): boolean => status != null && !AT_REST.includes(status);

/**
 * The "Last turn" diff of a task is read again when this value changes: the task moved on, or
 * a turn of its agent started or ended. Not on each file the agent writes, as the whole task's
 * diff is: a read of the last turn takes a snapshot of the worktree (git hashes every changed
 * file there, also one that is half-written). The steps inside a turn are one value.
 */
export function turnReadKey(task: TaskStatus | undefined, agent: AgentStatus | undefined): string {
  return `${task}:${inTurn(agent) ? 'turn' : 'rest'}`;
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

/** Where a file of a 1:1 agent's diff opens: its own worktree (your project folder for an agent
 *  that works there), as it is now. `onTask`: the key of the task whose worktree the diff on
 *  screen is of (its last turn, while it is on that task): the file is not in its own worktree. */
export function agentEditorPlace(agent: Agent | null, projectFolder: string | null, onTask: string | null): EditorPlace {
  if (agent && onTask) return { agentId: null, tip: `This file is in the worktree of ${onTask}, not in ${agent.name}’s own` };
  if (!agent?.worktree) return { agentId: null, tip: 'No worktree has this file' };
  return { agentId: agent.id, tip: agent.worktree === projectFolder ? 'Opens the file in your project folder' : `Opens the file in ${agent.name}’s worktree` };
}

// ───────────── what the Changes room lists ─────────────

/** The chip of the Changes room that lists the 1:1 agents (the others are a plan's id, 'loose' and 'merged'). */
export const AGENTS_GROUP = 'agents';

/**
 * The chip a diff is under: a plan's id (a plan you have not merged), 'loose' (a task with no
 * plan shown), 'merged' (a plan in your branch, or kept on its own) or `AGENTS_GROUP` (a 1:1
 * agent's work). `planOfTask`: the plan of a task; `settled`: the merged or kept plans;
 * `shown`: a plan that has a chip of its own.
 */
export function diffGroup(
  target: { kind: 'plan' | 'task' | 'agent'; id: string } | null,
  planOfTask: (taskId: string) => string | null | undefined,
  settled: ReadonlySet<string>,
  shown: (planId: string) => boolean,
): string | null {
  if (!target) return null;
  if (target.kind === 'agent') return AGENTS_GROUP;
  const pid = target.kind === 'plan' ? target.id : planOfTask(target.id);
  if (pid && settled.has(pid)) return 'merged';
  return pid && shown(pid) ? pid : 'loose';
}

/**
 * The 1:1 agents the Changes room lists: each one with a worktree that changed files there, as
 * far as Codly saw (`changed`; the diff itself says the rest). The one you opened (`picked`, from
 * its sheet) is listed too, so its diff is never off the list while you read it.
 */
export function agentsWithChanges(agents: readonly Agent[], changed: (a: Agent) => boolean, picked: string | null): Agent[] {
  return agents.filter((a) => a.role === 'solo' && !a.archived && !!a.worktree && (a.id === picked || changed(a)));
}

/** Which task wrote each file of a diff: the first task of `scope` that names it (`filesOf`: the
 *  files of its commits, then the ones its agent changes now). */
export function fileOwners(scope: readonly Task[], filesOf: (t: Task) => Iterable<string>): Map<string, Task> {
  const out = new Map<string, Task>();
  for (const t of scope) for (const path of filesOf(t)) if (!out.has(path)) out.set(path, t);
  return out;
}

/**
 * Who takes the comments on each file of a plan's whole diff, for Send back (`splitByTask`).
 * That diff holds merged work only, and a task is merged on its way to Done: a file that a done
 * task's commits name is that task's (`committed`), even when a task at work touched it too, so
 * its comments go to a new request. A task at work takes a file only when no done task names
 * it (`filesOf`: the files of its commits, then the ones its agent changes now; an agent's
 * files now are of its current task, not of a task it finished).
 */
export function splitOwners(scope: readonly Task[], committed: (t: Task) => Iterable<string>, filesOf: (t: Task) => Iterable<string>): Map<string, Task> {
  const out = fileOwners(scope.filter((t) => t.status === 'done'), committed);
  for (const [path, t] of fileOwners(scope.filter((t) => t.status !== 'done'), filesOf)) if (!out.has(path)) out.set(path, t);
  return out;
}

/** A task still at work takes review comments itself; a done or cancelled one does not. */
const takesComments = (t: Task) => t.status !== 'done' && t.status !== 'cancelled';

/**
 * Send back on a plan's whole diff: each comment goes where its file's work is. The comments on
 * a file of a task still at work go to that task (`tasks`, in the order of their first comment);
 * the ones on a file of a done or cancelled task, or of no task, are the `rest`: they go where
 * the plan's finished work goes (`finishedWorkGoes`).
 */
export function splitByTask(list: readonly ReviewComment[], owners: ReadonlyMap<string, Task>): { tasks: { task: Task; comments: ReviewComment[] }[]; rest: ReviewComment[] } {
  const tasks: { task: Task; comments: ReviewComment[] }[] = [];
  const rest: ReviewComment[] = [];
  for (const c of list) {
    const t = owners.get(c.path);
    if (!t || !takesComments(t)) {
      rest.push(c);
      continue;
    }
    const to = tasks.find((x) => x.task.id === t.id);
    if (to) to.comments.push(c);
    else tasks.push({ task: t, comments: [c] });
  }
  return { tasks, rest };
}

/** One part of a split Send back: how many comments go to whom, and the task they are on. */
export type SplitPart = { n: number; who: string; key: string };

/** Where the `rest` of a split Send back go (`finishedWorkGoes`), after the press and before it. */
const TO_REQUEST: { sent: [string, string]; before: [string, string] } = {
  sent: ['is in a new request', 'are in a new request'],
  before: ['goes to a new request', 'go to a new request'],
};
const REST_WORDS: Record<FinishedGoes, { sent: [string, string]; before: [string, string] }> = {
  request: TO_REQUEST,
  'pull request': TO_REQUEST,
  wait: {
    sent: ['stays here: send it back when the plan is done', 'stay here: send them back when the plan is done'],
    before: ['stays here until the plan is done', 'stay here until the plan is done'],
  },
  plan: { sent: ['is one more task of this plan', 'are one more task of this plan'], before: ['goes to one more task of this plan', 'go to one more task of this plan'] },
};

/**
 * Where a split Send back takes the comments: "2 comments to Ada (t1), 1 to Bo (t2); 1 stays
 * here: send it back when the plan is done". `sent`: false says it before the press ("…; 1 stays
 * here until the plan is done"). `goes`: where the comments that no task at work takes go.
 */
export function splitWords(parts: readonly SplitPart[], rest: number, sent: boolean, goes: FinishedGoes): string {
  const to = parts.map((p, i) => `${p.n}${i ? '' : p.n === 1 ? ' comment' : ' comments'} to ${p.who} (${p.key})`).join(', ');
  if (!rest) return to;
  const words = REST_WORDS[goes][sent ? 'sent' : 'before'][rest === 1 ? 0 : 1];
  return to ? `${to}; ${rest} ${words}` : `${rest} ${words}`;
}

/** What an agent's diff is when it works in your project folder itself (`inProjectFolder`). */
export const IN_FOLDER_WORDS = 'Every uncommitted change in your project folder, your own edits included';

/** Does `agent` work in your project folder itself (`repoPath`), not in a worktree of its own? */
export const inProjectFolder = (agent: Agent | null, repoPath: string | null | undefined): boolean => !!agent?.worktree && agent.worktree === repoPath;

/** The Changes room's title and the line under it, on a 1:1 agent's diff. */
export function agentDiffHead(agent: Agent, repoPath: string | null | undefined): { title: string; sub: string } {
  return { title: agent.name, sub: inProjectFolder(agent, repoPath) ? IN_FOLDER_WORDS : `What ${agent.name} changed in its own worktree` };
}

/** What the review bar says after "N comments not sent." on a 1:1 agent's diff. */
export function agentBarWords(total: number, agent: string): string {
  const them = total === 1 ? 'it' : 'them';
  return `Send gives ${them} to ${agent} as one message in your 1:1 chat; if ${agent} is working, it reads ${them} when its turn ends.`;
}
