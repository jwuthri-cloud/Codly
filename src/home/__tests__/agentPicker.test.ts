// The picker under a 1:1 chat's composer: what its chip says and what its menu lists, per agent.
import { describe, expect, it } from 'vitest';
import type { Agent } from '../../lib/gen/Agent';
import type { ApprovalMode } from '../../lib/gen/ApprovalMode';
import { chipOf, groupsOf, wayOf } from '../agentPicker';

/** The permissions, as LeadControls lists them. */
const PERMISSIONS: { id: ApprovalMode; label: string; hint: string }[] = [
  { id: 'strict', label: 'Ask first', hint: 'Every change asks you' },
  { id: 'balanced', label: 'Balanced', hint: 'Only risky ones ask' },
  { id: 'autonomous', label: 'Autonomous', hint: 'Worktree edits allowed' },
];
const agent = (provider: Agent['provider'], role: Agent['role'], permission_mode?: Agent['permission_mode']) => ({ provider, role, permission_mode });

describe('the way of working the picker can switch', () => {
  it('is a 1:1 Claude agent’s, in its own chat; one that never chose asks for edits', () => {
    expect(wayOf(agent('claude', 'solo'), false)).toBe('ask');
    expect(wayOf(agent('claude', 'solo', 'plan'), false)).toBe('plan');
    expect(wayOf(agent('claude', 'solo', 'accept_edits'), false)).toBe('accept_edits');
  });

  it('is none for another CLI, for a plan’s worker, and in a chat that shows its request’s permissions', () => {
    expect(wayOf(agent('codex', 'solo'), false)).toBeNull();
    expect(wayOf(agent('claude', 'worker', 'plan'), false)).toBeNull();
    // A 1:1 agent on a task: the picker is the request's.
    expect(wayOf(agent('claude', 'solo', 'plan'), true)).toBeNull();
    expect(wayOf(null, false)).toBeNull();
  });
});

describe('the chip', () => {
  const words = (c: { text: string; sub: string | null }) => [c.text, c.sub].filter(Boolean).join(' ');

  it('follows the agent’s way of working', () => {
    expect(words(chipOf('Balanced', { own: true, request: false, way: 'plan' }))).toBe('Plan first · then Balanced');
    expect(words(chipOf('Balanced', { own: true, request: false, way: 'accept_edits' }))).toBe('Accept edits · Balanced');
    // Every way is named, asking for edits too.
    expect(words(chipOf('Balanced', { own: true, request: false, way: 'ask' }))).toBe('Ask for edits · Balanced');
    expect(words(chipOf('Ask first', { own: false, request: false, way: 'ask' }))).toBe('Ask for edits · Ask first');
  });

  it('is the permissions alone for the other agents, as before', () => {
    expect(words(chipOf('Autonomous', { own: true, request: false, way: null }))).toBe('Autonomous');
    expect(words(chipOf('Balanced', { own: false, request: false, way: null }))).toBe('Balanced · Settings');
    expect(words(chipOf('Balanced', { own: true, request: true, way: null }))).toBe('Balanced · whole request');
  });

  it('its tip says what the way of working changes', () => {
    expect(chipOf('Balanced', { own: true, request: false, way: 'plan' }).tip).toContain('until you accept its plan, only reads are approved for it');
    expect(chipOf('Balanced', { own: true, request: false, way: 'accept_edits' }).tip).toContain('your permissions do not see them');
    expect(chipOf('Balanced', { own: true, request: false, way: 'ask' }).tip).toBe('Every edit follows your permissions first, like a command.');
    expect(chipOf('Balanced', { own: true, request: false, way: null }).tip).toBe('How much this agent asks you before it acts');
    expect(chipOf('Balanced', { own: true, request: true, way: null }).tip).toBe('How much every agent on this request asks you before it acts');
  });
});

describe('the chip’s menu', () => {
  const read = (groups: ReturnType<typeof groupsOf>) => groups.map((g) => [g.header, ...g.entries.map((e) => `${e.checked ? '✓ ' : ''}${e.label}`)]);

  it('a 1:1 Claude agent has a second group, "How it works", with its own way checked', () => {
    expect(read(groupsOf(PERMISSIONS, 'balanced', 'accept_edits'))).toEqual([
      ['Permissions', 'Ask first', '✓ Balanced', 'Autonomous'],
      ['How it works', 'Ask for edits', '✓ Accept edits', 'Plan first'],
    ]);
    expect(read(groupsOf(PERMISSIONS, 'strict', 'ask'))[1]).toEqual(['How it works', '✓ Ask for edits', 'Accept edits', 'Plan first']);
    // In "plan first" the permissions apply once you accept its plan: their header says so.
    expect(read(groupsOf(PERMISSIONS, 'autonomous', 'plan'))).toEqual([
      ['Permissions, once you accept its plan', 'Ask first', 'Balanced', '✓ Autonomous'],
      ['How it works', 'Ask for edits', 'Accept edits', '✓ Plan first'],
    ]);
  });

  it('each way is the one a pick sets, with a few words on what it means', () => {
    const [, ways] = groupsOf(PERMISSIONS, 'balanced', 'ask');
    expect(ways.entries.map((e) => [e.id, e.hint])).toEqual([['ask', 'Per your permissions'], ['accept_edits', 'Edits do not ask you'], ['plan', 'You accept its plan']]);
  });

  it('the other agents see the permissions alone, as before', () => {
    expect(read(groupsOf(PERMISSIONS, 'balanced', null))).toEqual([['Permissions', 'Ask first', '✓ Balanced', 'Autonomous']]);
  });
});
