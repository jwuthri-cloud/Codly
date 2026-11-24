// The agent sheet walks the event log only while the part that shows it is open, and again only
// when an event of its own agent came (agentLog.ts). Counted, not timed: how many times the work
// runs, with the store's own per-agent version as the key.
import { describe, expect, it } from 'vitest';
import { mkAgent } from '../../office/demo/script';
import type { EventKind } from '../../lib/gen/EventKind';
import type { EventRow } from '../../lib/gen/EventRow';
import type { Snapshot } from '../../lib/gen/Snapshot';
import { store } from '../../lib/state/store.svelte';
import { agentLogKey, logWork } from '../agentLog';

const sources = import.meta.glob<string>(['/src/sheet/AgentSheet.svelte', '/src/sheet/AgentMcp.svelte', '/src/sheet/AgentControls.svelte', '/src/sheet/AgentTabs.svelte', '/src/sheet/AgentTracesTab.svelte'], { query: '?raw', import: 'default', eager: true });
const source = (name: string) => sources[`/src/sheet/${name}`];

const agents = [mkAgent('a1', 'Ada', 'claude', 'worker', 0, 'working'), mkAgent('b1', 'Bo', 'codex', 'worker', 1, 'working')];
const apply = (evs: EventRow[]) => (store as unknown as { applyEvents(evs: EventRow[]): void }).applyEvents(evs);
const reset = () => {
  const snap = { as_of: 0, clis: [], accounts: [], workspace: null, settings: null, workspace_team: null, request_prefs: {}, agents, plans: [], tasks: [], approvals: [], chats: [], memories: [], triples: [], artifacts: [], events: [] };
  (store as unknown as { applySnapshot(s: Snapshot): void }).applySnapshot(snap as unknown as Snapshot);
};
let id = 0;
const ev = (agent: string, kind: unknown): EventRow => ({ id: ++id, ts: id * 10, agent_id: agent, task_id: null, run_id: null, kind: kind as EventKind });
const said = (agent: string) => ev(agent, { type: 'message', item_id: `m${id}`, text: 'hello', phase: 'final' });

describe('the log work of an agent’s sheet', () => {
  it('runs for an event of its own agent only, and only while it is open', () => {
    reset();
    let builds = 0;
    const work = logWork(() => ++builds);
    const read = (open: boolean) => work.read(open, agentLogKey('a1', store.revBy.get('a1')));
    expect(read(true)).toBe(1);
    // Another agent's events: nothing runs.
    apply([said('b1'), said('b1')]);
    expect(read(true)).toBe(1);
    expect(builds).toBe(1);
    // An event of this agent while it is closed (the Traces tab, History, …): nothing runs.
    apply([said('a1')]);
    expect(read(false)).toBeNull();
    expect(builds).toBe(1);
    // Opened again: once, for what came meanwhile; then not again until the next event of its own.
    expect(read(true)).toBe(2);
    expect(read(true)).toBe(2);
    apply([said('a1')]);
    expect(read(true)).toBe(3);
  });

  it('runs again when what else it reads changes (History’s filter, the live run)', () => {
    reset();
    let builds = 0;
    const work = logWork(() => ++builds);
    work.read(true, agentLogKey('a1', store.revBy.get('a1'), 'all'));
    work.read(true, agentLogKey('a1', store.revBy.get('a1'), 'all'));
    work.read(true, agentLogKey('a1', store.revBy.get('a1'), 'tools'));
    expect(builds).toBe(2);
  });

  it('is how the sheet walks the log: History, the MCP line and the compaction line go through it', () => {
    const sheet = source('AgentSheet.svelte');
    expect(sheet).toMatch(/logWork\(\(\) => historyFacts\(store\.events/);
    // History's rows are built inside its work, keyed on this agent's version.
    const rows = sheet.slice(sheet.indexOf('const foldedWork = logWork('), sheet.indexOf('const folded = $derived(foldedWork.read('));
    expect(rows).toContain('store.events');
    expect(rows).not.toContain('store.rev');
    expect(sheet).toMatch(/foldedWork\.read\(historyOpen, agentLogKey\(agent\.id, store\.revBy\.get\(agent\.id\)/);
    expect(source('AgentMcp.svelte')).toMatch(/logWork\(\(\) => startedWithoutFolderConfig\(store\.events/);
    expect(source('AgentControls.svelte')).toMatch(/logWork\(\(\) => compactionLine\(store\.events/);
  });

  it('builds the traces only while the Traces tab is open: the tab mounts alone, and reads its own agent’s version', () => {
    const tabs = source('AgentTabs.svelte');
    expect(tabs).toMatch(/\{:else if open === 'traces'\}\s*<AgentTracesTab \{agent\} \/>/);
    expect(tabs.match(/<AgentTracesTab/g)).toHaveLength(1);
    const traces = source('AgentTracesTab.svelte');
    expect(traces).toMatch(/void store\.revBy\.get\(agent\.id\);\s*return buildTraces\(store\.events/);
    expect(traces).not.toMatch(/store\.rev\b(?!By)/);
  });
});
