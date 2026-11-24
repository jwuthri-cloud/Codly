import { describe, expect, it } from 'vitest';
import type { AgentDo, SlashContext } from '../../home/composerActions';
import type { Agent } from '../../lib/gen/Agent';
import { mkAgent } from '../../office/demo/script';
import {
  ROOM_WORDS, agentGroup, askFirst, bySection, escStep, guard, inCliOrder, noRow, okRow, press, rankRows, saidBy, saidWhere, snippet, wants, type SaidNames,
} from '../palette';

describe('palette sections', () => {
  const row = (id: string, section: string) => ({ id, section });

  it('shows each section once, in the order of its best row', () => {
    const ranked = [row('settings', 'App'), row('art1', 'Artifacts'), row('theme', 'App'), row('t1', 'Tasks'), row('art2', 'Artifacts')];
    expect(bySection(ranked).map((r) => r.id)).toEqual(['settings', 'theme', 'art1', 'art2', 't1']);
  });

  it('leaves rows that are already grouped as they are', () => {
    const grouped = [row('a', 'Go to'), row('b', 'Go to'), row('c', 'View')];
    expect(bySection(grouped)).toEqual(grouped);
    expect(bySection([])).toEqual([]);
  });
});

describe('palette confirmation', () => {
  const merge = { id: 'plan:merge:p1', confirm: 'Merge “Dark mode” into main?' };
  const go = { id: 'go:plan' };

  it('a row that cannot be taken back runs on the second Enter only', () => {
    const first = press(merge, null, false);
    expect(first).toEqual({ run: false, armed: 'plan:merge:p1' });
    expect(press(merge, first.armed, false)).toEqual({ run: true, armed: null });
  });

  it('a held Enter or a double-click never confirms: only a new press does', () => {
    const first = press(merge, null, false);
    expect(press(merge, first.armed, true)).toEqual({ run: false, armed: 'plan:merge:p1' });
    expect(press(merge, null, true)).toEqual({ run: false, armed: null });
    expect(press(merge, first.armed, false)).toEqual({ run: true, armed: null });
  });

  it('arming one row never confirms another', () => {
    const deny = { id: 'ap:no:a1', confirm: 'Deny?' };
    expect(press(deny, 'plan:merge:p1', false)).toEqual({ run: false, armed: 'ap:no:a1' });
  });

  it('an ordinary row runs at once and lets an armed row go', () => {
    expect(press(go, null, false)).toEqual({ run: true, armed: null });
    expect(press(go, 'plan:merge:p1', false)).toEqual({ run: true, armed: null });
  });

  it('says in full what an approval asks for', () => {
    const long = `rm -rf ${'very/long/path/'.repeat(8)}dist`;
    expect(wants({ kind: 'command', command: long, cwd: '/repo' }, 'claude')).toBe(`run ${long}\nin /repo`);
    expect(wants({ kind: 'command', command: 'ls', cwd: null }, 'codex')).toBe('run ls');
    expect(wants({ kind: 'permissions' as const, detail: { cwd: '/Users/someone/Code/acme', reason: null, permissions: { network: null, fileSystem: { read: null, write: ['/Users/someone/Code/acme-docs'] } } } }, 'codex')).toBe('write in ../acme-docs');
    expect(wants({ kind: 'file_change', paths: ['a.ts', 'b.ts'], diff: null }, 'claude')).toBe('change a.ts, b.ts');
    expect(wants({ kind: 'tool', name: 'WebFetch', input: null }, 'claude')).toBe('use the tool WebFetch');
    // A command in Claude Code's sandbox wants a host: the whole host, not the name of the request's tool.
    const host = `github.com.${'a'.repeat(96)}.evil.example`;
    expect(wants({ kind: 'tool', name: 'SandboxNetworkAccess', input: { host } }, 'claude')).toBe(`reach ${host}`);
    expect(wants({ kind: 'tool', name: 'SandboxNetworkAccess', input: {} }, 'claude')).toBe('use the network');
    // That request is Claude Code's own: from another CLI, or an agent Codly does not know, it is a tool by its name.
    expect(wants({ kind: 'tool', name: 'SandboxNetworkAccess', input: { host } }, 'codex')).toBe('use the tool SandboxNetworkAccess');
    expect(wants({ kind: 'tool', name: 'SandboxNetworkAccess', input: { host } }, undefined)).toBe('use the tool SandboxNetworkAccess');
    // The same for a plan: a Claude agent shows you its plan, any other agent uses a tool of that name.
    const plan = { kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add the line' } } as const;
    expect(wants(plan, 'claude')).toBe('show you its plan');
    expect(wants(plan, 'codex')).toBe('use the tool ExitPlanMode');
    expect(wants(plan, undefined)).toBe('use the tool ExitPlanMode');
  });

  it('has no plain "no" for a plan: its row opens the note field', () => {
    const plan = { kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add the line' } } as const;
    expect(noRow(plan, 'claude', 'Eli', 'tool')).toEqual({ title: "Send Eli's plan back with a note…", note: true });
    // Every other request is denied from the palette (after its confirmation), as before.
    expect(noRow({ kind: 'command', command: 'ls', cwd: null }, 'claude', 'Eli', 'ls')).toEqual({ title: 'Deny Eli: ls', note: false });
    expect(noRow({ kind: 'tool', name: 'WebFetch', input: null }, 'claude', 'Eli', 'tool')).toEqual({ title: 'Deny Eli: tool', note: false });
    // A request with the plan's name from another CLI, or from an agent Codly does not know, is a tool by its name.
    expect(noRow(plan, 'codex', 'Eli', 'tool')).toEqual({ title: 'Deny Eli: tool', note: false });
    expect(noRow(plan, undefined, 'agent', 'tool')).toEqual({ title: 'Deny agent: tool', note: false });
  });
});

describe('the palette rows of a waiting request', () => {
  it('a Claude agent’s plan is accepted, as on its card; every other request is approved once', () => {
    const plan = { kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add the line' } } as const;
    expect(okRow(plan, 'claude', 'Eli', 'show you its plan', 'Eli wants to show you its plan')).toEqual({ title: "Accept Eli's plan", confirm: 'Accept the plan? Eli wants to show you its plan' });
    expect(okRow({ kind: 'command', command: 'ls', cwd: null }, 'claude', 'Eli', 'ls', 'Eli wants to run ls')).toEqual({ title: 'Approve Eli: ls', confirm: 'Approve once? Eli wants to run ls' });
    // That request is Claude Code's own: from another CLI it is a tool by its name.
    expect(okRow(plan, 'codex', 'Bo', 'use the tool ExitPlanMode', 'Bo wants to use the tool ExitPlanMode').title).toBe('Approve Bo: use the tool ExitPlanMode');
  });
});

describe('the group "This agent"', () => {
  /** An agent with a live process, between turns, that took a turn before. */
  function running(id: string, provider: Agent['provider'], role: Agent['role']): Agent {
    const a = mkAgent(id, id[0].toUpperCase() + id.slice(1), provider, role, 1, 'idle');
    return { ...a, run_id: `run-${id}`, usage: { ...a.usage, input_tokens: 1200, output_tokens: 300 } };
  }
  function recorder() {
    const calls: string[] = [];
    const of = (what: string) => (a: Agent) => void calls.push(`${what} ${a.id}`);
    const on: AgentDo = {
      stop: () => void calls.push('stop'), agentMode: (a, mode) => void calls.push(`agentMode ${a.id} ${mode}`), agentModel: of('agentModel'),
      compact: of('compact'), context: of('context'), undo: of('undo'), terminal: of('terminal'), showTerminal: of('showTerminal'), mcp: of('mcp'),
    };
    return { on, calls };
  }
  const at = (over: Partial<SlashContext>): SlashContext => ({ oneToOne: false, agent: null, busy: false, changes: false, modes: [], mcp: [], takeover: { takeover: null, blocked: null }, turns: { off: null }, ...over });
  /** Title, then what its line says: what it does, or why it cannot run ("off: …"). */
  const shown = (c: SlashContext) => agentGroup(c, recorder().on).map((r) => [r.title, r.run ? r.sub : `off: ${r.sub}`]);

  it('in a request: the Lead’s controls, the ones that cannot run last with their reason', () => {
    const lead = running('lea', 'claude', 'lead');
    expect(shown(at({ agent: lead, busy: true, turns: { off: 'Lea has no worktree of its own, so there are no checkpoints.' } }))).toEqual([
      ['Stop Lea', 'Ends the running turn'],
      ['Check Lea’s context', 'How full its conversation is'],
      ['Continue in your terminal', 'Codly stops its own process first, and sends it nothing until you give it back'],
      ['Compact Lea’s context', 'off: Only a Codex agent or a 1:1 Claude agent'],
      ['Undo Lea’s last turn…', 'off: Lea has no worktree of its own, so there are no checkpoints.'],
      ['MCP servers', 'off: Lea’s run reports no MCP server'],
    ]);
    expect(agentGroup(at({ agent: lead }), recorder().on).every((r) => r.section === 'This agent')).toBe(true);
  });

  it('in a 1:1 chat with a Claude agent: its ways of working and its model too, and a row runs what the menu runs', () => {
    const ada = running('ada', 'claude', 'solo');
    const c = at({ oneToOne: true, agent: ada });
    expect(shown(c).map((r) => r[0])).toEqual(['Ask for edits', 'Plan first', 'Accept edits', 'Model', 'Compact Ada’s context', 'Check Ada’s context', 'Undo Ada’s last turn…', 'Continue in your terminal', 'Stop Ada', 'MCP servers']);
    const { on, calls } = recorder();
    const rows = agentGroup(c, on);
    rows.find((r) => r.title === 'Accept edits')!.run!();
    rows.find((r) => r.title === 'Undo Ada’s last turn…')!.run!();
    expect(calls).toEqual(['agentMode ada accept_edits', 'undo ada']);
  });

  it('in a 1:1 chat with a Codex agent: what is Claude Code’s says so, and goes last', () => {
    const cody = running('cody', 'codex', 'solo');
    const rows = shown(at({ oneToOne: true, agent: cody, turns: null }));
    expect(rows.slice(0, 4)).toEqual([
      ['Model', 'Cody’s model'],
      ['Compact Cody’s context', expect.stringContaining('Codex sums up the conversation so far')],
      // The core has not answered yet: the row waits in its place.
      ['Undo Cody’s last turn…', 'off: Reading what Codly kept of its turns…'],
      ['Continue in your terminal', 'Codly stops its own process first, and sends it nothing until you give it back'],
    ]);
    expect(rows.slice(4)).toEqual([
      ['Stop Cody', 'off: Nothing is running'],
      ['Ask for edits', 'off: Only a 1:1 Claude agent has a mode to switch'],
      ['Plan first', 'off: Only a 1:1 Claude agent has a mode to switch'],
      ['Accept edits', 'off: Only a 1:1 Claude agent has a mode to switch'],
      ['Check Cody’s context', 'off: Only Claude Code can be asked for it'],
      ['MCP servers', 'off: Cody’s run reports no MCP server'],
    ]);
  });

  it('with no agent there is no group', () => {
    expect(agentGroup(at({}), recorder().on)).toEqual([]);
    expect(agentGroup(at({ oneToOne: true }), recorder().on)).toEqual([]);
  });
});

describe('the group "Said in conversations"', () => {
  const names: SaidNames = {
    request: (t) => (t === 'lead:a' ? 'Fix the login page so that people who forgot their password can still get in' : null),
    closed: (t) => (t === 'closed:lead:b:900' ? 'Add a bye file' : null),
    agent: (id) => (id === 'ada' ? 'Ada' : null),
  };
  /** The letters a snippet marks. */
  const marked = (s: { line: string; hits: number[] }) => s.hits.map((i) => s.line[i]).join('');

  it('says where it was said, for each kind of thread', () => {
    expect(saidWhere('lead:a', names)).toBe('Request: Fix the login page so that people who…');
    expect(saidWhere('agent:ada', names)).toBe('1:1 with Ada');
    expect(saidWhere('agent:gone', names)).toBe('1:1 with an agent');
    expect(saidWhere('closed:lead:b:900', names)).toBe('Closed request: Add a bye file');
    expect(saidWhere('closed:lead:c:500', names)).toBe('Closed request');
    // A request with no message from you is its Lead's conversation.
    expect(saidWhere('lead', names)).toBe('Lead');
  });

  it('says who said it', () => {
    expect([saidBy('user', names), saidBy('codly', names), saidBy('ada', names), saidBy('l9', names)]).toEqual(['You', 'Codly', 'Ada', 'An agent']);
  });

  it('a hit at the start of a long message: its first words, cut at a word border', () => {
    const s = snippet(`Needle ${'word '.repeat(40)}`, 'needle');
    expect(s.line.startsWith('Needle word')).toBe(true);
    expect(s.line.endsWith('word…')).toBe(true);
    expect(s.line.length).toBeLessThanOrEqual(73);
    expect(marked(s)).toBe('Needle');
  });

  it('a hit in the middle: the words around it, cut on both sides', () => {
    const s = snippet(`${'word '.repeat(30)}needle ${'word '.repeat(30)}`, 'NEEDLE');
    expect(s.line).toMatch(/^…word .* needle .*word…$/);
    expect(s.line.length).toBeLessThanOrEqual(74);
    expect(marked(s)).toBe('needle');
  });

  it('a hit at the end: the words before it, nothing after', () => {
    const s = snippet(`${'word '.repeat(40)}the needle`, 'needle');
    expect(s.line).toMatch(/^…word .*the needle$/);
    expect(marked(s)).toBe('needle');
  });

  it('a hit in a code block keeps its one line', () => {
    const s = snippet('Here is the fix:\n```ts\n  const needle = 1;\nconst other = 2;\n```\nDone.', 'needle');
    expect(s).toEqual({ line: 'const needle = 1;', hits: [6, 7, 8, 9, 10, 11] });
  });

  it('a message that does not hold the query marks nothing (the palette leaves it out)', () => {
    expect(snippet('Something else', 'needle').hits).toEqual([]);
  });
});

describe('palette search words and the quick start order', () => {
  const row = (id: string, title: string, sub?: string, words?: string) => ({ id, title, sub, words });
  const goTo = [
    row('go:knowledge', 'Go to Knowledge', 'Team memory, knowledge graph, code search'),
    row('go:changes', 'Go to Changes', 'Diffs and artifacts', ROOM_WORDS.changes),
  ];

  it('"merge" finds the Changes room by its words, under the merge of a finished plan', () => {
    expect(rankRows(goTo, 'merge', [], 12).map((r) => r.id)).toEqual(['go:changes', 'go:knowledge']);
    const withPlan = [...goTo, row('plan:merge:p1', 'Merge into main: dark mode')];
    expect(rankRows(withPlan, 'merge', [], 12).map((r) => r.id)).toEqual(['plan:merge:p1', 'go:changes', 'go:knowledge']);
    expect(rankRows(goTo, 'review', [], 12).map((r) => r.id)).toEqual(['go:changes']);
    expect(rankRows(goTo, 'diff', [], 12)[0].id).toBe('go:changes');
    // A word match marks no letters of the title.
    expect(rankRows(goTo, 'review', [], 12)[0].hits).toEqual([]);
  });

  it('"@Ada" finds agents by their name only', () => {
    const agents = [row('agent:l1', 'Mira', 'Claude Code · reading adapters.rs'), row('agent:a1', 'Ada', 'Codex · idle')];
    expect(rankRows(agents, 'Ada', [], 50, true).map((r) => r.id)).toEqual(['agent:a1']);
    expect(rankRows(agents, 'Zed', [], 50, true)).toEqual([]);
  });

  it('quick start lists the CLIs as the wizard does, Claude Code first', () => {
    const clis = [{ provider: 'opencode' }, { provider: 'codex' }, { provider: 'gemini' }, { provider: 'claude' }];
    expect(inCliOrder(clis).map((c) => c.provider)).toEqual(['claude', 'codex', 'gemini', 'opencode']);
    expect(clis[0].provider).toBe('opencode'); // the store's list is not changed
  });
});

describe('palette keys', () => {
  it('Esc clears the text of a scope, and closes an empty scope or the palette', () => {
    expect(escStep('@', 'Ada')).toBe('clear');
    expect(escStep('>', 'merge')).toBe('clear');
    expect(escStep('@', '')).toBe('close');
    expect(escStep(null, 'fix the bug')).toBe('close');
    expect(escStep(null, '')).toBe('close');
  });

  it('"Ask the Lead…" alone asks once when ↵ takes it; a click sends at once', () => {
    const ask = { id: 'ask', askFirst: askFirst(0) };
    expect(ask.askFirst).toBe('Send this to the Lead? It starts a turn of the Lead.');
    const first = press({ id: ask.id, confirm: guard(ask, true, false) }, null, false);
    expect(first).toEqual({ run: false, armed: 'ask' });
    expect(press({ id: ask.id, confirm: guard(ask, true, false) }, first.armed, false)).toEqual({ run: true, armed: null });
    expect(press({ id: ask.id, confirm: guard(ask, false, false) }, null, false)).toEqual({ run: true, armed: null });
    // ⌘↵ only puts the text in the composer: nothing to ask.
    expect(guard(ask, true, true)).toBeUndefined();
  });

  it('"Ask the Lead…" under other rows asks nothing: ↵ reached it on purpose', () => {
    expect(askFirst(3)).toBeUndefined();
    expect(guard({ askFirst: askFirst(3) }, true, false)).toBeUndefined();
    // A row that cannot be taken back asks however it was taken.
    expect(guard({ confirm: 'Merge?' }, false, false)).toBe('Merge?');
  });
});
