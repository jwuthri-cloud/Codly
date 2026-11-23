// Settings → Safety → "Always allowed in this repo": how a saved rule reads. A rule is for a
// command (the exact line, or lines that start with some words) or for a tool of an MCP server,
// which the core saves by the tool's full name, `mcp__<server>__<tool>`, as an exact rule
// (orchestrator/approvals.rs, `rule_offer`: a command line of that shape is never saved).

import type { ApprovalRule } from '../gen/ApprovalRule';

/** One row of the list: the kind in the first column, then the text (`code`: a command line,
 *  shown as code), and under it what the rule covers when the text does not say it. */
export type RuleWords = { kind: string; text: string; code: boolean; note: string | null };

const TOOL_NOTE = 'Every use of this tool, whatever its input';

export function ruleWords(rule: Pick<ApprovalRule, 'kind' | 'command'>): RuleWords {
  const name = rule.command;
  // The server's name ends at the first "__", as the core reads it (`mcp_server`).
  const rest = rule.kind === 'exact' && name.startsWith('mcp__') ? name.slice('mcp__'.length) : '';
  const cut = rest.indexOf('__');
  if (cut <= 0) return { kind: rule.kind === 'prefix' ? 'Starts with' : 'Exactly', text: name, code: true, note: null };
  const server = rest.slice(0, cut);
  const tool = rest.slice(cut + 2);
  // A server's name may hold "__" too: such a name reads two ways, so it is shown whole.
  if (!tool || tool.includes('__')) return { kind: 'Tool', text: `The MCP tool ${name}`, code: false, note: TOOL_NOTE };
  return { kind: 'Tool', text: `The tool ${tool} of the MCP server ${server}`, code: false, note: TOOL_NOTE };
}
