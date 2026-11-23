import { describe, expect, it } from 'vitest';
import type { ApprovalSubject } from '../../gen/ApprovalSubject';
import { isPlanCall, planCallEnd, shownPlan, showsPlan, wantsTo } from '../planRequest';
import { sandboxHost } from '../sandbox';

const plan: ApprovalSubject = { kind: 'tool', name: 'ExitPlanMode', input: { plan: '# Plan\n1. Add the switch' } };
const host: ApprovalSubject = { kind: 'tool', name: 'SandboxNetworkAccess', input: { host: 'example.com' } };

describe('which card a tool request shows as', () => {
  it('a Claude agent’s plan is the plan card', () => {
    expect(shownPlan(plan, 'claude')).toBe('# Plan\n1. Add the switch');
    expect(sandboxHost(plan, 'claude')).toBeNull();
  });

  it('a Claude agent’s blocked host is the network card', () => {
    expect(sandboxHost(host, 'claude')).toBe('example.com');
    expect(shownPlan(host, 'claude')).toBeNull();
  });

  it('the same two names from another CLI are tools like any other: name and raw input', () => {
    for (const provider of ['codex', 'gemini', 'opencode', 'cursor', 'copilot'] as const) {
      expect(shownPlan(plan, provider), provider).toBeNull();
      expect(sandboxHost(host, provider), provider).toBeNull();
    }
  });

  it('an agent Codly does not know counts as not Claude', () => {
    for (const provider of [null, undefined]) {
      expect(shownPlan(plan, provider)).toBeNull();
      expect(sandboxHost(host, provider)).toBeNull();
    }
  });

  it('a plan request with no plan text, and every other request, is no plan', () => {
    const others: ApprovalSubject[] = [
      { kind: 'tool', name: 'ExitPlanMode', input: {} },
      { kind: 'tool', name: 'ExitPlanMode', input: null },
      { kind: 'tool', name: 'ExitPlanMode', input: { plan: 7 } },
      { kind: 'tool', name: 'WebFetch', input: { plan: 'x' } },
      { kind: 'command', command: 'ls', cwd: null },
    ];
    for (const s of others) expect(shownPlan(s, 'claude')).toBeNull();
  });
});

describe('what a request asks for, in words', () => {
  const tool = (name: string, input: Record<string, string> = {}): ApprovalSubject => ({ kind: 'tool', name, input });

  it('a Claude agent’s plan is "its plan": the name of the tool that carries it is never shown', () => {
    expect(showsPlan(plan, 'claude')).toBe(true);
    expect(showsPlan(tool('WebFetch'), 'claude')).toBe(false);
    expect(showsPlan({ kind: 'command', command: 'ExitPlanMode', cwd: null }, 'claude')).toBe(false);
    expect(wantsTo(plan, 'claude')).toBe('show you its plan');
  });

  it('the same request from another CLI, or from an agent Codly does not know, is a tool by its name', () => {
    for (const provider of ['codex', 'gemini', 'opencode', null, undefined] as const) {
      expect(showsPlan(plan, provider), String(provider)).toBe(false);
      expect(wantsTo(plan, provider), String(provider)).toBe('use ExitPlanMode');
      expect(isPlanCall('ExitPlanMode', provider), String(provider)).toBe(false);
    }
    expect(isPlanCall('ExitPlanMode', 'claude')).toBe(true);
    expect(isPlanCall('Bash', 'claude')).toBe(false);
  });

  it('what the card does not show as a plan (no plan text) is not called one', () => {
    expect(showsPlan(tool('ExitPlanMode'), 'claude')).toBe(false);
    expect(wantsTo(tool('ExitPlanMode'), 'claude')).toBe('use ExitPlanMode');
  });

  it('every kind of request is a sentence after "wants to"', () => {
    expect(wantsTo({ kind: 'command', command: 'pnpm test', cwd: null }, 'claude')).toBe('run pnpm test');
    expect(wantsTo({ kind: 'file_change', paths: ['src/theme/theme.ts'], diff: null }, 'codex')).toBe('change theme.ts');
    expect(wantsTo({ kind: 'file_change', paths: ['a.ts', 'b.ts'], diff: null }, null)).toBe('change 2 files');
    expect(wantsTo(tool('WebFetch', { url: 'https://example.com' }), 'claude')).toBe('use WebFetch');
    expect(wantsTo(host, 'claude')).toBe('reach example.com');
    expect(wantsTo(host, 'codex')).toBe('use SandboxNetworkAccess');
    expect(wantsTo({ kind: 'question', questions: [] }, 'claude')).toBe('ask you a question');
    expect(wantsTo({ kind: 'permissions', detail: {} }, 'claude')).toBe('get more permissions');
  });

  it('one short line cuts a long command', () => {
    expect(wantsTo({ kind: 'command', command: `echo ${'a'.repeat(100)}`, cwd: null }, 'claude', 20)).toHaveLength('run '.length + 20);
  });
});

describe('how a call that showed a plan ended', () => {
  it('you accepted the plan or sent it back: it did not fail', () => {
    expect(planCallEnd(true, 'approved')).toBe('You accepted the plan');
    expect(planCallEnd(false, 'denied')).toBe('You sent the plan back');
    // The request is not held any more (an older page of the history).
    expect(planCallEnd(false, null)).toBe('You sent the plan back');
  });

  it('a plan that was not accepted for another reason says only that', () => {
    expect(planCallEnd(false, 'cancelled')).toBe('The plan was not accepted');
    expect(planCallEnd(false, 'expired')).toBe('The plan was not accepted');
  });
});
