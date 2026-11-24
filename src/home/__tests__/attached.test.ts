import { describe, expect, it } from 'vitest';
import { lineCount, messageWithFiles, PASTE_MOST_CHARS, PASTE_MOST_LINES, pastedChip, pastedName, pastesAsFile } from '../attached';

describe('messageWithFiles', () => {
  it('names each attached file on a "- <path>" line: the core finds the images by these lines', () => {
    const paths = ['/repo/.codly/attachments/a1/shot.png', '/repo/.codly/attachments/b2/my notes.txt'];
    expect(messageWithFiles('  Make it look like this. ', paths)).toBe(
      'Make it look like this.\n\n' +
        'Attached files, saved at these paths (an image may already be in this message):\n' +
        '- /repo/.codly/attachments/a1/shot.png\n' +
        '- /repo/.codly/attachments/b2/my notes.txt',
    );
  });

  it('does not tell the agent to read the files: an image is already in the message', () => {
    expect(messageWithFiles('look', ['/repo/.codly/attachments/a1/shot.png'])).not.toMatch(/\bread\b/i);
  });

  it('files alone still make a message', () => {
    expect(messageWithFiles('', ['/repo/.codly/attachments/a1/shot.png'])).toBe(
      'See the attached files.\n\n' +
        'Attached files, saved at these paths (an image may already be in this message):\n' +
        '- /repo/.codly/attachments/a1/shot.png',
    );
  });

  it('text alone is sent as it is, trimmed', () => {
    expect(messageWithFiles(' hello ', [])).toBe('hello');
    expect(messageWithFiles('', [])).toBe('');
  });
});

describe('a long paste', () => {
  it('over 8,000 characters or 150 lines goes in as a text file', () => {
    expect(pastesAsFile('x'.repeat(PASTE_MOST_CHARS + 1))).toBe(true);
    expect(pastesAsFile(Array.from({ length: PASTE_MOST_LINES + 1 }, (_, i) => `line ${i}`).join('\n'))).toBe(true);
    expect([PASTE_MOST_CHARS, PASTE_MOST_LINES]).toEqual([8000, 150]);
  });

  it('a shorter one is pasted into the field as before', () => {
    expect(pastesAsFile('x'.repeat(PASTE_MOST_CHARS))).toBe(false);
    expect(pastesAsFile(`${'a\n'.repeat(PASTE_MOST_LINES - 1)}a\n`)).toBe(false);
    expect(pastesAsFile('')).toBe(false);
  });

  it('counts lines as an editor does: a break at the very end starts none', () => {
    expect(lineCount('one')).toBe(1);
    expect(lineCount('one\ntwo\n')).toBe(2);
    expect(lineCount('one\r\ntwo\rthree')).toBe(3);
    expect(lineCount('')).toBe(1);
  });

  it('its file is pasted-<n>.txt, and its chip says what it is', () => {
    expect(pastedName()).toMatch(/^pasted-\d+\.txt$/);
    expect(pastedName()).not.toBe(pastedName());
    expect(pastedChip(212)).toBe('Pasted text · 212 lines');
    expect(pastedChip(1)).toBe('Pasted text · 1 line');
  });
});
