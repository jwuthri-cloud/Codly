import { describe, expect, it } from 'vitest';
import type { RepoEntry } from '../gen/RepoEntry';
import { filterRepos, typedPath } from '../util/repos';

const repo = (name: string, path: string): RepoEntry => ({ name, path, modified_ms: 0 });
const repos = [
  repo('CrossModalAlignment', '/home/me/git/CrossModalAlignment'),
  repo('Codly', '/home/me/git/Codly'),
  repo('notes', '/home/me/Documents/notes'),
];

describe('repository picker', () => {
  it('keeps the repositories that hold every word, in any case, in list order', () => {
    expect(filterRepos(repos, '')).toEqual(repos);
    expect(filterRepos(repos, 'cod').map((r) => r.name)).toEqual(['Codly']);
    expect(filterRepos(repos, 'GIT  modal').map((r) => r.name)).toEqual(['CrossModalAlignment']);
    expect(filterRepos(repos, 'documents').map((r) => r.name)).toEqual(['notes']);
    expect(filterRepos(repos, 'git zzz')).toEqual([]);
  });

  it('opens a full path as typed and treats anything else as a search', () => {
    expect(typedPath('  /srv/repos/app ')).toBe('/srv/repos/app');
    expect(typedPath('codly')).toBeNull();
    expect(typedPath('~/git/codly')).toBeNull();
  });
});
