import { describe, expect, it } from 'vitest';
import { lineText, permissionsAsked, placeOf, within } from '../permissionsAsked';

const CWD = '/Users/someone/Code/acme';
// The shape of `PermissionsRequestApprovalParams` (Codex app-server protocol, generated types).
const req = (permissions: unknown, cwd: string | null = CWD) => ({
  threadId: 't', turnId: 'u', itemId: 'i', environmentId: null, startedAtMs: 1, cwd, reason: 'r', permissions,
});

describe('placeOf', () => {
  it('reads a near path relative to where the agent works', () => {
    expect(placeOf('/Users/someone/Code/acme-docs', CWD)).toBe('../acme-docs');
    expect(placeOf('/Users/someone/Code/acme/docs', CWD)).toBe('docs');
    expect(placeOf('/Users/someone/Code/acme', CWD)).toBe('.');
    expect(placeOf('/Users/someone/x/y', CWD)).toBe('../../x/y');
  });
  it('gives a far path whole, with ~ for the home folder', () => {
    expect(placeOf('/Users/someone/elsewhere/deep/z', '/Users/someone/Code/acme/.codly/worktrees/kit')).toBe('~/elsewhere/deep/z');
    expect(placeOf('/etc/hosts', CWD)).toBe('/etc/hosts');
    expect(placeOf('/Users/someone/a', null)).toBe('~/a');
  });
  it('keeps a relative path as it is', () => {
    expect(placeOf('build/out', CWD)).toBe('build/out');
  });
});

describe('within', () => {
  it('is true for the folder and below it, not for a sibling with the same start', () => {
    expect(within('/a/b', '/a/b')).toBe(true);
    expect(within('/a/b/c', '/a/b/')).toBe(true);
    expect(within('/a/bc', '/a/b')).toBe(false);
  });
});

describe('permissionsAsked', () => {
  it('says the folder it wants to write in, outside where it works', () => {
    const a = permissionsAsked(req({ network: null, fileSystem: { read: null, write: ['/Users/someone/Code/acme-docs'] } }));
    expect(a.wants).toBe('write in ../acme-docs');
    expect(a.lines).toEqual([{ verb: 'Write in', place: '../acme-docs', outside: true }]);
    expect(a.known).toBe(true);
    expect(a.cwd).toBe(CWD);
  });

  it('says a read, and a path inside the folder is not outside', () => {
    const a = permissionsAsked(req({ network: null, fileSystem: { read: [`${CWD}/vendor`], write: null } }));
    expect(a.wants).toBe('read vendor');
    expect(a.lines[0].outside).toBe(false);
  });

  it('says the network', () => {
    expect(permissionsAsked(req({ network: { enabled: true }, fileSystem: null })).wants).toBe('use the network');
    // enabled false or null asks for nothing
    expect(permissionsAsked(req({ network: { enabled: false }, fileSystem: null })).wants).toBe('get more permissions');
  });

  it('joins several asks, and counts several places', () => {
    const a = permissionsAsked(req({ network: { enabled: true }, fileSystem: { read: ['/opt/a', '/opt/b'], write: ['/Users/someone/Code/acme-docs'] } }));
    expect(a.wants).toBe('write in ../acme-docs, read 2 places and use the network');
    expect(a.lines.map(lineText)).toEqual(['Write in ../acme-docs', 'Read /opt/a', 'Read /opt/b', 'Use the network']);
  });

  it('reads the entries form, without listing a path twice when the older lists name it too', () => {
    const a = permissionsAsked(req({
      network: null,
      fileSystem: {
        read: null, write: ['/Users/someone/Code/acme-docs'],
        entries: [
          { path: { type: 'path', path: '/Users/someone/Code/acme-docs' }, access: 'write' },
          { path: { type: 'glob_pattern', pattern: '**/*.lock' }, access: 'read' },
          { path: { type: 'special', value: { kind: 'tmpdir' } }, access: 'write' },
          { path: { type: 'special', value: { kind: 'project_roots', subpath: 'docs' } }, access: 'read' },
          { path: { type: 'path', path: `${CWD}/.env` }, access: 'deny' },
        ],
      },
    }));
    expect(a.lines.map(lineText)).toEqual([
      'Write in ../acme-docs',
      'Read the files that match **/*.lock',
      'Write in the temporary folder',
      'Read docs in the project folders',
      'Keep out of .env',
    ]);
    expect(a.wants).toBe('write in 2 places and read 2 places');
    expect(a.known).toBe(true);
  });

  it('marks a shape it cannot read as not known, and keeps what it could read', () => {
    const a = permissionsAsked(req({ network: null, fileSystem: { read: null, write: ['/x'], entries: [{ path: { type: 'new_kind' }, access: 'write' }] } }));
    expect(a.lines.map(lineText)).toEqual(['Write in /x']);
    expect(a.known).toBe(false);
  });

  it('says only "get more permissions" for an unknown or empty shape', () => {
    for (const d of [null, 'text', 42, {}, { permissions: 'all' }, { cwd: CWD, permissions: { sandbox: 'off' } }]) {
      const a = permissionsAsked(d);
      expect(a.wants).toBe('get more permissions');
      expect(a.lines).toEqual([]);
      expect(a.known).toBe(false);
    }
    const empty = permissionsAsked(req({ network: null, fileSystem: null }));
    expect(empty.wants).toBe('get more permissions');
    expect(empty.known).toBe(true);
  });

  it('cuts a long path in the headline, not in its line', () => {
    const long = `/opt/${'x'.repeat(100)}`;
    const a = permissionsAsked(req({ network: null, fileSystem: { read: null, write: [long] } }));
    expect(a.wants.length).toBeLessThan(80);
    expect(a.lines[0].place).toBe(long);
  });
});
