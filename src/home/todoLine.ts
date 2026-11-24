// An agent's own todo list (from its CLI: Claude's todo tools, Codex's plan) in one line: how
// many steps are done, and the step it is on. Pure.
import type { PlanStep } from '../lib/gen/PlanStep';
import type { Tone } from '../lib/narrate';

export type TodoLine = {
  done: number;
  total: number;
  /** The step to show: the first in progress, else the first pending one; null when all are done. */
  step: PlanStep | null;
  /** Where that step is in the list (null with no step). */
  at: number | null;
};

/** The line of a todo list; null for an empty list. */
export function todoLine(steps: readonly PlanStep[]): TodoLine | null {
  if (!steps.length) return null;
  let at = steps.findIndex((s) => s.status === 'in_progress');
  if (at < 0) at = steps.findIndex((s) => s.status === 'pending');
  return {
    done: steps.filter((s) => s.status === 'completed').length,
    total: steps.length,
    step: at < 0 ? null : steps[at],
    at: at < 0 ? null : at,
  };
}

/** The step it is on and where that is in the list: "Step 2 of 7 · Add the toggle to the header"; "7 of 7 done" when all are. */
export function todoWords(line: TodoLine): string {
  return line.step && line.at != null ? `Step ${line.at + 1} of ${line.total} · ${line.step.text}` : `${line.done} of ${line.total} done`;
}

/**
 * The words of an agent's list while it works (its narration's tone is 'live'): "Step 3 of 7 · …".
 * Null otherwise, also with a list: what the narration says then (in your terminal, out of
 * quota, stopped, waiting for you) is the news, not the list.
 */
export function workingTodo(tone: Tone | undefined, steps: readonly PlanStep[]): string | null {
  if (tone !== 'live') return null;
  const line = todoLine(steps);
  return line ? todoWords(line) : null;
}

/**
 * The line of an agent's list where the agent itself is shown (its 1:1 header, the row above
 * the composer): while it works, and also while it waits for you (`waiting`: the line is greyed,
 * the step it waits in is still the step it is on). Null at any other time, or with no list.
 */
export function todoNow(tone: Tone | undefined, steps: readonly PlanStep[]): { text: string; waiting: boolean } | null {
  if (tone !== 'live' && tone !== 'wait') return null;
  const line = todoLine(steps);
  return line ? { text: todoWords(line), waiting: tone === 'wait' } : null;
}
