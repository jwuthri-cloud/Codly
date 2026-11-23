import { describe, expect, it } from 'vitest';
import type { ApprovalSubject } from '../../gen/ApprovalSubject';
import type { JsonValue } from '../../gen/serde_json/JsonValue';
import { sandboxHost, sandboxWants } from '../sandbox';

/** The subject the core makes of the sandbox's request for a host (input as recorded: `{ host }`). */
const hostRequest = (input: JsonValue): ApprovalSubject => ({ kind: 'tool', name: 'SandboxNetworkAccess', input });

describe('a command in Claude Code’s sandbox wants a host', () => {
  it('is said with the host, never with the name of the tool', () => {
    const s = hostRequest({ host: 'example.com' });
    expect(sandboxHost(s)).toBe('example.com');
    expect(sandboxWants(s)).toBe('reach example.com');
  });

  it('is still said in plain words when the request names no host', () => {
    for (const input of [{}, null, { host: 7 }, { host: '' }] as JsonValue[]) {
      expect(sandboxHost(hostRequest(input))).toBe('');
      expect(sandboxWants(hostRequest(input))).toBe('use the network');
    }
  });

  it('shows the host without characters that hide or reorder text', () => {
    expect(sandboxHost(hostRequest({ host: 'moc.elpmaxe‮.evil.example\n' }))).toBe('moc.elpmaxe.evil.example');
  });

  it('is no other request', () => {
    const others: ApprovalSubject[] = [
      { kind: 'tool', name: 'WebFetch', input: { host: 'example.com' } },
      { kind: 'command', command: 'curl https://example.com', cwd: null },
      { kind: 'permissions', detail: {} },
    ];
    for (const s of others) {
      expect(sandboxHost(s)).toBeNull();
      expect(sandboxWants(s)).toBeNull();
    }
  });
});
