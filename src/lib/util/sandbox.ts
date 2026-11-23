// Claude Code's shell sandbox asks with a request of its own when a command wants to reach a
// host that the sandbox does not allow yet. The name of that request ("SandboxNetworkAccess")
// says nothing to a user: what shows an approval asks here, and says the host in plain words.

import type { ApprovalSubject } from '../gen/ApprovalSubject';
import { clean, truncate } from './sanitize';

/** The tool name of that request (crates/codly-core/src/adapters/claude.rs, `SANDBOX_NETWORK`). */
const SANDBOX_NETWORK = 'SandboxNetworkAccess';

/**
 * The host that a sandboxed command wants to reach, when `s` is the sandbox's own request
 * ('' when the request names no host). null: `s` is another request. The host is whole: the
 * end of a host name says whose it is, so a cut at the end can show another owner's name.
 */
export function sandboxHost(s: ApprovalSubject): string | null {
  if (s.kind !== 'tool' || s.name !== SANDBOX_NETWORK) return null;
  const host = (s.input as { host?: unknown } | null)?.host;
  return typeof host === 'string' ? clean(host).replace(/\s+/g, '') : '';
}

/**
 * That request after "<agent> wants to": "reach example.com". null: `s` is another request.
 * `max` cuts the host to that many characters, for a place with one short line (a headline,
 * a toast, a bubble) next to which the whole host can be read; without it the host is whole.
 */
export function sandboxWants(s: ApprovalSubject, max?: number): string | null {
  const host = sandboxHost(s);
  if (host === null) return null;
  if (!host) return 'use the network';
  return `reach ${max === undefined ? host : truncate(host, max)}`;
}
