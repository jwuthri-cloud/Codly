// Claude Code's shell sandbox asks with a request of its own when a command wants to reach a
// host that the sandbox does not allow yet. The name of that request ("SandboxNetworkAccess")
// says nothing to a user: what shows an approval asks here, and says the host in plain words.
// Only for a Claude agent: for every other CLI the name of a tool request is the title of the
// call, so a request with this name is shown as what it is, by its name and its input. An agent
// Codly does not know counts as not Claude.

import type { ApprovalSubject } from '../gen/ApprovalSubject';
import type { Provider } from '../gen/Provider';
import { clean, truncate } from './sanitize';

/** The tool name of that request (crates/codly-core/src/adapters/claude.rs, `SANDBOX_NETWORK`). */
const SANDBOX_NETWORK = 'SandboxNetworkAccess';

/**
 * How long an allow lasts, on the request's card and in Settings → Safety. As recorded on
 * Claude Code 2.1.311 (docs/research/cli-claude-stream-json-2.1.311-part3.md, section 6), each
 * case once (n=1):
 * - after a plain allow the same process did not ask again for the host 2 s later
 *   (`r5-sandbox-network-once`) and 304 s later (`r5-sandbox-network-once-later`);
 * - a new process that resumed the session asked again (`r5-sandbox-network-once-restart-a`, `-b`).
 * A wait of more than 5 minutes inside one process is not recorded.
 */
export const HOST_ALLOW_LASTS = 'Approve once lets this command connect. Claude Code then does not ask again for this host while this agent keeps running. When the agent starts again (Codly restarts, or a change that restarts the agent) it asks again.';

/**
 * The host that a sandboxed command wants to reach, when `s` is the sandbox's own request
 * ('' when the request names no host). null: `s` is another request, or `provider` (the CLI of
 * the agent that asks) is not Claude. The host is whole: the end of a host name says whose it
 * is, so a cut at the end can show another owner's name.
 */
export function sandboxHost(s: ApprovalSubject, provider: Provider | null | undefined): string | null {
  if (provider !== 'claude' || s.kind !== 'tool' || s.name !== SANDBOX_NETWORK) return null;
  const host = (s.input as { host?: unknown } | null)?.host;
  return typeof host === 'string' ? clean(host).replace(/\s+/g, '') : '';
}

/**
 * That request after "<agent> wants to": "reach example.com". null: `s` is another request, or
 * the agent's CLI is not Claude. `max` cuts the host to that many characters, for a place with
 * one short line (a headline, a toast, a bubble) next to which the whole host can be read;
 * without it the host is whole.
 */
export function sandboxWants(s: ApprovalSubject, provider: Provider | null | undefined, max?: number): string | null {
  const host = sandboxHost(s, provider);
  if (host === null) return null;
  if (!host) return 'use the network';
  return `reach ${max === undefined ? host : truncate(host, max)}`;
}
