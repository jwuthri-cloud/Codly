import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { AgentControl } from '../demo/controls';
import type { Agent } from '../gen/Agent';
import { PERMISSION_MODES, compactNow, contextLine, readContext, setPermissionMode } from '../state/control.svelte';
import { toasts } from '../state/toasts.svelte';

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

// A Claude agent's mode, "compact now" and context reading (requests to its live session).
describe('Claude session controls', () => {
  afterEach(() => vi.restoreAllMocks());

  it('offers the three modes the core knows, and never one that decides in Codly\'s place', () => {
    expect(PERMISSION_MODES.map((m) => m.id)).toEqual(['ask', 'accept_edits', 'plan']);
  });

  it('says how full the context is, with the point where Claude compacts by itself', () => {
    const usage = { used_tokens: 6035, max_tokens: 1_000_000, percent: 1, auto_compact_at: 967_000, parts: [] };
    expect(contextLine(usage)).toBe('6k of 1M tokens (1%) · compacts by itself at 967k');
    expect(contextLine({ ...usage, auto_compact_at: null })).toBe('6k of 1M tokens (1%)');
  });

  it('a refused switch, compaction or reading says why and reports that it did not happen', async () => {
    for (const t of toasts.list) toasts.dismiss(t.id);
    vi.spyOn(api, 'setAgentPermissionMode').mockRejectedValue('Bo is on a task: its mode can change when the task is done');
    vi.spyOn(api, 'compactAgent').mockRejectedValue('Bo is still working: compact it when its turn ends');
    vi.spyOn(api, 'agentContextUsage').mockRejectedValue('Bo is not running');
    expect(await setPermissionMode('bo', 'plan')).toBe(false);
    expect(await compactNow('bo')).toBe(false);
    expect(await readContext('bo')).toBeNull();
    expect(toasts.list.map((t) => [t.kind, t.body])).toEqual([
      ['error', 'Bo is on a task: its mode can change when the task is done'],
      ['error', 'Bo is still working: compact it when its turn ends'],
      ['error', 'Bo is not running'],
    ]);
    for (const t of toasts.list) toasts.dismiss(t.id);
  });

  it('sends the mode the user picked and reports the switch', async () => {
    const core = vi.spyOn(api, 'setAgentPermissionMode').mockResolvedValue({ id: 'bo', permission_mode: 'plan' } as Agent);
    expect(await setPermissionMode('bo', 'plan')).toBe(true);
    expect(core).toHaveBeenCalledWith('bo', 'plan');
  });
});
