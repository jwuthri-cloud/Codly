import { describe, expect, it } from 'vitest';
import { ruleWords } from '../approvalRules';

describe('a saved rule in Settings → Safety', () => {
  it('a command rule reads as before: the exact line, or what the line starts with', () => {
    expect(ruleWords({ kind: 'exact', command: 'npm run build' })).toEqual({ kind: 'Exactly', text: 'npm run build', code: true, note: null });
    expect(ruleWords({ kind: 'prefix', command: 'git commit' })).toEqual({ kind: 'Starts with', text: 'git commit', code: true, note: null });
  });

  it('a rule for a tool of an MCP server names the tool and its server, and says that the input does not matter', () => {
    expect(ruleWords({ kind: 'exact', command: 'mcp__linear__create_issue' })).toEqual({
      kind: 'Tool',
      text: 'The tool create_issue of the MCP server linear',
      code: false,
      note: 'Every use of this tool, whatever its input',
    });
    expect(ruleWords({ kind: 'exact', command: 'mcp__my-docs__search' }).text).toBe('The tool search of the MCP server my-docs');
  });

  it('a name that reads two ways is shown whole', () => {
    // `mcp__github__x__tool`: the tool `x__tool` of `github`, or `tool` of a server named `github__x`.
    expect(ruleWords({ kind: 'exact', command: 'mcp__github__x__tool' })).toEqual({
      kind: 'Tool',
      text: 'The MCP tool mcp__github__x__tool',
      code: false,
      note: 'Every use of this tool, whatever its input',
    });
    expect(ruleWords({ kind: 'exact', command: 'mcp__linear__' }).text).toBe('The MCP tool mcp__linear__');
  });

  it('only a full tool name is a tool rule', () => {
    for (const command of ['mcp__', 'mcp____tool', 'mcp__linear', 'mcp_linear__create_issue', 'echo mcp__linear__create_issue']) {
      expect(ruleWords({ kind: 'exact', command }), command).toEqual({ kind: 'Exactly', text: command, code: true, note: null });
    }
    // The core saves a tool as an exact rule only.
    expect(ruleWords({ kind: 'prefix', command: 'mcp__linear__create_issue' }).kind).toBe('Starts with');
  });
});
