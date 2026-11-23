import { describe, expect, it } from 'vitest';
import type { ApprovalSubject } from '../../gen/ApprovalSubject';
import { shownPlan } from '../planRequest';
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
