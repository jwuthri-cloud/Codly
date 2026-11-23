import { describe, expect, it } from 'vitest';
import { bySection, noRow, press, wants } from '../palette';

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
