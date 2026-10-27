import { describe, expect, it } from 'vitest';
import { mockIde } from '../mockFs';

const call = (cmd: string, a: Record<string, unknown> = {}) => mockIde(cmd, { agentId: 'a1', ...a });

describe('mock IDE backend', () => {
  it('lists folders first and loads lazily one level at a time', () => {
    const top = call('ide_list', { path: '' }) as { name: string; is_dir: boolean }[];
    expect(top.some((e) => e.name === 'src' && e.is_dir)).toBe(true);
    expect(top.findIndex((e) => !e.is_dir)).toBeGreaterThan(top.findLastIndex((e) => e.is_dir));
    const theme = call('ide_list', { path: 'src/theme' }) as { path: string }[];
    expect(theme.map((e) => e.path)).toContain('src/theme/ThemeProvider.tsx');
  });

  it('refuses paths that escape or touch .git', () => {
    for (const path of ['../x', '/etc/passwd', 'src/../../x', '.git/config'])
      expect(() => call('ide_read', { path })).toThrow(/outside the workspace/);
  });

  it('save shows up as a change with a diff, create makes a new file', () => {
    expect(call('ide_diff', { path: 'README.md' })).toBe('');
    call('ide_write', { path: 'README.md', content: '# acme/web\n\nEdited.\n' });
    expect((call('ide_changes') as { path: string }[]).map((c) => c.path)).toContain('README.md');
    expect(call('ide_diff', { path: 'README.md' }) as string).toContain('+Edited.');
    call('ide_create', { path: 'notes/todo.md' });
    expect(() => call('ide_create', { path: 'notes/todo.md' })).toThrow(/already exists/);
    expect(() => call('ide_write', { path: 'nope.txt', content: 'x' })).toThrow();
  });

  it('searches file names and text', () => {
    const r = call('ide_search', { query: 'themeprovider' }) as { files: string[]; hits: { path: string }[] };
    expect(r.files).toContain('src/theme/ThemeProvider.tsx');
    expect(r.hits.length).toBeGreaterThan(0);
    expect((call('ide_search', { query: 'x' }) as { hits: unknown[] }).hits).toHaveLength(0);
  });
});
