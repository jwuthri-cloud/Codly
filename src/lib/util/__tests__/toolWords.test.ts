// A tool of an MCP server in a sentence. Before, History printed the raw name
// ("mcp__codly__codly_submit_plan").
import { describe, expect, it } from 'vitest';
import { mcpToolOf, toolInWords } from '../toolWords';

const target = (server: string | null, tool: string | null) => ({ server, tool });

describe('a tool in words', () => {
  it('reads the server and the tool from the call’s target, else from the name', () => {
    expect(mcpToolOf('anything', target('linear', 'create_issue'))).toEqual({ server: 'linear', tool: 'create_issue' });
    expect(mcpToolOf('mcp__linear__create_issue')).toEqual({ server: 'linear', tool: 'create_issue' });
    // The server's name ends at the first "__", as the core reads it.
    expect(mcpToolOf('mcp__a__b__c')).toEqual({ server: 'a', tool: 'b__c' });
    expect(mcpToolOf('Bash', target(null, null))).toBeNull();
    expect(mcpToolOf('mcp__broken')).toBeNull();
    expect(mcpToolOf('mcp____x')).toBeNull();
  });

  it('names Codly’s own tools by what they are, another server’s by server and name, a CLI’s own by its name', () => {
    expect(toolInWords('mcp__codly__codly_submit_plan')).toBe('Codly’s submit plan tool');
    expect(toolInWords('mcp__linear__create_issue')).toBe('linear: create_issue');
    expect(toolInWords('WebFetch')).toBe('WebFetch');
  });
});
