// Where a focus on the agent sheet lands. Before, "/undo" ("Its sheet opens: put the files back
// under Git, at Checkpoints") opened the sheet on the Messages tab: the open tab was the tabs'
// own state, and the focus only scrolled to the section of the tabs.
import { describe, expect, it } from 'vitest';
import { focusPlace, tabOf, withTab } from '../sheetFocus';

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

// Round 7's review: the open tab was one value for every agent. After "/undo" or one click on
// Git, every agent's sheet you opened mounted the Git tab and ran its git commands at once.
describe('the open tab of an agent’s sheet', () => {
  it('is the agent’s own: another agent’s sheet opens on Messages', () => {
    const tabs = withTab({}, 'ada', 'git');
    expect(tabOf(tabs, 'ada')).toBe('git');
    expect(tabOf(tabs, 'bo')).toBe('messages');
  });

  it('a tab you pick for one agent leaves the others as they were', () => {
    const tabs = withTab(withTab({}, 'ada', 'git'), 'bo', 'traces');
    expect(tabs).toEqual({ ada: 'git', bo: 'traces' });
    expect(withTab(tabs, 'ada', 'messages')).toEqual({ ada: 'messages', bo: 'traces' });
  });

  it('a focus that names a tab opens it for the agent of the sheet', () => {
    const at = focusPlace('checkpoints');
    expect(tabOf(withTab({}, 'ada', at.tab!), 'ada')).toBe('git');
  });
});
