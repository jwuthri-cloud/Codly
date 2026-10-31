import { describe, expect, it } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { EventKind } from '../gen/EventKind';
import type { Task } from '../gen/Task';
import { transferBlock, transferredOut } from '../util/transfer';

const agent = (patch: Partial<Agent> = {}) => ({ id: 'dee', name: 'Dee', role: 'worker', status: 'idle', task_id: null, member: null, ...patch }) as Agent;
const task = (patch: Partial<Task>) => ({ id: 't1', key: 'COD-3', assignee: 'dee', status: 'ready', ...patch }) as Task;
const moved = (detail: Record<string, string>): EventKind => ({ type: 'decision', id: 'd1', decision_type: 'transfer', method: 'rule', summary: 'Dee moved', detail });

describe('transfers between offices', () => {
  it('an idle agent with nothing on the board can go', () => {
    expect(transferBlock(agent(), [task({ status: 'done' }), task({ id: 't2', assignee: 'bo' })])).toBeNull();
  });

  it('never the Lead or a team member, never mid-task or mid-turn', () => {
    expect(transferBlock(agent({ role: 'lead' }), [])).toMatch(/Lead/);
    expect(transferBlock(agent({ member: 'm1' }), [])).toMatch(/Team members/);
    expect(transferBlock(agent({ task_id: 't1' }), [task({ status: 'running' })])).toBe("On COD-3: it can't transfer mid-task");
    expect(transferBlock(agent(), [task({ status: 'blocked' })])).toMatch(/COD-3/);
    expect(transferBlock(agent({ task_id: 'gone' }), [])).toMatch(/On a task/);
    expect(transferBlock(agent({ status: 'working' }), [])).toMatch(/middle of a turn/);
  });

  it('the office it left drops it; the one it joins and other decisions do not', () => {
    expect(transferredOut(moved({ agent: 'dee', from: 'ws-a', to: 'ws-b' }), 'ws-a')).toBe('dee');
    expect(transferredOut(moved({ agent: 'dee', from: 'ws-a', to: 'ws-b' }), 'ws-b')).toBeNull();
    expect(transferredOut(moved({ agent: 'dee', from: 'ws-a', to: 'ws-b' }), null)).toBeNull();
    expect(transferredOut({ ...moved({ agent: 'dee', from: 'ws-a', to: 'ws-b' }), decision_type: 'staff' } as EventKind, 'ws-a')).toBeNull();
  });
});
