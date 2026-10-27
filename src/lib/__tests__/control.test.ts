import { describe, expect, it } from 'vitest';
import { AgentControl } from '../demo/controls';

// The demo's state machine mirrors orchestrator/controls.rs; these are the same cases.
describe('AgentControl', () => {
  it('a running agent pauses only when its turn ends, and holds notes meanwhile', () => {
    const c = new AgentControl();
    c.requestStop(true);
    expect(c.state()).toEqual({ block_tools: false, stop_requested: true, paused: false, held: 0 });
    expect(c.route('note')).toBeNull();
    c.turnCompleted();
    expect(c.state()).toMatchObject({ stop_requested: false, paused: true, held: 1 });
    expect(c.resume()).toEqual(['note']);
    expect(c.route('go')).toBe('go');
  });

  it('an idle agent pauses at once; resume cancels a pending stop', () => {
    const c = new AgentControl();
    c.requestStop(false);
    expect(c.state().paused).toBe(true);
    c.resume();
    c.requestStop(true);
    expect(c.resume()).toEqual([]);
    c.turnCompleted();
    expect(c.state().paused).toBe(false);
  });

  it('caps held notes, dropping the oldest', () => {
    const c = new AgentControl();
    c.requestStop(false);
    for (let i = 0; i < 35; i++) c.route(`n${i}`);
    const held = c.resume();
    expect(held).toHaveLength(32);
    expect(held[0]).toBe('n3');
  });
});
