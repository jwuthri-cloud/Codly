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

/** "3 of 7", and the step it is on: "3 of 7 · Add the toggle to the header". */
export function todoWords(line: TodoLine): string {
  const count = `${line.done} of ${line.total}`;
  return line.step ? `${count} · ${line.step.text}` : count;
}

/**
 * The words of an agent's list while it works (its narration's tone is 'live'): "3 of 7 · …".
 * Null otherwise, also with a list: what the narration says then (in your terminal, out of
 * quota, stopped, waiting for you) is the news, not the list.
 */
export function workingTodo(tone: Tone | undefined, steps: readonly PlanStep[]): string | null {
  if (tone !== 'live') return null;
  const line = todoLine(steps);
  return line ? todoWords(line) : null;
}
