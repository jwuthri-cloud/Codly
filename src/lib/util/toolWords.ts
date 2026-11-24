// How a tool of an MCP server reads in a sentence. Claude Code names one `mcp__<server>__<tool>`;
// the adapters give its server and its tool apart in a call's target (`Target.server`,
// `Target.tool`). Codly's own server is `codly`: its tools are named by what they do, never by
// their raw names.
import type { Target } from '../gen/Target';

/** Codly's own MCP server, in every run of an agent. */
export const CODLY_SERVER = 'codly';

export type McpTool = { server: string; tool: string };

/**
 * The server and the tool of an MCP tool: from the call's target when the adapter gave them,
 * else from a name `mcp__<server>__<tool>` (the server's name ends at the first "__", as the
 * core reads it). Null for a CLI's own tool ("Bash", "WebFetch").
 */
export function mcpToolOf(name: string, target: Pick<Target, 'server' | 'tool'> | null = null): McpTool | null {
  if (target?.server && target.tool) return { server: target.server, tool: target.tool };
  if (!name.startsWith('mcp__')) return null;
  const rest = name.slice('mcp__'.length);
  const cut = rest.indexOf('__');
  const tool = cut > 0 ? rest.slice(cut + 2) : '';
  return tool ? { server: rest.slice(0, cut), tool } : null;
}

/**
 * A tool, as a sentence names it: "linear: create_issue" for a tool of an MCP server, "Codly’s
 * submit plan tool" for one of Codly's own, and a CLI's own tool by its name ("WebFetch").
 */
export function toolInWords(name: string, target: Pick<Target, 'server' | 'tool'> | null = null): string {
  const t = mcpToolOf(name, target);
  if (!t) return name;
  if (t.server === CODLY_SERVER) return `Codly’s ${t.tool.replace(/^codly_/, '').replace(/_/g, ' ')} tool`;
  return `${t.server}: ${t.tool}`;
}
