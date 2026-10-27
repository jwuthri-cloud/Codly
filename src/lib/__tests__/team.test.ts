import { describe, expect, it } from 'vitest';
import { PRESETS, fromPreset, joinModel, memberTitle, splitModel, teamProblem } from '../util/team';
import type { TeamMember } from '../gen/TeamMember';

const m = (name: string, o: Partial<TeamMember> = {}): TeamMember => ({ id: '', name, provider: 'claude', model: null, kind: 'worker', title: '', role: '', enabled: true, ...o });

describe('team', () => {
  it('titles a member like the core does', () => {
    expect(memberTitle(m('Ana', { title: 'product manager', role: 'Backend dev: owns crates/' }))).toBe('Product manager');
    expect(memberTitle(m('Ana', { role: 'Backend dev: owns crates/' }))).toBe('Backend dev');
    expect(memberTitle(m('Ana', { role: 'Docs — guides' }))).toBe('Docs');
    expect(memberTitle(m('Rex', { kind: 'reviewer' }))).toBe('Reviewer');
    expect(memberTitle(m('Ana', { role: 'A role sentence that is far too long for a badge' }))).toBe('Teammate');
  });

  it('splits and joins model:effort', () => {
    expect(splitModel('opus:high')).toEqual({ name: 'opus', effort: 'high' });
    expect(splitModel('vendor/model:7b')).toEqual({ name: 'vendor/model:7b', effort: '' });
    expect(joinModel(' sonnet ', 'max')).toBe('sonnet:max');
    expect(joinModel('', '')).toBeNull();
  });

  it('refuses empty, reserved and duplicate names', () => {
    expect(teamProblem([m('Ana'), m('Leo')])).toBeNull();
    expect(teamProblem([m(' ')])).toMatch(/needs a name/);
    expect(teamProblem([m('Consultant')])).toMatch(/something else/);
    expect(teamProblem([m('ana')], [m('Ana')])).toMatch(/Two members/);
  });

  it('builds a preset on a CLI you can run, with a free name', () => {
    const rex = PRESETS.find((p) => p.kind === 'reviewer')!;
    const onClaude = fromPreset(rex, ['claude'], ['Rex'])!;
    expect(onClaude.provider).toBe('claude');
    expect(onClaude.name).toBe('Rex 2');
    expect(onClaude.model).toBeNull(); // its model was for another CLI
    expect(fromPreset(rex, [], [])).toBeNull();
  });
});
