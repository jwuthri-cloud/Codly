import { describe, expect, it } from 'vitest';
import type { ApprovalSubject } from '../../gen/ApprovalSubject';
import { planCallEnd, showsPlan, wantsTo } from '../planTool';

const tool = (name: string, input: Record<string, string> = {}): ApprovalSubject => ({ kind: 'tool', name, input });

describe('what a request asks for, in words', () => {
  it('a plan is "its plan": the name of the tool that carries it is never shown', () => {
    expect(showsPlan(tool('ExitPlanMode', { plan: '1. Add the line' }))).toBe(true);
    expect(showsPlan(tool('WebFetch'))).toBe(false);
    expect(showsPlan({ kind: 'command', command: 'ExitPlanMode', cwd: null })).toBe(false);
    expect(wantsTo(tool('ExitPlanMode', { plan: '1. Add the line' }))).toBe('show you its plan');
  });

  it('every kind of request is a sentence after "wants to"', () => {
    expect(wantsTo({ kind: 'command', command: 'pnpm test', cwd: null })).toBe('run pnpm test');
    expect(wantsTo({ kind: 'file_change', paths: ['src/theme/theme.ts'], diff: null })).toBe('change theme.ts');
    expect(wantsTo({ kind: 'file_change', paths: ['a.ts', 'b.ts'], diff: null })).toBe('change 2 files');
    expect(wantsTo(tool('WebFetch', { url: 'https://example.com' }))).toBe('use WebFetch');
    expect(wantsTo(tool('SandboxNetworkAccess', { host: 'example.com' }))).toBe('reach example.com');
    expect(wantsTo({ kind: 'question', questions: [] })).toBe('ask you a question');
    expect(wantsTo({ kind: 'permissions', detail: {} })).toBe('get more permissions');
  });

  it('one short line cuts a long command', () => {
    expect(wantsTo({ kind: 'command', command: `echo ${'a'.repeat(100)}`, cwd: null }, 20)).toHaveLength('run '.length + 20);
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
