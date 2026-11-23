import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { Office } from '../gen/Office';
import type { Plan } from '../gen/Plan';
import type { Workspace } from '../gen/Workspace';
import { followLinks } from '../../overlays/settingsLink';
import type { SettingsTarget } from '../../overlays/settingsPages';
import { openWorkspace } from '../state/actions';
import { blockers } from '../state/blockers.svelte';
import { offices } from '../state/offices.svelte';
import { openNotice } from '../state/openNotice';
import { store } from '../state/store.svelte';
import { toast, toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';

// Opening another office is the real action's job (a snapshot, its memories): here it only switches.
vi.mock('../state/actions', async (original) => ({ ...(await original<typeof import('../state/actions')>()), openWorkspace: vi.fn() }));

const office = (id: string) => ({ id, name: id, repo_path: `/code/${id}` }) as Workspace;
const other = (id: string) => ({ id, name: id, repo_path: `/code/${id}`, crew: 1, working: 1, needs_you: 1, home: [] }) satisfies Office;
const said = () => toasts.list.map((t) => [t.kind, t.title, t.action?.label]);

describe('opening what a notification names', () => {
  beforeEach(() => {
    store.setWorkspace(office('a'));
    vi.spyOn(offices, 'refresh').mockResolvedValue();
    vi.mocked(openWorkspace).mockImplementation(async (path) => store.setWorkspace(office(path!.replace('/code/', ''))));
  });
  // The store and the UI state are singletons: each test puts back what it set.
  afterEach(() => {
    vi.restoreAllMocks();
    vi.mocked(openWorkspace).mockReset();
    for (const t of toasts.list) toasts.dismiss(t.id);
    store.setWorkspace(null);
    store.agents = [];
    store.plans = [];
    store.approvals = [];
    offices.others = [];
    ui.selection = null;
    ui.revealApproval = null;
    ui.settingsOpen = false;
    ui.settingsTab = null;
    blockers.panel = false;
    ui.openRequest('lead');
  });

  it('an approval opens the conversation that shows it, on its card', async () => {
    store.agents = [{ id: 'solo1', role: 'solo', task_id: null, thread: null } as Agent];
    store.approvals = [{ id: 'ap1', agent_id: 'solo1', task_id: null } as Approval];
    await openNotice({ workspace_id: 'a', kind: 'approval', id: 'ap1', agent_id: 'solo1' });
    expect(ui.thread).toBe('agent:solo1');
    expect(ui.revealApproval).toBe('ap1');
    expect(openWorkspace).not.toHaveBeenCalled();
  });

  it('an approval the store does not hold opens its agent’s conversation', async () => {
    store.agents = [{ id: 'lead2', role: 'lead', task_id: null, thread: 'lead:x' } as Agent];
    await openNotice({ workspace_id: 'a', kind: 'approval', id: 'ap9', agent_id: 'lead2' });
    expect(ui.thread).toBe('lead:x');
    expect(ui.revealApproval).toBe('ap9');
  });

  it('a plan opens the request it answers', async () => {
    store.plans = [{ id: 'p1', thread: 'lead:x', status: 'done' } as Plan];
    await openNotice({ workspace_id: 'a', kind: 'plan', id: 'p1' });
    expect(ui.request).toBe('lead:x');
    expect(ui.thread).toBe('lead:x');
    await openNotice({ workspace_id: 'a', kind: 'plan', id: 'gone' });
    expect(ui.request).toBe('lead');
  });

  it('a task and an agent open their sheet', async () => {
    await openNotice({ workspace_id: 'a', kind: 'task', id: 't1' });
    expect(ui.selection).toMatchObject({ kind: 'task', id: 't1' });
    await openNotice({ workspace_id: 'a', kind: 'agent', id: 'w1' });
    expect(ui.selection).toMatchObject({ kind: 'agent', id: 'w1' });
  });

  it('a question opens the questions that wait for you', async () => {
    await openNotice({ workspace_id: 'a', kind: 'questions' });
    expect(blockers.panel).toBe(true);
  });

  it('a budget opens Settings on the budgets', async () => {
    await openNotice({ workspace_id: null, kind: 'budgets' });
    expect(ui.settingsOpen).toBe(true);
    // At the budgets, not at the top of the tab: the same way as the toast and the mark in the top bar.
    expect(ui.settingsTab).toBe('safety/budgets');
    expect(openWorkspace).not.toHaveBeenCalled();
  });

  it('a budget clicked while Settings is open moves the open sheet to the budgets', async () => {
    ui.settingsOpen = true;
    const went: SettingsTarget[] = [];
    const stop = followLinks((to) => went.push(to));
    await openNotice({ workspace_id: null, kind: 'budgets' });
    stop();
    expect(went).toEqual([{ tab: 'safety', page: 'limits', section: 'budgets' }]);
    // Nothing is left for the next time Settings opens: that one opens on General again.
    expect(ui.settingsTab).toBeNull();
  });

  // The click is a guess (Codly came to the front while the notification showed): it must not
  // take you out of the office you work in.
  it('what is in another office stays there: a toast offers the way', async () => {
    offices.others = [other('b')];
    ui.selectAgent('w1'); // your place in office a
    await openNotice({ workspace_id: 'b', kind: 'task', id: 't7' });
    expect(openWorkspace).not.toHaveBeenCalled();
    expect(store.workspace?.id).toBe('a');
    expect(ui.selection).toMatchObject({ kind: 'agent', id: 'w1' });
    expect(said()).toEqual([['attention', 'b: a task stopped', 'Review']]);
  });

  it('Review on that toast opens the office, then what the notification names', async () => {
    offices.others = [other('b')];
    await openNotice({ workspace_id: 'b', kind: 'task', id: 't7' });
    await toasts.list[0].action!.run();
    expect(openWorkspace).toHaveBeenCalledWith('/code/b');
    expect(store.workspace?.id).toBe('b');
    expect(ui.selection).toMatchObject({ kind: 'task', id: 't7' });
  });

  it('the toast says what kind of thing waits in the other office', async () => {
    offices.others = [other('b'), other('c'), other('d'), other('e')];
    const titles: string[] = [];
    for (const [ws, t] of [
      ['b', { kind: 'approval', id: 'ap1', agent_id: 'w1' }],
      ['c', { kind: 'questions' }],
      ['d', { kind: 'plan', id: 'p1' }],
      ['e', { kind: 'agent', id: 'w1' }],
    ] as const) {
      await openNotice({ workspace_id: ws, ...t });
      titles.push(toasts.list[toasts.list.length - 1].title);
    }
    expect(titles).toEqual(['b: an approval waits for you', 'c: a question waits for you', 'd: a plan needs a look', 'e: an agent needs a look']);
    expect(openWorkspace).not.toHaveBeenCalled();
  });

  it('an office a toast already points at gets no second toast', async () => {
    offices.others = [other('b')];
    // What waits for you in another office has its toast already (state/notify.ts).
    toast({ kind: 'attention', title: 'b: An agent needs your approval', action: { label: 'Review', run: () => {} } });
    await openNotice({ workspace_id: 'b', kind: 'approval', id: 'ap1', agent_id: 'w1' });
    await openNotice({ workspace_id: 'b', kind: 'task', id: 't7' });
    expect(said()).toEqual([['attention', 'b: An agent needs your approval', 'Review']]);
  });

  it('an office that is gone has no toast, and one that did not open leaves the screen as it is', async () => {
    await openNotice({ workspace_id: 'gone', kind: 'task', id: 't7' });
    expect(said()).toEqual([]);
    offices.others = [other('b')];
    vi.mocked(openWorkspace).mockImplementation(async () => {}); // the core refused: its toast says why
    await openNotice({ workspace_id: 'b', kind: 'task', id: 't7' });
    await toasts.list[0].action!.run();
    expect(store.workspace?.id).toBe('a');
    expect(ui.selection).toBeNull();
  });
});
