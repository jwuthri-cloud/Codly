// The agent's own todo list in one line: done, total and the step it is on.
import { describe, expect, it } from 'vitest';
import type { PlanStep } from '../../lib/gen/PlanStep';
import { todoLine, todoNow, todoWords, workingTodo } from '../todoLine';

const step = (text: string, status: string): PlanStep => ({ text, status });

describe('todoLine', () => {
  it('an empty list gives nothing', () => {
    expect(todoLine([])).toBeNull();
  });

  it('shows the first step in progress, also after a pending one', () => {
    const line = todoLine([step('Read the code', 'completed'), step('Write the test', 'pending'), step('Add the toggle', 'in_progress')]);
    expect(line).toEqual({ done: 1, total: 3, step: step('Add the toggle', 'in_progress'), at: 2 });
    // The review's defect 18: "1 of 3" counted the done steps and named another one. It names the step it is on, and where that is.
    expect(todoWords(line!)).toBe('Step 3 of 3 · Add the toggle');
  });

  it('with none in progress, shows the first pending step', () => {
    const line = todoLine([step('Read the code', 'completed'), step('Write the test', 'pending'), step('Ship', 'pending')]);
    expect(line?.step?.text).toBe('Write the test');
    expect(line?.at).toBe(1);
    expect(todoWords(line!)).toBe('Step 2 of 3 · Write the test');
  });

  it('all done: the count and no step', () => {
    const line = todoLine([step('a', 'completed'), step('b', 'completed')]);
    expect(line).toEqual({ done: 2, total: 2, step: null, at: null });
    expect(todoWords(line!)).toBe('2 of 2 done');
  });
});

describe('workingTodo', () => {
  const list = [step('Read the code', 'completed'), step('Add the toggle', 'in_progress')];
  it('while the agent works: the step and where it is', () => {
    expect(workingTodo('live', list)).toBe('Step 2 of 2 · Add the toggle');
  });
  it('nothing while it does not work (in your terminal, out of quota, stopped, waiting for you), or with no list', () => {
    expect([workingTodo('idle', list), workingTodo('blocked', list), workingTodo('off', list), workingTodo('wait', list), workingTodo(undefined, list), workingTodo('live', [])]).toEqual([null, null, null, null, null, null]);
  });
});

describe('todoNow', () => {
  const list = [step('Read the code', 'completed'), step('Add the note to README.md', 'in_progress'), step('Test', 'pending'), step('Ship', 'pending')];
  it('stays while the agent waits for you, greyed, on the step it waits in', () => {
    expect(todoNow('live', list)).toEqual({ text: 'Step 2 of 4 · Add the note to README.md', waiting: false });
    expect(todoNow('wait', list)).toEqual({ text: 'Step 2 of 4 · Add the note to README.md', waiting: true });
  });
  it('nothing at other times, or with no list', () => {
    expect([todoNow('idle', list), todoNow('blocked', list), todoNow('off', list), todoNow('done', list), todoNow(undefined, list), todoNow('wait', [])]).toEqual([null, null, null, null, null, null]);
  });
});
