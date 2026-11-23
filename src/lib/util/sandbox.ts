// Claude Code's shell sandbox asks with a request of its own when a command wants to reach a
// host that the sandbox does not allow yet. The name of that request ("SandboxNetworkAccess")
// says nothing to a user: what shows an approval asks here, and says the host in plain words.

import type { ApprovalSubject } from '../gen/ApprovalSubject';
import { oneLine } from './sanitize';

/** The tool name of that request (crates/codly-core/src/adapters/claude.rs, `SANDBOX_NETWORK`). */
const SANDBOX_NETWORK = 'SandboxNetworkAccess';

/**
 * The host that a sandboxed command wants to reach, when `s` is the sandbox's own request
 * ('' when the request names no host). null: `s` is another request.
 */
export function sandboxHost(s: ApprovalSubject): string | null {
  if (s.kind !== 'tool' || s.name !== SANDBOX_NETWORK) return null;
  const host = (s.input as { host?: unknown } | null)?.host;
  return typeof host === 'string' ? oneLine(host, 80) : '';
}

/** That request after "<agent> wants to": "reach example.com". null: `s` is another request. */
export function sandboxWants(s: ApprovalSubject): string | null {
  const host = sandboxHost(s);
  if (host === null) return null;
  return host ? `reach ${host}` : 'use the network';
}
