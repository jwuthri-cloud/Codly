import { describe, expect, it } from 'vitest';
import { fileMention, filterSlash, MenuPointer, pointerMoved, rankFiles, triggerAt, unknownSlash, unknownSlashLine } from '../composerMenu';

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

  describe('an action that cannot run right now', () => {
    // Nothing is running, and the request's plan has changed nothing yet.
    const idle = [
      { name: 'new', label: 'New request' },
      { name: 'stop', label: 'Stop Mira', off: 'Nothing is running' },
      { name: 'diff', label: 'View the changes', off: 'This request has no changes yet' },
      { name: 'settings', label: 'Settings' },
    ];
    const found = (q: string) => filterSlash(idle, q).map((c) => (c.off ? `${c.name} (${c.off})` : c.name));

    it('is still found by its name, with the reason: the name is known, so it is not sent as text', () => {
      expect(found('stop')).toEqual(['stop (Nothing is running)']);
      expect(found('sto')).toEqual(['stop (Nothing is running)']);
      expect(found('diff')).toEqual(['diff (This request has no changes yet)']);
    });

    it('comes after the actions that can run, so ↵ on a few letters runs one of those', () => {
      expect(found('')).toEqual(['new', 'settings', 'stop (Nothing is running)', 'diff (This request has no changes yet)']);
      expect(found('s')).toEqual(['settings', 'new', 'stop (Nothing is running)', 'diff (This request has no changes yet)']); // "changes" holds an s
    });
  });

  describe('an action that waits for the core’s answer', () => {
    // `/` just opened the menu: the core has not said yet what it kept of Ada's turns, or if she can go to your terminal.
    const asked = [
      { name: 'new', label: 'New request' },
      { name: 'stop', label: 'Stop Ada', off: 'Nothing is running' },
      { name: 'compact', label: 'Compact Ada’s context' },
      { name: 'undo', label: 'Undo Ada’s last turn…', off: 'Reading what Codly kept of its turns…', wait: true },
      { name: 'terminal', label: 'Continue in your terminal…', off: 'Asking Codly if it can go…', wait: true },
    ];
    const found = (rows: typeof asked, q: string) => filterSlash(rows, q).map((c) => (c.off ? `${c.name} (off)` : c.name));

    it('keeps its place: ↵ on its name runs nothing, and not the action that holds the same letters', () => {
      expect(found(asked, 'u')).toEqual(['undo (off)', 'new', 'terminal (off)']); // "New request" and "your" hold a u
      expect(found(asked, 'te')).toEqual(['terminal (off)', 'compact']); // "context" holds "te"
      expect(found(asked, '')).toEqual(['new', 'compact', 'undo (off)', 'terminal (off)', 'stop (off)']);
    });

    it('goes last when the core’s answer is no, as any action that cannot run', () => {
      const answered = asked.map((c) => (c.name === 'undo' ? { name: c.name, label: c.label, off: 'Codly kept no turn of Ada yet' } : c));
      expect(found(answered, 'u')).toEqual(['new', 'terminal (off)', 'undo (off)']);
    });
  });
});

describe('a slash command Codly does not have', () => {
  const names = ['new', 'stop', 'model', 'compact', 'context', 'clear', 'closed', 'undo', 'terminal', 'mcp'];

  it('is held back with the action you most likely meant', () => {
    expect(unknownSlash('/comapct', names)).toEqual({ word: 'comapct', closest: 'compact' });
    expect(unknownSlash('/modle please', names)).toEqual({ word: 'modle', closest: 'model' });
    expect(unknownSlash('  /termnal', names)).toEqual({ word: 'termnal', closest: 'terminal' });
    // Nothing near: no guess.
    expect(unknownSlash('/review this', names)).toEqual({ word: 'review', closest: null });
    expect(unknownSlashLine({ word: 'comapct', closest: 'compact' })).toBe('No such command: /comapct. Did you mean /compact? ↵ sends nothing until you change it.');
    expect(unknownSlashLine({ word: 'review', closest: null })).toBe('No such command: /review. Type / alone to see the actions. ↵ sends nothing until you change it.');
  });

  it('a known action, a path or a slash later in the text is not one', () => {
    expect(unknownSlash('/compact', names)).toBeNull();
    expect(unknownSlash('/Model', names)).toBeNull();
    expect(unknownSlash('/usr/bin/env is missing', names)).toBeNull();
    expect(unknownSlash('look at /etc/hosts', names)).toBeNull();
    expect(unknownSlash('/ is the root', names)).toBeNull();
    expect(unknownSlash('', names)).toBeNull();
  });
});

describe('a first word with a second slash', () => {
  const names = ['new', 'stop', 'model', 'compact', 'clear', 'undo', 'terminal', 'mcp'];

  it('that starts with an action’s name is a mistyped command: it is not sent', () => {
    expect(unknownSlash('/mcp/terminal', names)).toEqual({ word: 'mcp/terminal', closest: 'mcp' });
    expect(unknownSlash('/mcp/terminal turn it on', names)).toEqual({ word: 'mcp/terminal', closest: 'mcp' });
    expect(unknownSlashLine({ word: 'mcp/terminal', closest: 'mcp' })).toBe('No such command: /mcp/terminal. Did you mean /mcp? ↵ sends nothing until you change it.');
  });

  it('alone on the line is a mistyped command too; in front of more words it is a path', () => {
    expect(unknownSlash('/usr/bin/env', names)).toEqual({ word: 'usr/bin/env', closest: null });
    expect(unknownSlash('/usr/bin/env is missing', names)).toBeNull();
    expect(unknownSlash('/etc/hosts\nhas a typo', names)).toBeNull();
  });
});

describe('the closest action of a typo', () => {
  const names = ['new', 'issue', 'pr', 'stop', 'ask', 'plan', 'auto', 'direct', 'edits', 'model', 'compact', 'context', 'diff', 'undo', 'terminal', 'mcp', 'clear', 'closed', 'settings', 'shortcuts'];
  const closest = (word: string) => unknownSlash(`/${word} please`, names)?.closest;

  it('allows one typo up to four letters, two in a longer word, never more than half the word', () => {
    expect(closest('x')).toBeNull();
    expect(closest('exit')).toBeNull();
    expect(closest('stpo')).toBe('stop');
    expect(closest('modle')).toBe('model');
    expect(closest('compat')).toBe('compact');
    expect(closest('cmpat')).toBe('compact');
  });
});

describe('the pointer over a menu that opens', () => {
  it('a menu that opens under a resting pointer keeps its first row', () => {
    const p = new MenuPointer();
    p.opened();
    expect(p.picks({ x: 10, y: 20 })).toBe(false);
    expect(p.picks({ x: 10, y: 20 })).toBe(false);
  });

  it('a list that scrolls under a resting pointer keeps the row of the keys', () => {
    const p = new MenuPointer();
    p.opened();
    p.picks({ x: 10, y: 20 });
    expect(p.picks({ x: 10, y: 20 })).toBe(false);
    // Opened again where the pointer was seen last time, after it moved meanwhile: still no pick.
    p.picks({ x: 40, y: 90 });
    p.opened();
    expect(p.picks({ x: 10, y: 20 })).toBe(false);
  });

  it('a real move picks the row under it', () => {
    const p = new MenuPointer();
    p.opened();
    p.picks({ x: 10, y: 20 });
    expect(p.picks({ x: 10, y: 24 })).toBe(true);
  });
});

describe('the row under the pointer', () => {
  it('becomes the active one only when the pointer moved', () => {
    expect(pointerMoved(null, { x: 10, y: 20 })).toBe(false);
    expect(pointerMoved({ x: 10, y: 20 }, { x: 10, y: 20 })).toBe(false); // the list scrolled under it
    expect(pointerMoved({ x: 10, y: 20 }, { x: 10, y: 24 })).toBe(true);
  });
});
