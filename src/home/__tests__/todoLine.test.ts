// The agent's own todo list in one line: done, total and the step it is on.
import { describe, expect, it } from 'vitest';
import type { PlanStep } from '../../lib/gen/PlanStep';
import { todoLine, todoWords } from '../todoLine';

const step = (text: string, status: string): PlanStep => ({ text, status });

describe('todoLine', () => {
  it('an empty list gives nothing', () => {
    expect(todoLine([])).toBeNull();
  });

  it('shows the first step in progress, also after a pending one', () => {
    const line = todoLine([step('Read the code', 'completed'), step('Write the test', 'pending'), step('Add the toggle', 'in_progress')]);
    expect(line).toEqual({ done: 1, total: 3, step: step('Add the toggle', 'in_progress'), at: 2 });
    expect(todoWords(line!)).toBe('1 of 3 · Add the toggle');
  });

  it('with none in progress, shows the first pending step', () => {
    const line = todoLine([step('Read the code', 'completed'), step('Write the test', 'pending'), step('Ship', 'pending')]);
    expect(line?.step?.text).toBe('Write the test');
    expect(line?.at).toBe(1);
  });

  it('all done: the count and no step', () => {
    const line = todoLine([step('a', 'completed'), step('b', 'completed')]);
    expect(line).toEqual({ done: 2, total: 2, step: null, at: null });
    expect(todoWords(line!)).toBe('2 of 2');
  });
});
