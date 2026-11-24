// Where a focus on the agent sheet lands (`ui.sheetFocus`, set by the screen that opens the
// sheet at one of its places). Most places are sections of the sheet. Some are inside one of
// its tabs (Messages, Traces, Git: AgentTabs.svelte): that tab opens first, then the sheet
// scrolls to the place in it. Pure, so the sheet and the tests agree.

/** The tabs under the agent sheet's sections, as `ui.sheetTab` holds the open one of each agent. */
export type SheetTab = 'messages' | 'traces' | 'git';

/**
 * The open tab of each agent's sheet, for the session, by agent id. The tab is one agent's:
 * Git runs its git commands while it is open, so another agent's sheet does not open on it.
 */
export type SheetTabs = Readonly<Record<string, SheetTab>>;

/** The tab an agent's sheet opens on: the one you left it on, else Messages. */
export const tabOf = (tabs: SheetTabs, agentId: string): SheetTab => tabs[agentId] ?? 'messages';

/** The tabs with `tab` open for `agentId`; the other agents keep theirs. */
export const withTab = (tabs: SheetTabs, agentId: string, tab: SheetTab): SheetTabs => ({ ...tabs, [agentId]: tab });

/** `section`: the sheet's section to open and scroll to; `tab`: the tab to open in it first; `inner`: the place in that tab to scroll to then. */
export type FocusPlace = { section: string; tab: SheetTab | null; inner: string | null };

/** The section that holds the tabs. */
const TABS_SECTION = 'tabs';

/** The places inside a tab: the tab, and the `data-section` of the place. */
const IN_TAB: Record<string, { tab: SheetTab; inner: string }> = {
  checkpoints: { tab: 'git', inner: 'checkpoints' },
  // The composer's "/undo" ("put the files back under Git, at Checkpoints") asks for the tabs
  // (src/home/Composer.svelte, `undo`): it means the checkpoints.
  [TABS_SECTION]: { tab: 'git', inner: 'checkpoints' },
};

export function focusPlace(focus: string): FocusPlace {
  const inTab = IN_TAB[focus];
  return inTab ? { section: TABS_SECTION, ...inTab } : { section: focus, tab: null, inner: null };
}

/** How often, and how many times, the sheet looks for a place inside a tab that is still reading what it shows (3 s in all). */
export const FOCUS_LOOK_MS = 100;
export const FOCUS_LOOK_TRIES = 30;
