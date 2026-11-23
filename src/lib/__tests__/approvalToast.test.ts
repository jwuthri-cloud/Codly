import { describe, expect, it } from 'vitest';
import { asks } from '../state/notify';

describe('the one line of a toast that says what a request asks', () => {
  it('shows a command by itself and points a question to the conversation', () => {
    expect(asks({ kind: 'command', command: 'rm  -rf\ndist', cwd: null }, 'claude')).toBe('rm -rf dist');
    expect(asks({ kind: 'question', questions: [] }, 'codex')).toBe('Answer it in the conversation');
  });

  it('names the tool of a tool request', () => {
    expect(asks({ kind: 'tool', name: 'WebFetch', input: { url: 'https://example.com' } }, 'claude')).toBe('Wants to use WebFetch');
    expect(asks({ kind: 'tool', name: 'Fetch a page', input: null }, 'gemini')).toBe('Wants to use Fetch a page');
  });

  it('says the host that a Claude agent’s sandboxed command wants, and for no other CLI', () => {
    const host = { kind: 'tool', name: 'SandboxNetworkAccess', input: { host: 'example.com' } } as const;
    expect(asks(host, 'claude')).toBe('Wants to reach example.com');
    expect(asks(host, 'codex')).toBe('Wants to use SandboxNetworkAccess');
    expect(asks(host, undefined)).toBe('Wants to use SandboxNetworkAccess');
  });
});
