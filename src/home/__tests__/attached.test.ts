import { describe, expect, it } from 'vitest';
import { messageWithFiles } from '../attached';

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
