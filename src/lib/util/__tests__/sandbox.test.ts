import { describe, expect, it } from 'vitest';
import type { ApprovalSubject } from '../../gen/ApprovalSubject';
import type { JsonValue } from '../../gen/serde_json/JsonValue';
import { HOST_ALLOW_LASTS, sandboxHost, sandboxWants } from '../sandbox';

/** The subject the core makes of the sandbox's request for a host (input as recorded: `{ host }`). */
const hostRequest = (input: JsonValue): ApprovalSubject => ({ kind: 'tool', name: 'SandboxNetworkAccess', input });

describe('a command in Claude Code’s sandbox wants a host', () => {
  it('is said with the host, never with the name of the tool', () => {
    const s = hostRequest({ host: 'example.com' });
    expect(sandboxHost(s, 'claude')).toBe('example.com');
    expect(sandboxWants(s, 'claude')).toBe('reach example.com');
  });

  it('is Claude Code’s own request: from another CLI, or an agent Codly does not know, it is a tool like any other', () => {
    // For the other CLIs the name of a tool request is the title of the call: any text can be there.
    const s = hostRequest({ host: 'example.com' });
    for (const provider of ['codex', 'gemini', 'opencode', 'cursor', 'copilot', null, undefined] as const) {
      expect(sandboxHost(s, provider), String(provider)).toBeNull();
      expect(sandboxWants(s, provider), String(provider)).toBeNull();
      expect(sandboxWants(s, provider, 80), String(provider)).toBeNull();
    }
  });

  it('is still said in plain words when the request names no host', () => {
    for (const input of [{}, null, { host: 7 }, { host: '' }] as JsonValue[]) {
      expect(sandboxHost(hostRequest(input), 'claude')).toBe('');
      expect(sandboxWants(hostRequest(input), 'claude')).toBe('use the network');
    }
  });

  it('shows the host without characters that hide or reorder text', () => {
    expect(sandboxHost(hostRequest({ host: 'moc.elpmaxe‮.evil.example\n' }), 'claude')).toBe('moc.elpmaxe.evil.example');
  });

  it('keeps a long host whole where it is read, and cuts it only for a short line', () => {
    // The end of a host name says whose it is: a cut there would show "github.com" alone.
    const host = `github.com.${'a'.repeat(96)}.evil.example`;
    expect(host).toHaveLength(120);
    const s = hostRequest({ host });
    expect(sandboxHost(s, 'claude')).toBe(host);
    expect(sandboxWants(s, 'claude')).toBe(`reach ${host}`);
    expect(sandboxWants(s, 'claude', 80)).toBe(`reach ${host.slice(0, 79)}…`);
    expect(sandboxWants(hostRequest({ host: 'example.com' }), 'claude', 80)).toBe('reach example.com');
    // No space in a host: text after one would read as words of the sentence.
    expect(sandboxHost(hostRequest({ host: 'example.com and\tevil.example' }), 'claude')).toBe('example.comandevil.example');
  });

  it('is no other request', () => {
    const others: ApprovalSubject[] = [
      { kind: 'tool', name: 'WebFetch', input: { host: 'example.com' } },
      { kind: 'command', command: 'curl https://example.com', cwd: null },
      { kind: 'permissions', detail: {} },
    ];
    for (const s of others) {
      expect(sandboxHost(s, 'claude')).toBeNull();
      expect(sandboxWants(s, 'claude')).toBeNull();
    }
  });

  it('says how long an allow lasts without a promise that nothing recorded backs', () => {
    expect(HOST_ALLOW_LASTS).toBe('Approve once lets this command connect. Claude Code can keep the host allowed until this agent’s session ends, so it may not ask again for it.');
  });
});
