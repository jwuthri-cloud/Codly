// Where a focus on the agent sheet lands. Before, "/undo" ("Its sheet opens: put the files back
// under Git, at Checkpoints") opened the sheet on the Messages tab: the open tab was the tabs'
// own state, and the focus only scrolled to the section of the tabs.
import { describe, expect, it } from 'vitest';
import { focusPlace } from '../sheetFocus';

describe('a focus on the agent sheet', () => {
  it('“/undo” lands on the Git tab’s checkpoints with no extra click', () => {
    // The composer's "/undo" asks for the tabs (src/home/Composer.svelte, `undo`).
    expect(focusPlace('tabs')).toEqual({ section: 'tabs', tab: 'git', inner: 'checkpoints' });
    expect(focusPlace('checkpoints')).toEqual({ section: 'tabs', tab: 'git', inner: 'checkpoints' });
  });

  it('a section of the sheet itself opens no tab', () => {
    for (const section of ['mcp', 'subagents', 'trust', 'cost']) expect(focusPlace(section)).toEqual({ section, tab: null, inner: null });
  });
});
