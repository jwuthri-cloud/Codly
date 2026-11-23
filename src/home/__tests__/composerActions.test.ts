// What `/` lists in each kind of conversation: which names run, which are off and why, and that
// every name is in every list (a known name is never sent to the agent as a message).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkAgent } from '../../office/demo/script';
import { api } from '../../lib/api';
import type { Agent } from '../../lib/gen/Agent';
import type { Checkpoint } from '../../lib/gen/Checkpoint';
import type { LeadMode } from '../../lib/gen/LeadMode';
import { showContext } from '../../lib/state/control.svelte';
import type { McpServer } from '../../lib/state/session';
import { toasts } from '../../lib/state/toasts.svelte';
import { noTurn, slashCommands, type SlashContext, type SlashDo } from '../composerActions';
import { filterSlash } from '../composerMenu';

/** The modes of a request, as LeadControls lists them. */
const modes: SlashContext['modes'] = (['ask', 'plan', 'auto', 'direct'] as LeadMode[]).map((id) => ({ id, label: id[0].toUpperCase() + id.slice(1), hint: `${id} hint`, icon: 'chat' }));

/** Every action, recording what was asked for. */
function recorder() {
  const calls: string[] = [];
  const say = (what: string) => () => void calls.push(what);
  const of = (what: string) => (a: Agent) => void calls.push(`${what} ${a.id}`);
  const on: SlashDo = {
    newRequest: say('newRequest'), fromIssue: say('fromIssue'), fromPullRequest: say('fromPullRequest'), stop: say('stop'),
    requestMode: (mode) => void calls.push(`requestMode ${mode}`),
    agentMode: (a, mode) => void calls.push(`agentMode ${a.id} ${mode}`),
    leadModels: say('leadModels'), agentModel: of('agentModel'), compact: of('compact'), context: of('context'), changes: say('changes'),
    undo: of('undo'), terminal: of('terminal'), showTerminal: of('showTerminal'), mcp: of('mcp'),
    clear: say('clear'), closed: say('closed'), settings: say('settings'), shortcuts: say('shortcuts'),
  };
  return { on, calls };
}

/** An agent with a live process, between turns. */
const running = (id: string, provider: Agent['provider'], role: Agent['role']): Agent => ({ ...mkAgent(id, id[0].toUpperCase() + id.slice(1), provider, role, 1, 'idle'), run_id: `run-${id}` });
const server = (name: string, status: McpServer['status']): McpServer => ({ name, status, error: null, runId: 'run-ada', ts: 1 });

/** A conversation where the core has answered: the agent may go to your terminal, and a turn of it is kept. */
const at = (over: Partial<SlashContext>): SlashContext => ({ oneToOne: false, agent: null, busy: false, changes: false, modes, mcp: [], takeover: { takeover: null, blocked: null }, turns: { off: null }, ...over });

const list = (c: SlashContext, on = recorder().on) => slashCommands(c, on);
const names = (c: SlashContext) => list(c).map((x) => x.name);
/** name → the reason it is off; a name that runs is not in it. */
const off = (c: SlashContext): Record<string, string> => Object.fromEntries(list(c).flatMap((x) => ('off' in x ? [[x.name, x.off]] : [])));
const runs = (c: SlashContext) => list(c).filter((x) => 'run' in x).map((x) => x.name);
/** Type the name and press ↵. */
function pick(c: SlashContext, name: string): string[] {
  const { on, calls } = recorder();
  const row = slashCommands(c, on).find((x) => x.name === name);
  if (row && 'run' in row) row.run();
  return calls;
}

const ALL = ['new', 'issue', 'pr', 'stop', 'ask', 'plan', 'auto', 'direct', 'edits', 'model', 'compact', 'context', 'diff', 'undo', 'terminal', 'mcp', 'clear', 'closed', 'settings', 'shortcuts'];

const lead = running('lea', 'claude', 'lead');
const ada = running('ada', 'claude', 'solo');
const cody = running('cody', 'codex', 'solo');

const request = at({ agent: lead, takeover: { takeover: null, blocked: 'the Lead works through Codly\'s own tools, and a terminal does not have them' }, turns: { off: 'Lea has no worktree of its own, so there are no checkpoints.' } });
const claudeChat = at({ oneToOne: true, agent: ada, mcp: [server('github', 'connected'), server('linear', 'needs_sign_in')] });
const codexChat = at({ oneToOne: true, agent: cody });
const newRequest = at({});
const goneChat = at({ oneToOne: true });

describe('the actions of /', () => {
  it('every name is in every conversation, once', () => {
    for (const c of [request, claudeChat, codexChat, newRequest, goneChat, at({ agent: cody, busy: true, changes: true })]) {
      expect([...names(c)].sort()).toEqual([...ALL].sort());
    }
  });

  it('the letters you typed before still pick the same action: the agent’s controls come after it', () => {
    /** The first rows after `/` and these letters: the names that start with them, in the list's order. */
    const typed = (query: string, rows: number) => filterSlash(list(claudeChat), query).slice(0, rows).map((x) => x.name);
    expect(typed('c', 4)).toEqual(['clear', 'closed', 'compact', 'context']);
    expect(typed('m', 2)).toEqual(['model', 'mcp']);
    expect(typed('co', 2)).toEqual(['compact', 'context']);
    expect(typed('s', 2)).toEqual(['settings', 'shortcuts']); // (stop is off: nothing runs)
  });

  it('a request: its modes and its Lead; what is only a 1:1 agent’s says so', () => {
    expect(runs(request)).toEqual(['new', 'issue', 'pr', 'ask', 'plan', 'auto', 'direct', 'model', 'clear', 'closed', 'settings', 'shortcuts', 'context']);
    expect(off(request)).toEqual({
      stop: 'Nothing is running',
      edits: 'Only in a 1:1 chat',
      compact: 'Only a Codex agent or a 1:1 Claude agent',
      diff: 'This request has no changes yet',
      undo: 'Lea has no worktree of its own, so there are no checkpoints.',
      terminal: 'The Lead works through Codly\'s own tools, and a terminal does not have them',
      mcp: 'Lea’s run reports no MCP server',
    });
    // `ask` and `plan` are the request's modes here.
    expect(pick(request, 'plan')).toEqual(['requestMode plan']);
    expect(pick(request, 'context')).toEqual(['context lea']);
    expect(pick(request, 'model')).toEqual(['leadModels']);
    // A Codex Lead between turns can compact; a busy request can stop, and one with changes shows them.
    const codexLead = at({ agent: running('lex', 'codex', 'lead'), busy: true, changes: true });
    expect(pick(codexLead, 'compact')).toEqual(['compact lex']);
    expect(off(codexLead).context).toBe('Only Claude Code can be asked for it');
    expect([pick(codexLead, 'stop'), pick(codexLead, 'diff')]).toEqual([['stop'], ['changes']]);
  });

  it('a 1:1 chat with a Claude agent: ask, plan and edits set its way of working, and its controls run', () => {
    expect(off(claudeChat)).toEqual({ stop: 'Nothing is running', auto: 'Only in a request', direct: 'Only in a request', diff: 'Only in a request' });
    expect([pick(claudeChat, 'ask'), pick(claudeChat, 'plan'), pick(claudeChat, 'edits')]).toEqual([['agentMode ada ask'], ['agentMode ada plan'], ['agentMode ada accept_edits']]);
    expect(list(claudeChat).filter((x) => ['ask', 'plan', 'edits'].includes(x.name)).map((x) => x.label)).toEqual(['Ask for edits', 'Plan first', 'Accept edits']);
    expect(['compact', 'context', 'undo', 'terminal', 'mcp', 'model'].map((n) => pick(claudeChat, n))).toEqual([['compact ada'], ['context ada'], ['undo ada'], ['terminal ada'], ['mcp ada'], ['agentModel ada']]);
    expect(list(claudeChat).find((x) => x.name === 'compact')).toMatchObject({ label: 'Compact Ada’s context', hint: expect.stringContaining('Claude replaces the conversation so far with a summary') });
    expect(list(claudeChat).find((x) => x.name === 'mcp')).toMatchObject({ hint: '1 connected · 1 needs sign-in' });
  });

  it('a 1:1 chat with a Codex agent: it compacts; a mode, a context reading and MCP servers are Claude Code’s', () => {
    expect(off(codexChat)).toEqual({
      stop: 'Nothing is running',
      ask: 'Only a 1:1 Claude agent has a mode to switch',
      plan: 'Only a 1:1 Claude agent has a mode to switch',
      edits: 'Only a 1:1 Claude agent has a mode to switch',
      auto: 'Only in a request',
      direct: 'Only in a request',
      context: 'Only Claude Code can be asked for it',
      diff: 'Only in a request',
      mcp: 'Cody’s run reports no MCP server',
    });
    expect(['compact', 'undo', 'terminal'].map((n) => pick(codexChat, n))).toEqual([['compact cody'], ['undo cody'], ['terminal cody']]);
    expect(list(codexChat).find((x) => x.name === 'compact')).toMatchObject({ hint: expect.stringContaining('Codex sums up the conversation so far') });
  });

  it('a conversation with no agent: nothing of an agent runs, and each says there is none', () => {
    const none = 'This request has no Lead yet';
    expect(off(newRequest)).toEqual({ stop: 'Nothing is running', edits: 'Only in a 1:1 chat', compact: none, context: none, diff: 'This request has no changes yet', undo: none, terminal: none, mcp: none });
    // A request has its modes before its Lead starts.
    expect(pick(newRequest, 'auto')).toEqual(['requestMode auto']);
    const gone = 'This agent is gone';
    expect(off(goneChat)).toMatchObject({ ask: gone, plan: gone, edits: gone, model: 'Not for this agent', compact: gone, context: gone, undo: gone, terminal: gone, mcp: gone });
    expect(runs(goneChat)).toEqual(['new', 'issue', 'pr', 'clear', 'closed', 'settings', 'shortcuts']);
  });

  it('an agent’s controls wait for what the sheet waits for', () => {
    const chat = (agent: Agent, over: Partial<SlashContext> = {}) => off(at({ oneToOne: true, agent, ...over }));
    // Compact: between turns, off a task, with a live process.
    expect(chat({ ...ada, status: 'working' }).compact).toBe('Wait until its turn ends.');
    expect(chat({ ...cody, task_id: 't1' }).compact).toBe('It is on a task. Compact it between tasks.');
    expect(chat({ ...ada, run_id: null })).toMatchObject({ compact: 'Ada isn’t running', context: 'Ada isn’t running', mcp: 'Ada isn’t running' });
    expect(chat({ ...ada, status: 'offline' }).context).toBe('Ada isn’t running');
    // A mode: not while it is on a task, and only for a 1:1 agent.
    expect(chat({ ...ada, task_id: 't1' }).plan).toBe('Ada is on a task: its mode can change when the task is done');
    expect(chat(running('wes', 'claude', 'worker'))).toMatchObject({ edits: 'Only a 1:1 Claude agent has a mode to switch', model: 'Not for this agent', compact: 'Only a Codex agent or a 1:1 Claude agent' });
    // Undo: the core's word on what it kept.
    expect(chat(ada, { turns: { off: 'Codly kept no turn of Ada yet' } }).undo).toBe('Codly kept no turn of Ada yet');
  });

  it('your terminal: the core says whether the agent can go, and an agent that is there is shown', () => {
    const chat = (takeover: SlashContext['takeover']) => at({ oneToOne: true, agent: ada, takeover });
    expect(off(chat(null)).terminal).toBe('Asking Codly if it can go…');
    expect(off(chat({ takeover: null, blocked: 'Ada is in the middle of a turn: wait for it, or stop it first' })).terminal).toBe('Ada is in the middle of a turn: wait for it, or stop it first');
    const there = chat({ takeover: { agent_id: 'ada', cwd: '/w', command: 'claude --resume s1', copy_command: 'cd /w && claude --resume s1', since: 1 }, blocked: null });
    expect(list(there).find((x) => x.name === 'terminal')).toMatchObject({ hint: 'Ada is in your terminal: show it' });
    expect(pick(there, 'terminal')).toEqual(['showTerminal ada']);
  });
});

describe('what only the core knows, right after / opened the menu', () => {
  /** A 1:1 chat with Ada: the core was asked, and has not answered yet. */
  const opened = at({ oneToOne: true, agent: ada, takeover: null, turns: null });
  const typed = (c: SlashContext, query: string) => filterSlash(list(c), query);

  it('the action waits in its place: ↵ on its name runs nothing, and no action that holds the same letters', () => {
    // "New request" holds a u, and "Compact Ada’s context" holds "te": they run, and they are not first.
    expect(typed(opened, 'u')[0]).toEqual({ name: 'undo', label: 'Undo Ada’s last turn…', icon: 'arrow-left', off: 'Reading what Codly kept of its turns…', wait: true });
    expect(typed(opened, 'u')[1]).toMatchObject({ name: 'new', run: expect.any(Function) });
    expect(typed(opened, 'te')[0]).toEqual({ name: 'terminal', label: 'Continue in your terminal…', icon: 'terminal', off: 'Asking Codly if it can go…', wait: true });
    expect(typed(opened, 'te')[1]).toMatchObject({ name: 'compact', run: expect.any(Function) });
    expect(pick(opened, 'undo')).toEqual([]);
    expect(pick(opened, 'terminal')).toEqual([]);
  });

  it('the core’s answer: yes runs in the same place, and no goes last as any action that cannot run', () => {
    const yes = at({ oneToOne: true, agent: ada });
    expect(typed(yes, 'u')[0]).toMatchObject({ name: 'undo', run: expect.any(Function) });
    expect(typed(yes, 'te')[0]).toMatchObject({ name: 'terminal', run: expect.any(Function) });
    const no = typed(at({ oneToOne: true, agent: ada, turns: { off: 'Codly kept no turn of Ada yet' } }), 'u');
    expect(no[0]).toMatchObject({ name: 'new' });
    expect(no.find((x) => x.name === 'undo')).toEqual({ name: 'undo', label: 'Undo Ada’s last turn…', icon: 'arrow-left', off: 'Codly kept no turn of Ada yet' });
    expect(no.slice(no.findIndex((x) => x.name === 'undo')).every((x) => 'off' in x)).toBe(true);
  });
});

describe('what /undo can open', () => {
  const point = { n: 1 } as Checkpoint;

  it('a kept turn, or the reason there is none', () => {
    expect(noTurn({ off: null, checkpoints: [point] }, 'Ada')).toBeNull();
    expect(noTurn({ off: null, checkpoints: [] }, 'Ada')).toBe('Codly kept no turn of Ada yet');
    expect(noTurn({ off: 'Ada has no worktree now, so there are no checkpoints.', checkpoints: [] }, 'Ada')).toBe('Ada has no worktree now, so there are no checkpoints.');
  });
});

describe('/context', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of [...toasts.list]) toasts.dismiss(t.id);
  });

  it('says how full the agent’s context is in a toast', async () => {
    vi.spyOn(api, 'agentContextUsage').mockResolvedValue({ used_tokens: 6035, max_tokens: 1_000_000, percent: 1, auto_compact_at: 967_000, parts: [] });
    await showContext('ada', 'Ada');
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['info', 'Ada’s context', '6k of 1M tokens (1%) · compacts by itself at 967k']]);
  });

  it('a reading that fails says why, once', async () => {
    vi.spyOn(api, 'agentContextUsage').mockRejectedValue('Ada is not running');
    await showContext('ada', 'Ada');
    expect(toasts.list.map((t) => [t.kind, t.title, t.body])).toEqual([['error', 'Couldn’t read its context', 'Ada is not running']]);
  });
});
