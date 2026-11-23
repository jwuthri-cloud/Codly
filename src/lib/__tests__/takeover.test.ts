import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { blocked, DemoTakeovers, resumeCommand } from '../demo/takeover';
import type { Agent } from '../gen/Agent';
import type { Task } from '../gen/Task';
import type { TakeoverState } from '../gen/TakeoverState';
import { giveBack, loadTakeover, loadTakeovers, openInTerminal, takeovers } from '../state/takeover.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';

const SESSION = '0b9f1c5e-7a2d-4c1b-9e61-3f2a8d4c5b6a';
const REPO = '/demo/acme-web';
const NOW = 1_760_000_000_000;
const agent = (o: Partial<Agent> = {}) =>
  ({ id: 'a1', name: 'Ada', provider: 'claude', role: 'solo', status: 'idle', session_id: SESSION, worktree: '/demo/wt/ada', task_id: null, archived: false, ...o }) as Agent;
const task = (o: Partial<Task> = {}) => ({ id: 't1', key: 'T-1', status: 'running', assignee: 'a1', ...o }) as Task;

// The demo follows the rules of orchestrator/takeover.rs; these are the same cases.
describe('the demo’s side of "Open in my terminal"', () => {
  it('gives Claude Code its resume command, and no command to a CLI whose sessions do not go', () => {
    expect(resumeCommand(agent())).toEqual({ command: `claude --resume ${SESSION}` });
    // Codex has a resume command, but its sessions stay with Codly for now.
    expect(resumeCommand(agent({ provider: 'codex' }))).toMatchObject({ why: expect.stringMatching(/^a Codex agent cannot go to your terminal yet/) });
    for (const provider of ['gemini', 'opencode', 'cursor', 'copilot'] as const) {
      expect(resumeCommand(agent({ provider }))).toMatchObject({ why: expect.stringContaining('command that opens one session by its id') });
    }
    expect(resumeCommand(agent({ session_id: null }))).toEqual({ why: 'Ada has no session yet: send it a message first' });
    // A session id that could be read as an option never reaches a command line.
    for (const session_id of ['--dangerously-skip-permissions', 'a b', 'a;b']) {
      expect(resumeCommand(agent({ session_id }))).toMatchObject({ why: expect.stringContaining('not a plain word') });
    }
  });

  it('says why an agent cannot go', () => {
    expect(blocked(agent(), [])).toBeNull();
    expect(blocked(agent({ status: 'working' }), [])).toBe('Ada is in the middle of a turn: wait for it, or stop it first');
    expect(blocked(agent({ role: 'lead' }), [])).toContain('the Lead works through Codly’s own tools');
    expect(blocked(agent({ archived: true }), [])).toBe('Ada no longer works here');
    // A task that still runs is Codly's to run; one that waits for you is not.
    expect(blocked(agent({ task_id: 't1' }), [task()])).toBe('Ada is on T-1, which is still running: stop Ada first, or wait for the task');
    expect(blocked(agent(), [task({ status: 'verifying' })])).toContain('which is still running');
    expect(blocked(agent({ task_id: 't1' }), [task({ status: 'blocked' })])).toBeNull();
    expect(blocked(agent(), [task({ assignee: 'someone-else' })])).toBeNull();
  });

  it('takes an agent over until it is given back', () => {
    const demo = new DemoTakeovers();
    expect(demo.state(agent(), [], REPO)).toEqual({ takeover: null, blocked: null });
    expect(() => demo.take(agent({ status: 'thinking' }), [], REPO, NOW)).toThrow('in the middle of a turn');
    expect(demo.has('a1')).toBe(false);

    const command = `claude --resume ${SESSION}`;
    const there = { takeover: { agent_id: 'a1', cwd: '/demo/wt/ada', command, copy_command: `cd /demo/wt/ada && ${command}`, since: NOW }, blocked: null };
    expect(demo.take(agent(), [], REPO, NOW)).toEqual(there);
    expect(demo.has('a1')).toBe(true);
    // Asked again, the same hand-over.
    expect(demo.take(agent(), [], REPO, NOW + 5)).toEqual(there);
    // The list has it, and not an agent that stayed.
    expect(demo.list([agent({ id: 'a0' }), agent()], [], REPO)).toEqual([there.takeover]);
    expect(demo.giveBack(agent(), [], REPO)).toEqual({ takeover: null, blocked: null });
    expect(demo.has('a1')).toBe(false);
    expect(demo.list([agent()], [], REPO)).toEqual([]);

    // Without a worktree the terminal opens in the repository; a path with a space is quoted.
    expect(demo.take(agent({ id: 'a2', worktree: null }), [], '/demo/my repo', NOW).takeover).toMatchObject({ cwd: '/demo/my repo', copy_command: `cd '/demo/my repo' && ${command}` });
    expect(() => demo.state(undefined, [], REPO)).toThrow('unknown agent');
  });
});

describe('Open in my terminal', () => {
  const command = `claude --resume ${SESSION}`;
  const there: TakeoverState = { takeover: { agent_id: 'a1', cwd: '/demo/wt/ada', command, copy_command: `cd /demo/wt/ada && ${command}`, since: NOW }, blocked: null };
  const free: TakeoverState = { takeover: null, blocked: null };
  const said = () => toasts.list.map((t) => [t.kind, t.title, t.body]);

  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of toasts.list) toasts.dismiss(t.id);
    takeovers.byAgent = {};
    ui.terminalTakeover = null;
    ui.go('home');
  });

  it('shows the agent’s own terminal tab once the core took the agent over', async () => {
    const core = vi.spyOn(api, 'takeOverAgent').mockResolvedValue(there);
    expect(await openInTerminal('a1')).toBe(true);
    expect(core).toHaveBeenCalledWith('a1');
    expect(takeovers.active).toEqual([there.takeover]);
    expect([ui.room, ui.terminalTakeover]).toEqual(['terminal', 'a1']);
    expect(said()).toEqual([]);
  });

  it('says the core’s reason and opens nothing when the agent may not go', async () => {
    const why = 'Ada is in the middle of a turn: wait for it, or stop it first';
    vi.spyOn(api, 'takeOverAgent').mockRejectedValue(new Error(why));
    const look = vi.spyOn(api, 'agentTakeover').mockResolvedValue({ takeover: null, blocked: why });
    expect(await openInTerminal('a1')).toBe(false);
    expect(said()).toEqual([['error', 'Couldn’t open it in your terminal', why]]);
    expect([ui.room, ui.terminalTakeover]).toEqual(['home', null]);
    // The sheet shows where the agent is after the failure, not what it showed before.
    expect(look).toHaveBeenCalledWith('a1');
    expect(takeovers.of('a1')).toEqual({ takeover: null, blocked: why });
  });

  it('does not leave an agent marked when its process did not stop', async () => {
    // The sheet asked while the core was stopping the agent's process.
    takeovers.set('a1', { takeover: null, blocked: 'Ada is stopping: one moment' });
    vi.spyOn(api, 'takeOverAgent').mockRejectedValue(new Error('Ada did not stop in time; try again'));
    vi.spyOn(api, 'agentTakeover').mockResolvedValue(free);
    expect(await openInTerminal('a1')).toBe(false);
    expect(takeovers.of('a1')).toEqual(free);
    expect(takeovers.has('a1')).toBe(false);
  });

  it('gives the agent back', async () => {
    takeovers.set('a1', there);
    const core = vi.spyOn(api, 'giveBackAgent').mockResolvedValue(free);
    expect(await giveBack('a1')).toBe(true);
    expect(core).toHaveBeenCalledWith('a1');
    expect(takeovers.active).toEqual([]);

    // A give-back that failed leaves the agent where it is, and says so.
    takeovers.set('a1', there);
    core.mockRejectedValue('the store is locked');
    vi.spyOn(api, 'agentTakeover').mockResolvedValue(there);
    expect(await giveBack('a1')).toBe(false);
    expect(takeovers.active).toEqual([there.takeover]);
    expect(said()).toEqual([['error', 'Couldn’t give it back', 'the store is locked']]);
  });

  it('keeps what the core said last, and shows a failed look as the reason', async () => {
    const core = vi.spyOn(api, 'agentTakeover').mockResolvedValue({ takeover: null, blocked: 'Ada has no session yet: send it a message first' });
    await loadTakeover('a1');
    expect(takeovers.of('a1')?.blocked).toBe('Ada has no session yet: send it a message first');
    core.mockResolvedValue(there);
    await loadTakeover('a1');
    expect(takeovers.of('a1')).toEqual(there);
    core.mockRejectedValue(new Error('unknown agent a1'));
    await loadTakeover('a1');
    expect(takeovers.of('a1')).toEqual({ takeover: null, blocked: 'unknown agent a1' });
    expect(said()).toEqual([]);
  });

  it('marks every agent that is in your terminal when an office opens', async () => {
    const other = { ...there.takeover!, agent_id: 'a2' };
    // What this window knew: a1 is there, a3 cannot go.
    takeovers.set('a1', there);
    takeovers.set('a3', { takeover: null, blocked: 'Cy has no session yet: send it a message first' });
    const list = vi.spyOn(api, 'listTakeovers').mockResolvedValue([other]);
    // a1 is not on the core's list: it was given back elsewhere, and the core is asked about it.
    const look = vi.spyOn(api, 'agentTakeover').mockResolvedValue(free);
    await loadTakeovers();
    expect(takeovers.active).toEqual([other]);
    expect([takeovers.has('a1'), takeovers.has('a2'), takeovers.has('a3')]).toEqual([false, true, false]);
    expect(look.mock.calls).toEqual([['a1']]);
    expect(takeovers.of('a1')).toEqual(free);
    expect(takeovers.of('a3')?.blocked).toBe('Cy has no session yet: send it a message first');

    // A list that cannot be read is said, and what the window knew stays.
    list.mockRejectedValue(new Error('the store is locked'));
    await loadTakeovers();
    expect(takeovers.active).toEqual([other]);
    expect(said()).toEqual([['error', 'Couldn’t read which agents are in your terminal', 'the store is locked']]);
  });
});
