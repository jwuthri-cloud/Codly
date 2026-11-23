import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { AgentControl } from '../demo/controls';
import type { Agent } from '../gen/Agent';
import { PERMISSION_MODES, canCompact, changedPlan, compactAgent, compactHint, compactWait, contextLine, readContext, setPermissionMode } from '../state/control.svelte';
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

  it('says what each mode takes out of your permissions, and what it does not', () => {
    const hint = (id: string) => PERMISSION_MODES.find((m) => m.id === id)?.hint ?? '';
    // Recorded (Claude Code 2.1.311): in "accept edits" a shell command that writes, moves or
    // deletes a file in the worktree runs with no request, so Codly's permissions never see
    // it. An edit of the folder's own .claude settings still comes as a request.
    expect(hint('accept_edits')).toContain('shell commands that change, move or delete files in its worktree');
    expect(hint('accept_edits')).toContain('your permissions do not see them');
    expect(hint('accept_edits')).toContain('A change to its own Claude Code settings there (.claude) still asks you');
    expect(hint('accept_edits')).toContain('Not available with “Ask first” permissions');
    expect(hint('accept_edits')).not.toContain('Commands still follow your permissions');
    // "Plan first" promises what the core enforces: only reads are approved before the plan is.
    expect(hint('plan')).toContain('Codly approves only reads for it, whatever your permissions');
    expect(hint('plan')).not.toContain('changes nothing');
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
    expect(await compactAgent('bo', 'Bo')).toBe(false);
    expect(await readContext('bo')).toBeNull();
    expect(toasts.list.map((t) => [t.kind, t.body])).toEqual([
      ['error', 'Bo is on a task: its mode can change when the task is done'],
      ['error', 'Bo is still working: compact it when its turn ends'],
      ['error', 'Bo is not running'],
    ]);
    for (const t of toasts.list) toasts.dismiss(t.id);
  });

  // Before, a Claude agent had its own "Compact now" button in the Context row and a Codex
  // agent a row of its own, with two functions that called the same command.
  it('has one "Compact now" for a Codex agent and a 1:1 Claude agent, with one way to ask', async () => {
    const who = (provider: Agent['provider'], role: Agent['role']) => canCompact({ provider, role });
    expect([who('codex', 'solo'), who('codex', 'worker'), who('claude', 'solo')]).toEqual([true, true, true]);
    // The core refuses a Claude worker, reviewer or Lead (the end of a turn starts its check), and the other CLIs.
    expect([who('claude', 'worker'), who('claude', 'lead'), who('gemini', 'solo')]).toEqual([false, false, false]);
    expect(compactHint('codex')).toContain('Codex sums up the conversation so far');
    expect(compactHint('claude')).toContain('Claude replaces the conversation so far with a summary');

    // The button waits for the same things for both: a task, a turn.
    expect(compactWait({ task_id: 't1', status: 'idle' })).toBe('It is on a task. Compact it between tasks.');
    expect(compactWait({ task_id: null, status: 'working' })).toBe('Wait until its turn ends.');
    expect(compactWait({ task_id: null, status: 'idle' })).toBeNull();

    for (const t of toasts.list) toasts.dismiss(t.id);
    const core = vi.spyOn(api, 'compactAgent').mockResolvedValue(undefined);
    expect(await compactAgent('bo', 'Bo')).toBe(true);
    expect(core).toHaveBeenCalledWith('bo');
    expect(toasts.list.map((t) => [t.kind, t.title])).toEqual([['success', 'Compacting Bo’s conversation']]);
    for (const t of toasts.list) toasts.dismiss(t.id);
  });

  // Recorded: Claude Code takes a changed plan in the answer as the plan. Before, the card
  // could only accept the plan as it was or send it back.
  it('sends a changed plan with the accept, and nothing for a plan left as it was', () => {
    const plan = '1. Add the line `world`.\n';
    expect(changedPlan(plan, '1. Add the line `planet`.\n')).toEqual({ plan: '1. Add the line `planet`.\n' });
    expect(changedPlan(plan, plan)).toBeNull();
    expect(changedPlan(plan, '  \n')).toBeNull();
  });

  it('sends the mode the user picked and reports the switch', async () => {
    const core = vi.spyOn(api, 'setAgentPermissionMode').mockResolvedValue({ id: 'bo', permission_mode: 'plan' } as Agent);
    expect(await setPermissionMode('bo', 'plan')).toBe(true);
    expect(core).toHaveBeenCalledWith('bo', 'plan');
  });
});
