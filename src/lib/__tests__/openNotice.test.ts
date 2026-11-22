import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Agent } from '../gen/Agent';
import type { Approval } from '../gen/Approval';
import type { Office } from '../gen/Office';
import type { Plan } from '../gen/Plan';
import type { Workspace } from '../gen/Workspace';
import { openWorkspace } from '../state/actions';
import { blockers } from '../state/blockers.svelte';
import { offices } from '../state/offices.svelte';
import { openNotice } from '../state/openNotice';
import { store } from '../state/store.svelte';
import { ui } from '../state/ui.svelte';

// Opening another office is the real action's job (a snapshot, its memories): here it only switches.
vi.mock('../state/actions', async (original) => ({ ...(await original<typeof import('../state/actions')>()), openWorkspace: vi.fn() }));

const office = (id: string) => ({ id, name: id, repo_path: `/code/${id}` }) as Workspace;
const other = (id: string) => ({ id, name: id, repo_path: `/code/${id}`, crew: 1, working: 1, needs_you: 1, home: [] }) satisfies Office;

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
    expect(ui.settingsTab).toBe('safety');
    expect(openWorkspace).not.toHaveBeenCalled();
  });

  it('what is in another office opens that office first', async () => {
    offices.others = [other('b')];
    await openNotice({ workspace_id: 'b', kind: 'task', id: 't7' });
    expect(openWorkspace).toHaveBeenCalledWith('/code/b');
    expect(store.workspace?.id).toBe('b');
    expect(ui.selection).toMatchObject({ kind: 'task', id: 't7' });
  });

  it('an office that is gone, or that did not open, leaves the screen as it is', async () => {
    await openNotice({ workspace_id: 'gone', kind: 'task', id: 't7' });
    expect(openWorkspace).not.toHaveBeenCalled();
    expect(ui.selection).toBeNull();
    offices.others = [other('b')];
    vi.mocked(openWorkspace).mockImplementation(async () => {}); // the core refused: its toast says why
    await openNotice({ workspace_id: 'b', kind: 'task', id: 't7' });
    expect(store.workspace?.id).toBe('a');
    expect(ui.selection).toBeNull();
  });
});
