// Requests that wait for the same answer: the same command in the same place of each agent's
// worktree, with the same risks and the same choices, asked by several agents of one
// conversation. The conversation shows them as one card, and one answer goes to all of them.
import type { Approval } from '../gen/Approval';
import { store } from './store.svelte';

/** Where a command runs, seen from the worktree of the agent that asked ('' is its root);
 *  null when the request does not say, or the place is not inside that worktree. */
function placeInWorktree(a: Approval): string | null {
  const cwd = a.subject.kind === 'command' ? a.subject.cwd : null;
  const root = store.agentMap.get(a.agent_id)?.worktree;
  if (!cwd || !root) return null;
  const trim = (p: string) => p.replace(/\/+$/, '');
  const [dir, base] = [trim(cwd), trim(root)];
  if (dir === base) return '';
  return dir.startsWith(`${base}/`) ? dir.slice(base.length + 1) : null;
}

/** What makes two waiting requests the same question; null for a request that stands alone
 *  (answered ones, questions, file changes, tools: their cards differ in more than the agent;
 *  a command whose place in the worktree is not known: the card shows one directory only). */
export function sameQuestion(a: Approval): string | null {
  if (a.status !== 'pending' || a.subject.kind !== 'command') return null;
  const place = placeInWorktree(a);
  if (place === null) return null;
  return JSON.stringify([a.subject.command, place, [...a.risks].sort(), [...a.choices].sort()]);
}

export type ApprovalGroup = { approval: Approval; twins: Approval[] };

/** One entry per question, in the order asked: the oldest request, and the later ones that ask the same. */
export function groupSame(list: Approval[]): ApprovalGroup[] {
  const byQuestion = new Map<string, ApprovalGroup>();
  const out: ApprovalGroup[] = [];
  for (const a of [...list].sort((x, y) => x.created_at - y.created_at)) {
    const key = sameQuestion(a);
    const group = key === null ? undefined : byQuestion.get(key);
    if (group) {
      group.twins.push(a);
      continue;
    }
    const entry: ApprovalGroup = { approval: a, twins: [] };
    if (key !== null) byQuestion.set(key, entry);
    out.push(entry);
  }
  return out;
}

/** The requests of `list` that ask the same as `a` (itself left out). */
export function twinsIn(list: Approval[], a: Approval): Approval[] {
  const key = sameQuestion(a);
  return key === null ? [] : list.filter((x) => x.id !== a.id && sameQuestion(x) === key);
}
