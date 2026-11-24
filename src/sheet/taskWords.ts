// The words of the task sheet's head, and when its Git tab offers a merge. Pure, so the sheet
// and the tests agree.
import type { AgentGit } from '../lib/gen/AgentGit';
import type { Task } from '../lib/gen/Task';
import { waitLine } from '../home/threadWords';
import { TASK_STATUS_LABEL } from '../lib/util/labels';

/**
 * The task's status line: its status and the core's reason, said once. A reason that starts
 * with the status ("cancelled by you" on a cancelled task, "Ready: a free agent picks it up") is
 * the whole line ("Cancelled by you"). A task that waits on others (still in the backlog or ready)
 * says what it waits on as the thread does (`waitLine`: "Can’t start · t1 was cancelled"), not
 * the reason the core wrote when it began to wait. `deps`: the tasks it depends on.
 */
export function taskStatusLine(task: Pick<Task, 'status' | 'status_reason'>, deps: readonly Pick<Task, 'key' | 'status'>[]): string {
  const label = TASK_STATUS_LABEL[task.status];
  const waits = task.status === 'pending' || task.status === 'ready' ? waitLine(deps.filter((d) => d.status !== 'done')) : null;
  const reason = task.status_reason?.trim() || null;
  if (waits && (!reason || /^waiting on /i.test(reason))) return `${label} — ${waits.text}`;
  if (!reason) return label;
  const said = reason.toLowerCase().replace(/[.\s]+$/, '');
  const named = label.toLowerCase();
  if (said === named || said.startsWith(`${named} `) || said.startsWith(`${named}:`)) return reason[0].toUpperCase() + reason.slice(1).replace(/[.\s]+$/, '');
  return `${label} — ${reason}`;
}

/**
 * Whether an agent's Git tab offers "Merge into <target>": its worktree has a change, or its
 * branch has a commit that is not the tip of the branch it merges into. A branch whose tip is
 * the target's own tip has nothing to merge. When the target is not in the list of branches,
 * it cannot be told: the merge is offered and the core says what it finds.
 */
export function hasSomethingToMerge(view: Pick<AgentGit, 'branch' | 'files' | 'branches'>, target: string): boolean {
  if (!view.branch) return false;
  if (view.files.length) return true;
  const sha = (name: string) => view.branches.find((b) => b.name === name)?.sha ?? null;
  const mine = sha(view.branch);
  const theirs = sha(target);
  if (!mine || !theirs) return true;
  return !(mine.startsWith(theirs) || theirs.startsWith(mine));
}
