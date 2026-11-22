import { describe, expect, it } from 'vitest';
import { fileMention, filterSlash, rankFiles, triggerAt } from '../composerMenu';

/** The menu at the end of `text` (the caret is where typing left it). */
const at = (text: string) => triggerAt(text, text.length);

describe('triggerAt', () => {
  it('@ at the start of the message talks to an agent; later it mentions', () => {
    expect(at('@Ad')).toEqual({ trigger: '@', start: 0, query: 'Ad', atStart: true });
    expect(at('  @')).toEqual({ trigger: '@', start: 2, query: '', atStart: true });
    expect(at('ask @Ad')).toEqual({ trigger: '@', start: 4, query: 'Ad', atStart: false });
  });

  it('a path after @ stays one query: slashes, dots and dashes belong to it', () => {
    expect(at('look at @src/lib/api.ts')).toMatchObject({ trigger: '@', start: 8, query: 'src/lib/api.ts' });
    expect(at('see @my-file_2.test')).toMatchObject({ query: 'my-file_2.test' });
  });

  it('# refers to a task', () => {
    expect(at('after #T-1')).toEqual({ trigger: '#', start: 6, query: 'T-1', atStart: false });
  });

  it('@ or # inside a word, or with the caret past the word, opens nothing', () => {
    expect(at('mail me at a@b')).toBeNull();
    expect(at('issue#12')).toBeNull();
    expect(at('@Ada ')).toBeNull();
    expect(at('plain text')).toBeNull();
  });

  it('the caret decides: the word before it is the query', () => {
    expect(triggerAt('ask @Ada to help', 8)).toMatchObject({ trigger: '@', start: 4, query: 'Ada' });
    expect(triggerAt('ask @Ada to help', 6)).toMatchObject({ query: 'A' });
    expect(triggerAt('ask @Ada to help', 12)).toBeNull();
  });

  it('/ opens the actions only when the message is that one word', () => {
    expect(at('/')).toEqual({ trigger: '/', start: 0, query: '', atStart: true });
    expect(at('/mo')).toEqual({ trigger: '/', start: 0, query: 'mo', atStart: true });
    // The whole word is the query, wherever the caret is in it; a space typed after it changes nothing.
    expect(triggerAt('/model', 3)).toMatchObject({ trigger: '/', query: 'model' });
    expect(at('/new ')).toMatchObject({ trigger: '/', query: 'new' });
    expect(triggerAt('/new', 0)).toBeNull(); // the caret before the slash
  });

  it('a path or a sentence that starts with a slash is plain text', () => {
    expect(at('/usr/local')).toBeNull();
    expect(at('/stop the server and')).toBeNull();
    expect(triggerAt('/stop please', 5)).toBeNull(); // more text after the word
    expect(at(' /new')).toBeNull();
    expect(at('run /new')).toBeNull();
    expect(at('line one\n/new')).toBeNull();
    // A mention in such a sentence still works.
    expect(at('/stop @Ad')).toMatchObject({ trigger: '@', query: 'Ad', atStart: false });
  });
});

describe('rankFiles', () => {
  const paths = ['src/api/index.ts', 'docs/rapid.md', 'src/lib/api.ts', 'src/lib/api.test.ts', 'README.md', 'src/home/Composer.svelte'];

  it('nothing typed lists no file', () => {
    expect(rankFiles(paths, '', 6)).toEqual([]);
  });

  it("a match in the file's name comes before a match in its folders", () => {
    expect(rankFiles(paths, 'api', 6)).toEqual(['src/lib/api.ts', 'src/lib/api.test.ts', 'docs/rapid.md', 'src/api/index.ts']);
  });

  it('finds a name by a few of its letters, whatever their case', () => {
    expect(rankFiles(paths, 'cmpsr', 6)).toEqual(['src/home/Composer.svelte']);
    expect(rankFiles(paths, 'readme', 6)).toEqual(['README.md']);
  });

  it('a query with a slash is matched against the whole path', () => {
    expect(rankFiles(paths, 'lib/api', 6)).toEqual(['src/lib/api.ts', 'src/lib/api.test.ts']);
    expect(rankFiles(paths, 'api/', 6)).toEqual(['src/api/index.ts']);
  });

  it('shows at most `limit` files, and none when nothing matches', () => {
    expect(rankFiles(paths, 's', 2)).toHaveLength(2);
    expect(rankFiles(paths, 'zzz', 6)).toEqual([]);
  });

  it('a file is mentioned by its path, in backticks, ready for the next word', () => {
    expect(fileMention('src/lib/api.ts')).toBe('`src/lib/api.ts` ');
  });
});

describe('filterSlash', () => {
  const commands = [
    { name: 'new', label: 'New request' },
    { name: 'stop', label: 'Stop Mira' },
    { name: 'plan', label: 'Plan mode' },
    { name: 'model', label: 'Model' },
    { name: 'clear', label: 'Clear the draft' },
    { name: 'closed', label: 'Closed requests' },
    { name: 'settings', label: 'Settings' },
  ];
  const names = (q: string) => filterSlash(commands, q).map((c) => c.name);

  it('nothing typed lists every action, in order', () => {
    expect(names('')).toEqual(['new', 'stop', 'plan', 'model', 'clear', 'closed', 'settings']);
  });

  it('actions whose name starts with what is typed come first, in the list’s order', () => {
    expect(names('s')).toEqual(['stop', 'settings', 'new', 'closed']); // then "New request" and "closed" hold an s
    expect(names('cl')).toEqual(['clear', 'closed']);
    expect(names('MO')).toEqual(['model', 'plan']); // "Plan mode"
  });

  it('a word of the label finds its action', () => {
    expect(names('draft')).toEqual(['clear']);
    expect(names('request')).toEqual(['new', 'closed']);
  });

  it('what matches no action lists none, so the line is sent as text', () => {
    expect(names('usr')).toEqual([]);
  });
});
