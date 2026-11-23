// The sheet and the chat never show two different agents: an agent's sheet that is open follows
// the 1:1 chat you open, so what the sheet does (Block tools, Stop, Kill) acts on the agent on screen.
// A Lead's sheet follows the request you switch to in the same way.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkAgent } from '../../office/demo/script';
import { api } from '../api';
import type { HireManifest } from '../gen/HireManifest';
import { startHire } from '../state/hireWizard';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { threadAgentId, ui } from '../state/ui.svelte';

afterEach(() => {
  vi.restoreAllMocks();
  for (const t of toasts.list) toasts.dismiss(t.id);
  store.agents = [];
  ui.select(null);
  ui.openRequest('lead');
});

describe('the agent sheet follows the chat', () => {
  it('an open agent sheet moves to the agent whose 1:1 chat you open', () => {
    ui.selectAgent('ada');
    ui.openThread('agent:bo');
    expect(ui.thread).toBe('agent:bo');
    expect(ui.selectedAgentId).toBe('bo');
    // The chat of the agent the sheet already shows: nothing to move.
    ui.selectAgent('bo', 7);
    ui.openThread('agent:bo');
    expect(ui.selection).toEqual({ kind: 'agent', id: 'bo', eventId: 7 });
  });

  it('a closed sheet stays closed, and the sheet of a task or a file stays', () => {
    ui.openThread('agent:bo');
    expect(ui.selection).toBeNull();
    ui.selectTask('t1');
    ui.openThread('agent:ada');
    expect(ui.selection).toEqual({ kind: 'task', id: 't1' });
    ui.selectFile('src/a.ts');
    ui.openThread('agent:bo');
    expect(ui.selection).toEqual({ kind: 'file', id: 'src/a.ts' });
  });

  it('an open sheet on a Lead shows the Lead of the request you switch to, whatever took you there', () => {
    store.agents = [mkAgent('lead1', 'Mira', 'claude', 'lead', 6, 'idle', 'lead'), mkAgent('lead2', 'Noor', 'codex', 'lead', 1, 'idle', 'lead:x')];
    ui.selectAgent('lead1');
    ui.openRequest('lead:x'); // its tab, ⌘[ / ⌘]
    expect(ui.selectedAgentId).toBe('lead2');
    ui.openThread('lead'); // the palette's "Open its request"
    expect(ui.selectedAgentId).toBe('lead1');
    ui.openPlan(null, 'lead:x'); // a plan of the other request, in the Plan room
    expect(ui.selectedAgentId).toBe('lead2');
    // No move to another request: a Lead's sheet you opened by hand stays.
    ui.selectAgent('lead1');
    ui.openRequest('lead:x');
    expect(ui.selectedAgentId).toBe('lead1');
  });

  it('a request with no Lead yet closes a Lead’s sheet; the sheet of a worker, a task or a file follows no request', () => {
    store.agents = [mkAgent('lead1', 'Mira', 'claude', 'lead', 6, 'idle', 'lead'), mkAgent('lead2', 'Noor', 'codex', 'lead', 1, 'idle', 'lead:x'), mkAgent('ada', 'Ada', 'claude', 'worker', 0, 'idle')];
    ui.selectAgent('lead1');
    ui.newRequest(); // its Lead starts with your first message
    expect(ui.selection).toBeNull();
    // Closed stays closed.
    ui.openRequest('lead:x');
    expect(ui.selection).toBeNull();

    ui.selectAgent('ada');
    ui.openRequest('lead');
    expect(ui.selectedAgentId).toBe('ada');
    ui.selectTask('t1');
    ui.openRequest('lead:x');
    expect(ui.selection).toEqual({ kind: 'task', id: 't1' });
    ui.selectFile('src/a.ts');
    ui.openRequest('lead');
    expect(ui.selection).toEqual({ kind: 'file', id: 'src/a.ts' });
  });

  it('a request tab leaves the sheet of an agent that is no Lead where it is; the pill of its Lead moves an open sheet to the Lead', () => {
    ui.selectAgent('ada');
    ui.openThread('lead:x');
    expect([ui.thread, ui.selectedAgentId]).toEqual(['lead:x', 'ada']);
    ui.sheetFollows('lead1'); // what a click on the Lead's pill adds to opening its request
    expect(ui.selectedAgentId).toBe('lead1');
    ui.select(null);
    ui.sheetFollows('lead1');
    expect(ui.selection).toBeNull();
  });

  it('only a 1:1 thread names an agent', () => {
    expect(threadAgentId('agent:a-01')).toBe('a-01');
    expect(['lead', 'lead:x', 'closed:', 'closed:lead:x:17'].map(threadAgentId)).toEqual([null, null, null, null]);
  });

  it('Start agent opens the new agent’s chat and its sheet together', async () => {
    vi.spyOn(api, 'hireFromManifest').mockResolvedValue(mkAgent('cleo', 'Cleo', 'claude', 'solo', 2, 'idle'));
    // You talk to Bo with Ada's sheet open, then start Cleo.
    ui.openThread('agent:bo');
    ui.selectAgent('ada');
    expect(await startHire({ name: 'Cleo' } as HireManifest, false)).toBe(true);
    expect([ui.thread, ui.selectedAgentId]).toEqual(['agent:cleo', 'cleo']);
    expect(toasts.list.map((t) => t.title)).toEqual(['Cleo joined']);
  });
});
